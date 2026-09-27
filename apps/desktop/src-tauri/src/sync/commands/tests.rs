//! `acknowledge_all_conflicts` (DESIGN §11): the "Marcar vistos" action must
//! clear every unacknowledged conflict, not just the page the UI happened to
//! have loaded (bug: only the newest 50 were acknowledged, leaving older
//! entries stuck unacknowledged until the app restarted).

use super::*;
use crate::sync::test_support::new_synced_test_db;

fn insert_conflict(conn: &Connection, id: &str, acknowledged: i64) {
    conn.execute(
        "INSERT INTO sync_conflicts(id, table_name, row_id, reason, created_at, acknowledged)
         VALUES (?1, 'items', ?1, 'lww_lost', 1, ?2)",
        rusqlite::params![id, acknowledged],
    )
    .expect("insert conflict");
}

fn count_unacknowledged(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT COUNT(*) FROM sync_conflicts WHERE acknowledged = 0",
        [],
        |r| r.get(0),
    )
    .expect("count unacknowledged")
}

#[test]
fn acknowledge_all_conflicts_clears_more_than_a_single_page() {
    let conn = new_synced_test_db();
    // More than the 50-row page the card's list view loads.
    for i in 0..87 {
        insert_conflict(&conn, &format!("cf{i}"), 0);
    }
    // A couple already acknowledged: must stay untouched (and not be recounted).
    insert_conflict(&conn, "already-1", 1);
    insert_conflict(&conn, "already-2", 1);

    let changed = acknowledge_all_conflicts(&conn).expect("acknowledge all");

    assert_eq!(changed, 87, "every unacknowledged conflict was updated");
    assert_eq!(count_unacknowledged(&conn), 0, "none remain unacknowledged");
}

#[test]
fn acknowledge_all_conflicts_is_a_noop_with_nothing_pending() {
    let conn = new_synced_test_db();
    insert_conflict(&conn, "already-1", 1);

    let changed = acknowledge_all_conflicts(&conn).expect("acknowledge all");

    assert_eq!(changed, 0);
}
