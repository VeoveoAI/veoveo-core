//! Bounded retry for transactions that SurrealDB rejects with a retryable write conflict.
//!
//! SurrealDB commits optimistically and reports a conflicting concurrent write as a
//! retryable transaction conflict. The whole transaction rolls back, so running it again
//! re-evaluates every fence and precondition against the current state. Thrown domain
//! errors are not conflicts and never retry.

use std::{future::Future, time::Duration};

use crate::StoreError;

pub(crate) const MAX_TRANSACTION_ATTEMPTS: u32 = 8;

pub(crate) fn is_retryable_transaction_failure(error: &surrealdb::Error) -> bool {
    matches!(
        error.query_details(),
        Some(surrealdb::types::QueryError::TransactionConflict)
    ) || error.message().starts_with("Transaction conflict:")
        || error
            .message()
            .contains("not executed due to a failed transaction")
}

pub(crate) async fn transaction_retry_backoff(attempt: u32) {
    tokio::time::sleep(Duration::from_millis(1_u64 << attempt)).await;
}

/// Runs `transaction` again while the database reports a retryable conflict, up to
/// [`MAX_TRANSACTION_ATTEMPTS`] attempts in total.
pub(crate) async fn retry_conflicts<T, F, Fut>(mut transaction: F) -> Result<T, StoreError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, StoreError>>,
{
    for attempt in 0..MAX_TRANSACTION_ATTEMPTS {
        match transaction().await {
            Err(StoreError::Database(error))
                if is_retryable_transaction_failure(&error)
                    && attempt + 1 < MAX_TRANSACTION_ATTEMPTS =>
            {
                transaction_retry_backoff(attempt).await;
            }
            result => return result,
        }
    }
    unreachable!("the bounded retry loop always returns on its final attempt")
}
