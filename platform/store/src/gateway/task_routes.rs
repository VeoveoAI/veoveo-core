//! Cleanup of the routes the Gateway keeps for the upstream Tasks it projected. The
//! Gateway creates and reads routes with its own statements.
use chrono::{DateTime, Utc};

use crate::{MaintenanceBatch, PlatformStore, Pruned, StoreError};

impl PlatformStore {
    /// Delete the Task routes that expired at or before `now`, in batches. The Gateway
    /// schedules it; replicas that run it at once take different routes.
    pub async fn prune_task_routes(
        &self,
        now: DateTime<Utc>,
        batch: MaintenanceBatch,
    ) -> Result<Pruned, StoreError> {
        Pruned::in_batches(batch, |limit| async move {
            Ok(
                sqlx::query_file!("src/gateway/task_routes/prune.sql", now, limit)
                    .execute(self.pool())
                    .await?
                    .rows_affected(),
            )
        })
        .await
    }
}
