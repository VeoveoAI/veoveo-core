//! Independent OCSF 1.9.0 qualification using the upstream validator.
//! Sends only synthetic fixtures; no installation records or credentials are read.
#[path = "../../../testing/fixtures/audit_owner.rs"]
mod audit_owner;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use veoveo_audit::{export::ocsf::Event, *};

const UPSTREAM: &str = "https://schema.ocsf.io/1.9.0";

#[derive(Deserialize)]
struct Version {
    version: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Issue {
    error: String,
    message: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Warning {
    warning: String,
    message: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Validation {
    #[serde(default)]
    error: Option<String>,
    error_count: u64,
    errors: Vec<Issue>,
    warning_count: u64,
    warnings: Vec<Warning>,
}

#[derive(Serialize)]
struct CaseResult {
    case: &'static str,
    outcome: AuditOutcome,
    validation: Validation,
}

fn registry() -> AuditRegistry {
    let mut builder = AuditRegistry::builder();
    audit_owner::declare_audit_detail(&mut builder).unwrap();
    builder.build()
}

fn cases() -> Vec<(&'static str, AuditDetail, Option<AuditTarget>, bool)> {
    let registry = registry();
    let note = registry.detail(&audit_owner::Note { count: 1 }).unwrap();
    vec![
        (
            "anonymous",
            registry.detail(&platform::TaskUpdate {}).unwrap(),
            None,
            false,
        ),
        (
            "task",
            registry.detail(&platform::TaskCancel {}).unwrap(),
            Some(task_target(veoveo_types::TaskId::new())),
            true,
        ),
        (
            "owner-target",
            note,
            Some(audit_owner::item_target(audit_owner::ItemId::new())),
            true,
        ),
    ]
}

#[tokio::test]
#[ignore = "requires network access to the pinned upstream OCSF 1.9.0 validator; sends only synthetic fixtures"]
async fn upstream_accepts_each_export_class_and_outcome() {
    tokio::time::timeout(Duration::from_secs(180), async {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let version = client
            .get(format!("{UPSTREAM}/api/version"))
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<Version>()
            .await
            .unwrap();
        assert_eq!(version.version, "1.9.0");
        let mut results = Vec::new();
        for (case, detail, target, identified) in cases() {
            for outcome in [
                AuditOutcome::Success,
                AuditOutcome::Denied,
                AuditOutcome::Failure,
                AuditOutcome::Unknown,
            ] {
                let mut request = AuditRequest::background();
                if identified {
                    request.source_ip = Some("192.0.2.1".parse().unwrap());
                }
                let mut draft = AuditDraft::builder(request, detail.clone(), outcome);
                if let Some(target) = &target {
                    draft = draft.target(target.clone());
                }
                if outcome != AuditOutcome::Success {
                    draft = draft.reason(AuditReason::parse("fixture_reason").unwrap());
                }
                if identified {
                    draft = draft
                        .actor(AuditActor {
                            principal: "synthetic-user".parse().unwrap(),
                            tenant: Some("fixture".parse().unwrap()),
                            kind: AuditPrincipalKind::User,
                            work_context: Some("operations".parse().unwrap()),
                        })
                        .via(AuditVia {
                            server: "media".parse().unwrap(),
                            task: Some(veoveo_types::TaskId::new()),
                        });
                }
                let record = AuditRecord {
                    draft: draft.build().unwrap(),
                    recorded_at: Utc::now(),
                };
                let validation = client
                    .post(format!("{UPSTREAM}/api/v2/validate"))
                    .json(&Event::from_record(&record.stored()))
                    .send()
                    .await
                    .unwrap()
                    .error_for_status()
                    .unwrap()
                    .json::<Validation>()
                    .await
                    .unwrap();
                results.push(CaseResult {
                    case,
                    outcome,
                    validation,
                });
            }
        }
        println!("{}", serde_json::to_string(&results).unwrap());
        let failures: Vec<_> = results
            .iter()
            .filter(|result| {
                result.validation.error.is_some()
                    || result.validation.error_count != 0
                    || !result.validation.errors.is_empty()
            })
            .map(|result| (result.case, result.outcome, &result.validation))
            .collect();
        assert!(
            failures.is_empty(),
            "OCSF validation failures: {failures:#?}"
        );
    })
    .await
    .expect("OCSF qualification exceeded 180 seconds");
}
