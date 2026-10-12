//! Task lifecycle and recovery vocabularies, independent of storage and execution.

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, veoveo_types::Vocabulary)]
pub enum TaskStatus {
    #[vocabulary(rename = "queued")]
    Queued,
    #[vocabulary(rename = "running")]
    Running,
    #[vocabulary(rename = "waiting")]
    Waiting,
    #[vocabulary(rename = "succeeded")]
    Succeeded,
    #[vocabulary(rename = "failed")]
    Failed,
    #[vocabulary(rename = "cancel_requested")]
    CancelRequested,
    #[vocabulary(rename = "cancelled")]
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, veoveo_types::Vocabulary)]
pub enum RecoveryClass {
    #[vocabulary(rename = "resume")]
    Resume,
    #[vocabulary(rename = "webhook_wait")]
    WebhookWait,
    #[vocabulary(rename = "provider_wait")]
    ProviderWait,
    #[vocabulary(rename = "interrupted_indeterminate")]
    InterruptedIndeterminate,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_status_wire_order_and_unknown_rejection() {
        let expected = [
            "queued",
            "running",
            "waiting",
            "succeeded",
            "failed",
            "cancel_requested",
            "cancelled",
        ];
        assert_eq!(TaskStatus::ALL.len(), expected.len());
        for (state, spelling) in TaskStatus::ALL.iter().zip(expected) {
            assert_eq!(serde_json::to_value(state).unwrap(), spelling);
            assert_eq!(
                serde_json::from_value::<TaskStatus>(serde_json::json!(spelling)).unwrap(),
                *state
            );
        }
        assert!(serde_json::from_str::<TaskStatus>("\"unknown\"").is_err());
    }

    #[test]
    fn recovery_wire_order_includes_provider_wait() {
        let expected = [
            "resume",
            "webhook_wait",
            "provider_wait",
            "interrupted_indeterminate",
        ];
        assert_eq!(RecoveryClass::ALL.len(), expected.len());
        for (class, spelling) in RecoveryClass::ALL.iter().zip(expected) {
            assert_eq!(serde_json::to_value(class).unwrap(), spelling);
            assert_eq!(
                serde_json::from_value::<RecoveryClass>(serde_json::json!(spelling)).unwrap(),
                *class
            );
        }
        assert!(serde_json::from_str::<RecoveryClass>("\"unknown\"").is_err());
    }
}
