//! Task lifecycle and recovery vocabularies, independent of storage and execution.
//! The `postgres` feature binds both vocabularies to PostgreSQL `text` columns.

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, veoveo_types::Vocabulary)]
#[cfg_attr(feature = "postgres", vocabulary(postgres))]
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

/// How a Task proceeds after the process that owned it ends.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, veoveo_types::Vocabulary)]
#[cfg_attr(feature = "postgres", vocabulary(postgres))]
pub enum RecoveryClass {
    /// Another worker claims the Task and runs it again.
    #[vocabulary(rename = "resume")]
    Resume,
    /// The Task may have had an effect that cannot be repeated; it fails as interrupted.
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
    fn recovery_wire_order_and_unknown_rejection() {
        let expected = ["resume", "interrupted_indeterminate"];
        assert_eq!(RecoveryClass::ALL.len(), expected.len());
        for (class, spelling) in RecoveryClass::ALL.iter().zip(expected) {
            assert_eq!(serde_json::to_value(class).unwrap(), spelling);
            assert_eq!(
                serde_json::from_value::<RecoveryClass>(serde_json::json!(spelling)).unwrap(),
                *class
            );
        }
        for undeclared in ["\"unknown\"", "\"provider_wait\"", "\"webhook_wait\""] {
            assert!(serde_json::from_str::<RecoveryClass>(undeclared).is_err());
        }
    }

    #[cfg(feature = "postgres")]
    #[test]
    fn postgres_binding_is_text() {
        use sqlx::{Type, postgres::PgHasArrayType};
        assert_eq!(
            <TaskStatus as Type<sqlx::Postgres>>::type_info(),
            <String as Type<sqlx::Postgres>>::type_info()
        );
        assert_eq!(
            <RecoveryClass as PgHasArrayType>::array_type_info(),
            <String as PgHasArrayType>::array_type_info()
        );
    }
}
