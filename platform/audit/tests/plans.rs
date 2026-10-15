//! Plans of the Console and CLI reads with sequential scans disabled. A page seeks its
//! cursor in a generic plan, and an actor or trace filter seeks its own index in the
//! custom plan a page runs with.
#[allow(dead_code, reason = "each test binary uses part of the shared fixture")]
#[path = "../../../testing/fixtures/src/store.rs"]
mod store;

use serde_json::Value;
use sqlx::Row;
use store::TestDb;

/// One index a plan reads and its index condition.
type IndexScan = (String, String);

fn index_scans(node: &Value, scans: &mut Vec<IndexScan>) {
    if let Some(index) = node.get("Index Name").and_then(Value::as_str) {
        let condition = node.get("Index Cond").and_then(Value::as_str);
        scans.push((index.to_owned(), condition.unwrap_or_default().to_owned()));
    }
    for child in node
        .get("Plans")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        index_scans(child, scans);
    }
}

/// The index scans of each plan a script explains, in order.
async fn explained(db: &TestDb, script: &'static str) -> Vec<Vec<IndexScan>> {
    let mut connection = db.a.pool().acquire().await.unwrap();
    sqlx::raw_sql(script)
        .fetch_all(&mut *connection)
        .await
        .unwrap()
        .into_iter()
        .map(|row| {
            let plan: Value = row.try_get(0).unwrap();
            let mut scans = Vec::new();
            index_scans(&plan[0]["Plan"], &mut scans);
            scans
        })
        .collect()
}

fn reads(plan: &[IndexScan], index: &str, condition: &str) -> bool {
    plan.iter()
        .any(|(name, cond)| name == index && cond.contains(condition))
}

#[tokio::test]
async fn pages_seek_their_cursor_and_each_filter() {
    let db = TestDb::new().await;
    for (script, statement, cursor) in [
        (
            include_str!("queries/plans/page_oldest_first.sql"),
            include_str!("../src/store/records/page_oldest_first.sql"),
            "(id > COALESCE(",
        ),
        (
            include_str!("queries/plans/page_newest_first.sql"),
            include_str!("../src/store/records/page_newest_first.sql"),
            "(id < COALESCE(",
        ),
    ] {
        assert!(
            script.contains(statement.trim_end()),
            "the plan script prepares the page statement verbatim"
        );
        let plans = explained(&db, script).await;
        assert_eq!(plans.len(), 3);
        assert!(reads(&plans[0], "record_partition", cursor), "{plans:?}");
        assert!(reads(&plans[1], "record_actor", "actor ="), "{plans:?}");
        assert!(reads(&plans[2], "record_trace", "trace_id ="), "{plans:?}");
    }
}

#[tokio::test]
async fn daily_pages_seek_the_cursor_day() {
    let db = TestDb::new().await;
    let script = include_str!("queries/plans/daily.sql");
    assert!(
        script.contains(include_str!("../src/store/records/daily.sql").trim_end()),
        "the plan script prepares the daily statement verbatim"
    );
    let plans = explained(&db, script).await;
    assert_eq!(plans.len(), 1);
    assert!(
        reads(&plans[0], "record_daily", "(day <= COALESCE("),
        "{plans:?}"
    );
}
