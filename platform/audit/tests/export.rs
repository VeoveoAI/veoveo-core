//! Export intents, receipts and refusals under the sealer role, and provider delivery.
mod support;

use std::{num::NonZeroU32, sync::Arc, time::Duration};
use support::{AuditDb, draft, seal_all, within};
use veoveo_audit::{integrity::AuditSigningKey, sealer::AuditSealer, *};
use veoveo_types::Sha256Digest;

fn read(method: AuditReadMethod) -> AuditDraft {
    draft(AuditDetail::Read { method })
}

#[derive(Debug, PartialEq, Eq)]
struct Counts {
    records: i64,
    blocks: i64,
    deliveries: i64,
    cursors: i64,
    anchors: i64,
}

async fn counts(audit: &AuditDb) -> Counts {
    let row = sqlx::query_file!("tests/queries/export/counts.sql")
        .fetch_one(audit.db.b.pool())
        .await
        .unwrap();
    Counts {
        records: row.records,
        blocks: row.blocks,
        deliveries: row.deliveries,
        cursors: row.cursors,
        anchors: row.anchors,
    }
}

#[tokio::test]
async fn intents_receipts_and_refusals_fence_retention_across_replicas() {
    within(Duration::from_secs(90), async {
        let audit = AuditDb::new().await;
        let scope = AuditReadScope::new(None, true);
        let partition = AuditPartition::Installation;
        let key = Arc::new(AuditSigningKey::from_seed(&[19; 32]));
        let sealer = AuditSealer::acquire(audit.a.clone(), key.clone())
            .await
            .unwrap();
        audit
            .a
            .append(&[read(AuditReadMethod::Status)])
            .await
            .unwrap();
        assert_eq!(seal_all(&sealer).await, 1);
        let block = audit
            .b
            .blocks(&scope, &partition, None, 1)
            .await
            .unwrap()
            .remove(0);
        let first = AuditDestinationId::from_configuration_hash(Sha256Digest::from_bytes([1; 32]));
        let second = AuditDestinationId::from_configuration_hash(Sha256Digest::from_bytes([2; 32]));
        let destinations = vec![first.clone(), second.clone()];
        let payload = AuditExportPayload {
            content: Sha256Digest::from_bytes([3; 32]),
            seal: Sha256Digest::from_bytes([4; 32]),
        };
        let cutoff = chrono::Utc::now() + chrono::TimeDelta::days(1);
        assert!(
            matches!(
                sealer.complete_export(&first, &block, &payload).await,
                Err(AuditStoreError::Integrity)
            ),
            "a receipt requires a committed intent"
        );
        for _ in 0..4 {
            sealer
                .prepare_export(&first, &block, &payload)
                .await
                .unwrap();
        }
        assert!(!sealer.retire(&block, cutoff, &destinations).await.unwrap());
        let different = AuditExportPayload {
            content: Sha256Digest::from_bytes([5; 32]),
            ..payload.clone()
        };
        assert!(matches!(
            sealer.prepare_export(&first, &block, &different).await,
            Err(AuditStoreError::Integrity)
        ));
        for _ in 0..4 {
            sealer
                .complete_export(&first, &block, &payload)
                .await
                .unwrap();
        }
        assert!(audit.b.export_candidate(&first).await.unwrap().is_none());
        assert_eq!(
            audit.b.export_candidate(&second).await.unwrap(),
            Some(block.clone())
        );
        assert!(
            !sealer.retire(&block, cutoff, &destinations).await.unwrap(),
            "every configured destination must acknowledge"
        );
        sealer
            .prepare_export(&second, &block, &payload)
            .await
            .unwrap();
        sealer
            .reject_export(
                &second,
                &block,
                &payload,
                AuditExportRejection::PartialAcceptance,
            )
            .await
            .unwrap();
        assert!(matches!(
            sealer.prepare_export(&second, &block, &payload).await,
            Err(AuditStoreError::ExportRejected)
        ));

        let standby = tokio::spawn({
            let audit = audit.b.clone();
            let key = key.clone();
            async move { AuditSealer::acquire(audit, key).await }
        });
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(!standby.is_finished(), "one replica holds the sealer role");
        sealer.release().await.unwrap();
        let successor = within(Duration::from_secs(10), standby)
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(
                successor.prepare_export(&second, &block, &payload).await,
                Err(AuditStoreError::ExportRejected)
            ),
            "another replica cannot retry a recorded refusal"
        );
        assert_eq!(
            counts(&audit).await,
            Counts {
                records: 1,
                blocks: 1,
                deliveries: 2,
                cursors: 1,
                anchors: 0
            }
        );
        // The operator keeps only the destination that delivered. The retired block
        // becomes the anchor, and the delivery cursor outlives it.
        assert!(
            successor
                .retire(&block, cutoff, std::slice::from_ref(&first))
                .await
                .unwrap()
        );
        assert_eq!(
            counts(&audit).await,
            Counts {
                records: 0,
                blocks: 0,
                deliveries: 0,
                cursors: 1,
                anchors: 1
            }
        );
        let range = audit.b.export_range(&scope, &partition).await.unwrap();
        assert_eq!(range.first, None);
        assert_eq!(range.checkpoint, Some(block.checkpoint()));
        assert!(audit.b.export_candidate(&second).await.unwrap().is_none());
        successor.release().await.unwrap();
    })
    .await;
}

