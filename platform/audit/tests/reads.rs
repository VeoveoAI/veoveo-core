//! SQL filtering, view admission, change streams and recovery for the Console and CLI.
mod support;

use chrono::{DateTime, TimeDelta, Utc};
use futures::StreamExt;
use std::{collections::BTreeSet, num::NonZeroU32, sync::Arc, time::Duration};
use support::{AuditDb, user, within};
use veoveo_audit::{integrity::AuditSigningKey, *};

fn draft(
    actor: &AuditActor,
    request: &AuditRequest,
    time: DateTime<Utc>,
    outcome: AuditOutcome,
) -> AuditDraft {
    AuditDraft::builder(
        request.clone(),
        AuditTarget::Installation,
        AuditDetail::Read {
            method: AuditReadMethod::ResourceRead,
        },
        outcome,
        if outcome == AuditOutcome::Allowed {
            AuditReason::Accepted
        } else {
            AuditReason::PolicyDenied
        },
    )
    .actor(actor.clone())
    .occurred_at(time)
    .build()
    .unwrap()
}

async fn next(stream: &mut AuditLiveStream) -> AuditLiveChange {
    within(Duration::from_secs(10), stream.next())
        .await
        .expect("the change stream continues")
        .expect("the change stream delivers a change")
}

#[tokio::test]
async fn sql_filters_before_decoding_and_limits_and_binds_view_receipts_to_current_identity() {
    within(Duration::from_secs(90), async {
        let audit = AuditDb::new().await;
        let alice = user("alpha", "alice");
        let partition = AuditPartition::Tenant("alpha".parse().unwrap());
        let scope = AuditReadScope::new(Some("alpha".parse().unwrap()), true);
        let request = AuditRequest::background();
        let now = Utc::now();
        let first = draft(&alice, &request, now, AuditOutcome::Allowed);
        let denied = draft(&alice, &request, now, AuditOutcome::Denied);
        let last = draft(&alice, &request, now, AuditOutcome::Allowed);
        audit
            .a
            .append(&[first.clone(), denied, last.clone()])
            .await
            .unwrap();
        // A row of another class cannot decode as a draft. Successful filtered reads
        // therefore show the class filter ran in SQL.
        sqlx::query_file!(
            "tests/queries/reads/insert_undecodable.sql",
            first.id() as AuditRecordId,
            AuditRecordId::new() as AuditRecordId,
        )
        .execute(audit.db.a.pool())
        .await
        .unwrap();
        let mut query = AuditQuery::new(partition.clone());
        query.limit = 1;
        query.class = Some(AuditClass::ApiActivity);
        query.outcome = Some(AuditOutcome::Allowed);
        query.actor = Some(alice.principal.clone());
        query.target = Some(AuditTarget::Installation);
        query.trace = Some(request.trace_id.clone());
        query.from = Some(now - TimeDelta::seconds(1));
        query.until = Some(now + TimeDelta::seconds(1));
        for (order, expected) in [
            (AuditOrder::OldestFirst, [first.id(), last.id()]),
            (AuditOrder::NewestFirst, [last.id(), first.id()]),
        ] {
            query.order = order;
            query.cursor = None;
            let page = audit.b.page(&scope, &query).await.unwrap();
            assert_eq!(page.records.len(), 1);
            assert_eq!(page.records[0].draft.id(), expected[0]);
            query.cursor = Some(page.next.expect("a matching second row supplies a cursor"));
            let page = audit.b.page(&scope, &query).await.unwrap();
            assert_eq!(page.records.len(), 1);
            assert_eq!(page.records[0].draft.id(), expected[1]);
            assert!(page.next.is_none());
        }
        query.class = None;
        query.cursor = None;
        query.order = AuditOrder::NewestFirst;
        assert!(
            matches!(
                audit.b.page(&scope, &query).await,
                Err(AuditStoreError::Integrity)
            ),
            "the undecodable row fails a read that selects it"
        );
        let foreign = AuditReadScope::new(Some("bravo".parse().unwrap()), false);
        assert!(matches!(
            audit.b.page(&foreign, &query).await,
            Err(AuditStoreError::AccessDenied)
        ));

        let profile: veoveo_types::GatewayProfileId = "operator".parse().unwrap();
        for (age, admitted) in [(0, true), (30, false)] {
            let receipt = AuditDraft::builder(
                AuditRequest::background(),
                AuditTarget::AuditLog {
                    partition: partition.clone(),
                },
                AuditDetail::Read {
                    method: AuditReadMethod::AuditView,
                },
                AuditOutcome::Allowed,
                AuditReason::Accepted,
            )
            .actor(alice.clone())
            .authority(AuditAuthority {
                profile: Some(profile.clone()),
                ..Default::default()
            })
            .occurred_at(now - TimeDelta::minutes(age))
            .build()
            .unwrap();
            audit
                .a
                .append(std::slice::from_ref(&receipt))
                .await
                .unwrap();
            let view = |partition: AuditPartition,
                        actor: veoveo_types::PrincipalId,
                        profile: veoveo_types::GatewayProfileId| {
                let audit = audit.b.clone();
                let scope = scope.clone();
                let actor_partition = AuditPartition::Tenant("alpha".parse().unwrap());
                let receipt = receipt.id();
                async move {
                    audit
                        .view_admitted(
                            &scope,
                            &partition,
                            &actor_partition,
                            &actor,
                            &profile,
                            receipt,
                        )
                        .await
                        .unwrap()
                }
            };
            assert_eq!(
                view(partition.clone(), alice.principal.clone(), profile.clone()).await,
                admitted
            );
            assert!(!view(partition.clone(), "bob".parse().unwrap(), profile.clone()).await);
            assert!(
                !view(
                    partition.clone(),
                    alice.principal.clone(),
                    "other".parse().unwrap()
                )
                .await
            );
            assert!(
                !view(
                    AuditPartition::Installation,
                    alice.principal.clone(),
                    profile.clone()
                )
                .await
            );
        }
    })
    .await;
}

