//! Batched maintenance. A statement that deletes or clears expired rows changes at most
//! one batch of them, which it selects in expiry order and locks with `FOR UPDATE SKIP
//! LOCKED`. Each batch commits on its own, so no batch holds locks for long, and replicas
//! that prune at the same time take different rows. A pass repeats batches until one
//! changes fewer rows than the batch holds.
use std::future::Future;

use crate::StoreError;

/// How many rows one maintenance statement changes: 1 to 10,000.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct MaintenanceBatch(u16);

impl MaintenanceBatch {
    /// The batch the platform's own maintenance uses.
    pub const DEFAULT: Self = Self(1000);
    /// The largest batch.
    pub const MAX_ROWS: u16 = 10_000;

    pub fn new(rows: u16) -> Result<Self, StoreError> {
        if rows == 0 || rows > Self::MAX_ROWS {
            return Err(StoreError::InvalidBatch(rows));
        }
        Ok(Self(rows))
    }

    pub fn rows(self) -> u16 {
        self.0
    }

    /// The batch as a statement's `LIMIT`.
    pub fn limit(self) -> i64 {
        i64::from(self.0)
    }
}

impl Default for MaintenanceBatch {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// What one maintenance pass changed: the rows and the batches it took.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Pruned {
    pub rows: u64,
    pub batches: u64,
}

impl Pruned {
    /// Run `batch_statement` with the batch's limit until it changes fewer rows than
    /// the batch holds. Each call is its own short statement.
    pub async fn in_batches<F, Fut, E>(
        batch: MaintenanceBatch,
        mut batch_statement: F,
    ) -> Result<Self, E>
    where
        F: FnMut(i64) -> Fut,
        Fut: Future<Output = Result<u64, E>>,
    {
        let mut pruned = Self::default();
        loop {
            let rows = batch_statement(batch.limit()).await?;
            pruned.rows += rows;
            pruned.batches += 1;
            if rows < u64::from(batch.rows()) {
                return Ok(pruned);
            }
        }
    }
}

impl std::ops::Add for Pruned {
    type Output = Self;

    fn add(self, other: Self) -> Self {
        Self {
            rows: self.rows + other.rows,
            batches: self.batches + other.batches,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batches_hold_one_to_ten_thousand_rows() {
        assert_eq!(MaintenanceBatch::new(1).unwrap().limit(), 1);
        assert_eq!(MaintenanceBatch::new(10_000).unwrap().rows(), 10_000);
        assert!(matches!(
            MaintenanceBatch::new(0),
            Err(StoreError::InvalidBatch(0))
        ));
        assert!(MaintenanceBatch::new(10_001).is_err());
        assert_eq!(MaintenanceBatch::default(), MaintenanceBatch::DEFAULT);
    }

    #[tokio::test]
    async fn a_pass_repeats_batches_until_one_falls_short() {
        let batch = MaintenanceBatch::new(2).unwrap();
        let mut remaining = 5u64;
        let pruned = Pruned::in_batches(batch, |limit| {
            let taken = remaining.min(u64::try_from(limit).unwrap());
            remaining -= taken;
            async move { Ok::<_, StoreError>(taken) }
        })
        .await
        .unwrap();
        assert_eq!(
            pruned,
            Pruned {
                rows: 5,
                batches: 3
            }
        );
        // A pass over exactly one full batch confirms with an empty one.
        let mut remaining = 2u64;
        let pruned = Pruned::in_batches(batch, |limit| {
            let taken = remaining.min(u64::try_from(limit).unwrap());
            remaining -= taken;
            async move { Ok::<_, StoreError>(taken) }
        })
        .await
        .unwrap();
        assert_eq!(
            pruned,
            Pruned {
                rows: 2,
                batches: 2
            }
        );
    }
}