#[tokio::test]
async fn busy_sealing_and_export_acknowledge_each_record_once() {
    within(Duration::from_secs(90), async {
        use axum::{Json, Router, extract::State, routing::post};
        use std::sync::atomic::{AtomicUsize, Ordering};
        use veoveo_audit::export::{AuditExportConfig, AuditExporter, OtlpConfig};
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Request {
            resource_logs: Vec<Resource>,
        }
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Resource {
            scope_logs: Vec<Scope>,
        }
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Scope {
            log_records: Vec<serde::de::IgnoredAny>,
        }
        struct Server(tokio::task::JoinHandle<()>);
        impl Drop for Server {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let accepted = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1/logs", listener.local_addr().unwrap())
            .parse()
            .unwrap();
        let app = Router::new()
            .route(
                "/v1/logs",
                post(
                    |State(accepted): State<Arc<AtomicUsize>>, Json(request): Json<Request>| async move {
                        let count = request
                            .resource_logs
                            .into_iter()
                            .flat_map(|resource| resource.scope_logs)
                            .map(|scope| scope.log_records.len())
                            .sum::<usize>();
                        accepted.fetch_add(count, Ordering::SeqCst);
                        ([("content-type", "application/json")], "{}")
                    },
                ),
            )
            .with_state(accepted.clone());
        let _server = Server(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        }));
        let exports = AuditExportConfig {
            s3: None,
            otlp: Some(OtlpConfig {
                endpoint,
                allow_http: true,
                bearer_token_env: None,
            }),
        };
        let destination = AuditExporter::new(exports.clone())
            .unwrap()
            .destination_ids()
            .remove(0);
        let audit = AuditDb::new().await;
        let service = AuditService::start(
            audit.a.clone(),
            Arc::new(AuditSigningKey::from_seed(&[47; 32])),
            NonZeroU32::new(1).unwrap(),
            exports,
        )
        .unwrap();
        // Separate commits overlap sealing passes and collector acknowledgements.
        let mut records = Vec::new();
        for index in 0..48 {
            let record = read(AuditReadMethod::Status);
            audit.b.append(std::slice::from_ref(&record)).await.unwrap();
            records.push(record.id());
            if index % 8 == 0 {
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
        }
        let caught_up = tokio::time::timeout(Duration::from_secs(30), async {
            audit
                .b
                .wait_sealed(
                    &AuditReadScope::new(None, true),
                    &AuditPartition::Installation,
                    *records.last().unwrap(),
                )
                .await
                .unwrap();
            while audit.b.export_candidate(&destination).await.unwrap().is_some() {
                assert_ne!(service.health().state(), AuditHealthState::Failed);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        service.shutdown(Duration::from_secs(15)).await.unwrap();
        caught_up.expect("sealing and export did not catch up within 30 seconds");
        assert_eq!(
            accepted.load(Ordering::SeqCst),
            records.len(),
            "each record reaches the collector once"
        );
    })
    .await;
}