#[tokio::test]
async fn scoped_streams_daily_pages_and_sealed_recovery_preserve_partition_and_date_bounds() {
    within(Duration::from_secs(90), async {
        let audit = AuditDb::new().await;
        let alice = user("alpha", "alice");
        let partition = AuditPartition::Tenant("alpha".parse().unwrap());
        let scope = AuditReadScope::new(Some("alpha".parse().unwrap()), false);
        let today = Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();
        let now = Utc::now();
        let request = AuditRequest::background();
        let expected = [
            draft(&alice, &request, now, AuditOutcome::Allowed),
            draft(&alice, &request, now, AuditOutcome::Denied),
            draft(
                &alice,
                &request,
                today - TimeDelta::hours(1),
                AuditOutcome::Allowed,
            ),
            draft(&alice, &request, now, AuditOutcome::Allowed),
        ];
        let mut live = audit.b.live(&scope, &partition).await.unwrap();
        let mut sealed = audit.b.blocks_live(&scope, &partition).await.unwrap();
        let service = AuditService::start(
            audit.a.clone(),
            Arc::new(AuditSigningKey::from_seed(&[41; 32])),
            NonZeroU32::new(7).unwrap(),
            Default::default(),
        )
        .unwrap();
        let foreign = draft(&user("bravo", "bob"), &request, now, AuditOutcome::Allowed);
        audit.a.append(&[foreign]).await.unwrap();
        audit.a.append(&expected).await.unwrap();
        let mut received = BTreeSet::new();
        while received.len() < expected.len() {
            match next(&mut live).await {
                AuditLiveChange::Record(record) => {
                    assert_eq!(record.draft.partition(), &partition);
                    received.insert(record.draft.id());
                }
                AuditLiveChange::Reconcile => {}
                AuditLiveChange::RetainedRangeChanged => panic!("nothing was retired"),
            }
        }
        assert_eq!(received, expected.iter().map(AuditDraft::id).collect());
        within(Duration::from_secs(20), sealed.next())
            .await
            .unwrap()
            .unwrap();
        let blocks = audit.b.blocks(&scope, &partition, None, 10).await.unwrap();
        assert_eq!(blocks.len(), 1);
        let range = audit.b.export_range(&scope, &partition).await.unwrap();
        assert_eq!(range.first, Some(blocks[0].head.sequence));
        assert_eq!(range.checkpoint, Some(blocks[0].checkpoint()));
        let mut filter = AuditQuery::new(partition.clone());
        filter.outcome = Some(AuditOutcome::Denied);
        let records = audit
            .b
            .filtered_block_records(&scope, &blocks[0], &filter)
            .await
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].draft.id(), expected[1].id());
        // A block holds no pages, so a block read refuses a cursor.
        let mut paged = filter.clone();
        paged.cursor = Some(AuditCursor {
            order: paged.order,
            partition: partition.clone(),
            last_id: expected[0].id(),
        });
        assert!(matches!(
            audit
                .b
                .filtered_block_records(&scope, &blocks[0], &paged)
                .await,
            Err(AuditStoreError::Validation(AuditValidationError::Cursor))
        ));

        // Daily counts cover at most 366 days.
        let year = AuditDailyQuery {
            partition: partition.clone(),
            from: today - TimeDelta::days(365),
            until: today + TimeDelta::days(1),
            cursor: None,
            limit: 10,
        };
        audit.b.daily(&scope, &year).await.unwrap();
        let longer = AuditDailyQuery {
            from: year.from - TimeDelta::days(1),
            ..year
        };
        assert!(matches!(
            audit.b.daily(&scope, &longer).await,
            Err(AuditStoreError::Validation(AuditValidationError::DailySpan))
        ));

        let mut daily = AuditDailyQuery {
            partition: partition.clone(),
            from: today - TimeDelta::days(1),
            until: today + TimeDelta::days(1),
            cursor: None,
            limit: 1,
        };
        let mut counts = Vec::new();
        loop {
            let page = audit.b.daily(&scope, &daily).await.unwrap();
            assert!(page.counts.iter().all(|count| count.partition == partition));
            counts.extend(
                page.counts
                    .into_iter()
                    .map(|count| (count.day, count.outcome, count.count)),
            );
            let Some(next) = page.next else {
                break;
            };
            daily.cursor = Some(next);
        }
        assert_eq!(
            counts,
            vec![
                (today, AuditOutcome::Allowed, 2),
                (today, AuditOutcome::Denied, 1),
                (today - TimeDelta::days(1), AuditOutcome::Allowed, 1)
            ]
        );
        daily.from = today;
        daily.cursor = None;
        daily.limit = 100;
        assert_eq!(
            audit
                .b
                .daily(&scope, &daily)
                .await
                .unwrap()
                .counts
                .iter()
                .map(|count| count.count)
                .sum::<u64>(),
            3
        );

        drop(live);
        drop(sealed);
        let missed = draft(&alice, &request, now, AuditOutcome::Allowed);
        audit.a.append(std::slice::from_ref(&missed)).await.unwrap();
        within(
            Duration::from_secs(15),
            audit.b.wait_sealed(&scope, &partition, missed.id()),
        )
        .await
        .unwrap();
        // A resumed reader subscribes first, then replays the partition's blocks.
        let mut resumed = audit.b.live(&scope, &partition).await.unwrap();
        let recovered = audit
            .b
            .blocks(&scope, &partition, Some(blocks[0].head.sequence), 10)
            .await
            .unwrap();
        assert_eq!(recovered.len(), 1);
        let recovered_records = audit.b.block_records(&scope, &recovered[0]).await.unwrap();
        assert_eq!(recovered_records.len(), 1);
        assert_eq!(recovered_records[0].draft.id(), missed.id());
        service.shutdown(Duration::from_secs(15)).await.unwrap();

        sqlx::query_file!(
            "tests/queries/reads/remove_block.sql",
            expected[0].id() as AuditRecordId,
            partition.storage_key(),
            i64::try_from(blocks[0].head.sequence.get()).unwrap(),
        )
        .execute(audit.db.a.pool())
        .await
        .unwrap();
        assert!(matches!(
            next(&mut resumed).await,
            AuditLiveChange::RetainedRangeChanged
        ));
        assert!(
            audit
                .b
                .filtered_block_records(&scope, &blocks[0], &filter)
                .await
                .is_err(),
            "a missing member fails the read even when the filter would exclude it"
        );
    })
    .await;
}
