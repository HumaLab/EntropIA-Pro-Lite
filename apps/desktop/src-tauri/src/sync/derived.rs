//! Derived cache of paid Bibliography results (`bibliography-derived-v1`).
//!
//! Client side of `odd/plans/plan-sync-bibliografia-derivada.md` §2.2 and §2.9:
//! the local tables where a device records the results it already paid for
//! (embeddings, OCR answers) so another device of the same account never pays
//! for them twice. This module owns the account semantics only — the tables are
//! created by [`crate::sync::schema::ensure_sync_schema`], and reads/writes/transport
//! arrive in later slices.
//!
//! Account rules (§2.9), all of them durable across logout:
//!
//! - Every row carries the `account_id` that originated it. Rows written with no
//!   session take [`BOUND_ACCOUNT_KEY`] (the last account bound to this archive,
//!   whose content they are); only an archive that was never bound writes `''`.
//! - [`OWNER_ACCOUNT_KEY`] is the *first* account ever bound to the archive. It
//!   is written once, never overwritten, and is the only account the backfill
//!   ever runs for.
//! - Rows and both marks survive logout: [`crate::sync::session::clear_sync_state`]
//!   only resets `pushed` and drops the channel's account metadata. Deleting the
//!   rows would destroy paid work; the rows are never readable by another
//!   account either way.
//! - Reading and pushing use [`effective_account`]: the session account, or (no
//!   session) the bound account. After an account change the previous account's
//!   rows — including the ones written offline after its logout — are inert.

use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

use super::session::{meta_get, meta_set};

/// Durable mark of the account bound to this archive (§2.9). Written in the same
/// transaction as the session. Deliberately NOT in `SESSION_META_KEYS`: it
/// survives logout and account changes.
const BOUND_ACCOUNT_KEY: &str = "derived_bound_account";
/// The first account ever bound to this archive (§2.9), written once and never
/// overwritten. Not in `SESSION_META_KEYS` either.
const OWNER_ACCOUNT_KEY: &str = "derived_owner_account";
/// Literal prefix of the per-account backfill marks (§2.3b). They describe local
/// rows, so logout keeps them.
#[allow(dead_code)]
const BACKFILL_PREFIX: &str = "derived_backfill:";

/// Channel account metadata dropped on logout / account change (§2.9). Defined
/// here from the start so the transport slice has one owner for them: the
/// capability the server advertised, the catch-up epoch, the pull cursor, when
/// the last pull started (the freshness barrier of §2.6) and the wait verdict.
const CAPABILITY_KEY: &str = "derived_capability";
const CATCHUP_EPOCH_KEY: &str = "derived_catchup_epoch";
const PULL_CURSOR_KEY: &str = "derived_cursor";
const LAST_PULL_STARTED_AT_KEY: &str = "derived_last_pull_started_at";
const WAIT_VERDICT_KEY: &str = "derived_wait_verdict";

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// SHA-256 as lowercase hex.
fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// A `sync_meta` value that is present and not blank; `''` never counts.
fn meta_account(conn: &Connection, key: &str) -> Result<Option<String>, String> {
    Ok(meta_get(conn, key)?.filter(|value| !value.trim().is_empty()))
}

/// The account of the active session, if any.
fn session_account(conn: &Connection) -> Result<Option<String>, String> {
    meta_account(conn, "account_id")
}

/// Fallible form of [`effective_account`], for callers that surface database
/// errors (the pull/push transports of §2.9).
#[allow(dead_code)]
pub(crate) fn read_effective_account(conn: &Connection) -> Result<Option<String>, String> {
    match session_account(conn)? {
        Some(account) => Ok(Some(account)),
        // No session: the archive's owner still has the content locally, and
        // reading it is never paying twice for it (§2.9).
        None => meta_account(conn, BOUND_ACCOUNT_KEY),
    }
}

/// The account whose rows may be read: the session account, else the bound
/// account, else `None` (§2.9). A metadata read failure yields `None`, which
/// fails closed: nothing is read for an account the archive cannot name.
#[allow(dead_code)]
pub(crate) fn effective_account(conn: &Connection) -> Option<String> {
    read_effective_account(conn).ok().flatten()
}

/// The `account_id` to stamp on a row written right now: the session account,
/// else the bound account, else `''` (never bound) — §2.9. It only produces `''`
/// where `effective_account` does, so a read failure fails closed to the one
/// value the first-ever binding adopts.
#[allow(dead_code)]
pub(crate) fn write_account(conn: &Connection) -> String {
    effective_account(conn).unwrap_or_default()
}

/// Binds this archive to the account that just logged in, inside
/// [`crate::sync::session::write_sync_session`]'s transaction (§2.9). On the
/// archive's first-ever binding this also adopts the `''` rows and seals both
/// marks; later bindings only move [`BOUND_ACCOUNT_KEY`].
pub(crate) fn bind_account(conn: &Connection, account_id: &str) -> Result<(), String> {
    if account_id.trim().is_empty() {
        // Adoption deletes the `''` rows, so a blank account would destroy them.
        return Err("[sync] cannot bind derived rows to a blank account".to_string());
    }
    if meta_account(conn, BOUND_ACCOUNT_KEY)?.is_none() {
        adopt_unbound_rows(conn, account_id)?;
        meta_set(conn, OWNER_ACCOUNT_KEY, account_id)?;
    }
    meta_set(conn, BOUND_ACCOUNT_KEY, account_id)
}

/// One-shot migration of the rows written before this archive's first login:
/// they belong to whoever logs in first. On a primary-key collision the account's
/// own row wins and the `''` one is dropped (§2.9).
fn adopt_unbound_rows(conn: &Connection, account_id: &str) -> Result<(), String> {
    // `INSERT OR IGNORE ... SELECT` from the same table: the inserted rows carry
    // `account_id`, so the `WHERE account_id = ''` scan can never see them again.
    conn.execute(
        "INSERT OR IGNORE INTO sync_derived_vectors
           (account_id, contract_hash, input_hash, dimensions, embedding, origin, pushed, created_at)
         SELECT ?1, contract_hash, input_hash, dimensions, embedding, origin, pushed, created_at
           FROM sync_derived_vectors
          WHERE account_id = ''",
        [account_id],
    )
    .map_err(|e| format!("[sync] failed to adopt unbound derived vectors: {e}"))?;
    conn.execute("DELETE FROM sync_derived_vectors WHERE account_id = ''", [])
        .map_err(|e| format!("[sync] failed to drop adopted derived vectors: {e}"))?;

    conn.execute(
        "INSERT OR IGNORE INTO sync_derived_ocr_pages
           (account_id, source_hash, page_number, ocr_contract, outcome, text_content,
            text_hash, origin, pushed, created_at)
         SELECT ?1, source_hash, page_number, ocr_contract, outcome, text_content,
                text_hash, origin, pushed, created_at
           FROM sync_derived_ocr_pages
          WHERE account_id = ''",
        [account_id],
    )
    .map_err(|e| format!("[sync] failed to adopt unbound OCR pages: {e}"))?;
    conn.execute("DELETE FROM sync_derived_ocr_pages WHERE account_id = ''", [])
        .map_err(|e| format!("[sync] failed to drop adopted OCR pages: {e}"))?;
    Ok(())
}

/// Seeds the two durable marks of an archive that already had a live session
/// when the derived tables were created (§2.9). An existing mark is never
/// overwritten.
pub(crate) fn ensure_account_marks(conn: &Connection) -> Result<(), String> {
    if !conn.is_autocommit() {
        // The schema call came from inside a transaction: write inline instead of
        // nesting a second one.
        return seed_account_marks(conn);
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("[sync] failed to begin derived mark transaction: {e}"))?;
    seed_account_marks(&tx)?;
    tx.commit()
        .map_err(|e| format!("[sync] failed to commit derived marks: {e}"))
}

fn seed_account_marks(conn: &Connection) -> Result<(), String> {
    // Only an archive that is already logged in needs the upgrade path: a fresh
    // install has no account, and its first login seals both marks below.
    let Some(account) = session_account(conn)? else {
        return Ok(());
    };
    for key in [BOUND_ACCOUNT_KEY, OWNER_ACCOUNT_KEY] {
        if meta_account(conn, key)?.is_none() {
            meta_set(conn, key, &account)?;
        }
    }
    Ok(())
}

/// Logout / account change (§2.9): reset `pushed` on every row of both tables and
/// drop the channel's account metadata. Rows and backfill marks stay.
pub(crate) fn clear_account_metadata(conn: &Connection) -> Result<(), String> {
    // Same pattern as `sync_blob_index.uploaded = 0`: only the flag is session
    // state, the content is not.
    conn.execute_batch(
        "UPDATE sync_derived_vectors SET pushed = 0;
         UPDATE sync_derived_ocr_pages SET pushed = 0;",
    )
    .map_err(|e| format!("[sync] failed to reset derived push flags: {e}"))?;
    conn.execute(
        "DELETE FROM sync_meta WHERE key IN (?1, ?2, ?3, ?4, ?5)",
        params![
            CAPABILITY_KEY,
            CATCHUP_EPOCH_KEY,
            PULL_CURSOR_KEY,
            LAST_PULL_STARTED_AT_KEY,
            WAIT_VERDICT_KEY
        ],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to clear derived channel metadata: {e}"))
}

/// Records one paid embedding. First writer wins (`INSERT OR IGNORE`): the same
/// `(account, contract, input)` is the same content, so a later duplicate — a
/// remote copy, a retried call — never replaces the stored vector.
#[allow(dead_code)]
pub(crate) fn insert_vector(
    conn: &Connection,
    account: &str,
    contract_hash: &str,
    input_hash: &str,
    dimensions: i64,
    embedding: &[u8],
    origin: &str,
) -> Result<(), String> {
    let expected = dimensions
        .checked_mul(4)
        .filter(|bytes| *bytes >= 0)
        .ok_or_else(|| format!("[sync] invalid embedding dimensions {dimensions}"))?;
    if embedding.len() as i64 != expected {
        return Err(format!(
            "[sync] derived vector '{input_hash}' is {} bytes but dimensions {dimensions} require {expected}",
            embedding.len()
        ));
    }
    conn.execute(
        "INSERT OR IGNORE INTO sync_derived_vectors
           (account_id, contract_hash, input_hash, dimensions, embedding, origin, pushed, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
        params![
            account,
            contract_hash,
            input_hash,
            dimensions,
            embedding,
            origin,
            now_ms()
        ],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to cache a derived vector: {e}"))
}

/// Records one paid OCR answer, including an empty answer (`outcome='empty'`,
/// `text=''`): the provider did charge for it, so it travels like any other.
/// First writer wins.
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn insert_ocr_page(
    conn: &Connection,
    account: &str,
    source_hash: &str,
    page_number: i64,
    ocr_contract: &str,
    outcome: &str,
    text: &str,
    origin: &str,
) -> Result<(), String> {
    match outcome {
        "text" => {}
        "empty" if text.is_empty() => {}
        "empty" => {
            return Err("[sync] an empty OCR outcome must not carry text".to_string());
        }
        other => {
            return Err(format!(
                "[sync] invalid OCR outcome '{other}': expected 'text' or 'empty'"
            ));
        }
    }
    let text_hash = sha256_hex(text.as_bytes());
    conn.execute(
        "INSERT OR IGNORE INTO sync_derived_ocr_pages
           (account_id, source_hash, page_number, ocr_contract, outcome, text_content,
            text_hash, origin, pushed, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9)",
        params![
            account,
            source_hash,
            page_number,
            ocr_contract,
            outcome,
            text,
            text_hash,
            origin,
            now_ms()
        ],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to cache an OCR page: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::schema::{ensure_sync_schema, SYNC_TABLES};
    use crate::sync::session::{
        clear_sync_state, meta_get, meta_set, write_sync_session, SESSION_STATE_TABLES,
    };
    use crate::sync::test_support::{new_app_schema_db, new_synced_test_db};

    const ACCOUNT_A: &str = "account-a";
    const ACCOUNT_B: &str = "account-b";
    const CONTRACT: &str = "contract-v1";
    const OCR_CONTRACT: &str = "glm-ocr/page/v1";
    const SOURCE_HASH: &str = "sha256-of-the-file";

    fn login(conn: &Connection, account: &str) {
        write_sync_session(
            conn,
            "https://sync.example.test",
            account,
            "reader@example.test",
            "device-a",
        )
        .expect("write sync session");
    }

    fn count(conn: &Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get(0))
            .expect("count rows")
    }

    fn account_count(conn: &Connection, table: &str, account: &str) -> i64 {
        conn.query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE account_id = ?1"),
            [account],
            |row| row.get(0),
        )
        .expect("count account rows")
    }

    fn stored_embedding(conn: &Connection, account: &str, input_hash: &str) -> Vec<u8> {
        conn.query_row(
            "SELECT embedding FROM sync_derived_vectors
              WHERE account_id = ?1 AND contract_hash = ?2 AND input_hash = ?3",
            params![account, CONTRACT, input_hash],
            |row| row.get(0),
        )
        .expect("stored embedding")
    }

    fn stored_page(conn: &Connection, account: &str, page_number: i64) -> (String, String, String) {
        conn.query_row(
            "SELECT outcome, text_content, text_hash FROM sync_derived_ocr_pages
              WHERE account_id = ?1 AND source_hash = ?2 AND page_number = ?3
                AND ocr_contract = ?4",
            params![account, SOURCE_HASH, page_number, OCR_CONTRACT],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("stored OCR page")
    }

    fn max_pushed(conn: &Connection, table: &str) -> i64 {
        conn.query_row(
            &format!("SELECT COALESCE(MAX(pushed), 0) FROM {table}"),
            [],
            |row| row.get(0),
        )
        .expect("max pushed")
    }

    fn blob(dimensions: i64, value: u8) -> Vec<u8> {
        vec![value; (dimensions * 4) as usize]
    }

    /// §2.9: the rows are content, not session state, so logout keeps them — but
    /// the channel's account metadata is gone and every row must be pushed again
    /// by whoever logs in next.
    #[test]
    fn logout_keeps_derived_rows_with_pushed_reset_and_clears_channel_meta() {
        let conn = new_synced_test_db();
        login(&conn, ACCOUNT_A);
        insert_vector(&conn, ACCOUNT_A, CONTRACT, "input-1", 1, &blob(1, 7), "local")
            .expect("paid vector");
        insert_ocr_page(
            &conn,
            ACCOUNT_A,
            SOURCE_HASH,
            1,
            OCR_CONTRACT,
            "text",
            "page one",
            "local",
        )
        .expect("paid page");
        insert_ocr_page(
            &conn,
            ACCOUNT_A,
            SOURCE_HASH,
            2,
            OCR_CONTRACT,
            "empty",
            "",
            "local",
        )
        .expect("paid empty page");
        // The transport already confirmed both tables against the server.
        conn.execute_batch(
            "UPDATE sync_derived_vectors SET pushed = 1;
             UPDATE sync_derived_ocr_pages SET pushed = 1;",
        )
        .expect("mark pushed");
        for (key, value) in [
            (CAPABILITY_KEY, "{\"epoch\":\"ep-1\",\"advertised\":true}"),
            (CATCHUP_EPOCH_KEY, "ep-1"),
            (PULL_CURSOR_KEY, "42"),
            (LAST_PULL_STARTED_AT_KEY, "1700000000000"),
            (WAIT_VERDICT_KEY, "{\"epoch\":\"ep-1\",\"advertised\":true}"),
        ] {
            meta_set(&conn, key, value).expect("channel metadata");
        }
        let backfill_mark = format!("{BACKFILL_PREFIX}{ACCOUNT_A}:{CONTRACT}");
        meta_set(&conn, &backfill_mark, "done").expect("backfill mark");

        clear_sync_state(&conn).expect("logout");

        assert_eq!(
            account_count(&conn, "sync_derived_vectors", ACCOUNT_A),
            1,
            "a paid vector survives logout"
        );
        assert_eq!(
            account_count(&conn, "sync_derived_ocr_pages", ACCOUNT_A),
            2,
            "paid pages survive logout"
        );
        assert_eq!(
            max_pushed(&conn, "sync_derived_vectors"),
            0,
            "vectors are pending for the next account"
        );
        assert_eq!(
            max_pushed(&conn, "sync_derived_ocr_pages"),
            0,
            "OCR pages are pending for the next account"
        );
        for key in [
            CAPABILITY_KEY,
            CATCHUP_EPOCH_KEY,
            PULL_CURSOR_KEY,
            LAST_PULL_STARTED_AT_KEY,
            WAIT_VERDICT_KEY,
        ] {
            assert_eq!(
                meta_get(&conn, key).expect("read channel metadata"),
                None,
                "channel metadata {key} cleared on logout"
            );
        }
        assert_eq!(
            meta_get(&conn, BOUND_ACCOUNT_KEY).expect("read bound mark"),
            Some(ACCOUNT_A.to_string()),
            "the binding mark is durable"
        );
        assert_eq!(
            meta_get(&conn, OWNER_ACCOUNT_KEY).expect("read owner mark"),
            Some(ACCOUNT_A.to_string()),
            "the owner mark is durable"
        );
        assert!(
            meta_get(&conn, &backfill_mark).expect("read backfill mark").is_some(),
            "backfill marks describe local rows and survive"
        );
        assert_eq!(
            meta_get(&conn, "account_id").expect("read session account"),
            None,
            "the session itself is gone"
        );
    }

    /// WU1 case: A logs out, OCR is processed offline, B logs in. Those rows are
    /// A's (never `''`), B neither reads nor pushes them, and A gets them back.
    #[test]
    fn rows_written_offline_after_a_logout_belong_to_a_and_never_to_b() {
        let conn = new_synced_test_db();
        login(&conn, ACCOUNT_A);
        clear_sync_state(&conn).expect("A logs out");

        assert_eq!(
            effective_account(&conn).as_deref(),
            Some(ACCOUNT_A),
            "without a session the archive still knows its owner"
        );
        let offline_account = write_account(&conn);
        assert_eq!(
            offline_account, ACCOUNT_A,
            "offline rows take the last bound account, never ''"
        );
        insert_vector(
            &conn,
            &offline_account,
            CONTRACT,
            "offline-input",
            1,
            &blob(1, 1),
            "local",
        )
        .expect("offline vector");
        insert_ocr_page(
            &conn,
            &offline_account,
            SOURCE_HASH,
            1,
            OCR_CONTRACT,
            "text",
            "offline page",
            "local",
        )
        .expect("offline page");

        login(&conn, ACCOUNT_B);
        assert_eq!(write_account(&conn), ACCOUNT_B, "new rows belong to B");
        assert_eq!(
            effective_account(&conn).as_deref(),
            Some(ACCOUNT_B),
            "B may only read its own rows"
        );
        assert_eq!(
            account_count(&conn, "sync_derived_vectors", ACCOUNT_B),
            0,
            "B never sees A's offline vector"
        );
        assert_eq!(
            account_count(&conn, "sync_derived_ocr_pages", ACCOUNT_B),
            0,
            "B never sees A's offline OCR"
        );
        assert_eq!(
            account_count(&conn, "sync_derived_vectors", ACCOUNT_A),
            1,
            "the offline vector stays A's"
        );
        assert_eq!(
            account_count(&conn, "sync_derived_ocr_pages", ACCOUNT_A),
            1,
            "the offline page stays A's"
        );
        assert_eq!(
            account_count(&conn, "sync_derived_vectors", ""),
            0,
            "nothing was left unbound"
        );

        // B's own result does not touch A's row.
        insert_vector(&conn, &write_account(&conn), CONTRACT, "b-input", 1, &blob(1, 2), "local")
            .expect("B vector");
        assert_eq!(account_count(&conn, "sync_derived_vectors", ACCOUNT_B), 1);
        assert_eq!(account_count(&conn, "sync_derived_vectors", ACCOUNT_A), 1);

        login(&conn, ACCOUNT_A);
        assert_eq!(
            effective_account(&conn).as_deref(),
            Some(ACCOUNT_A),
            "A gets its rows back"
        );
        assert_eq!(write_account(&conn), ACCOUNT_A);
        assert_eq!(account_count(&conn, "sync_derived_vectors", ACCOUNT_A), 1);
    }

    /// WU1 case: on an archive with no prior binding, what was written before the
    /// first login is adopted by that account, and both marks get written.
    #[test]
    fn first_binding_adopts_unbound_rows_and_seals_both_marks() {
        let conn = new_synced_test_db();
        assert_eq!(
            write_account(&conn),
            "",
            "an archive that was never bound writes ''"
        );
        insert_vector(&conn, "", CONTRACT, "pre-login", 1, &blob(1, 3), "local")
            .expect("unbound vector");
        insert_ocr_page(
            &conn,
            "",
            SOURCE_HASH,
            3,
            OCR_CONTRACT,
            "text",
            "pre-login page",
            "local",
        )
        .expect("unbound page");
        // A row the logging-in account already owns wins the key collision.
        insert_vector(&conn, ACCOUNT_A, CONTRACT, "collides", 1, &blob(1, 9), "local")
            .expect("account vector");
        insert_vector(&conn, "", CONTRACT, "collides", 1, &blob(1, 4), "local")
            .expect("unbound colliding vector");

        login(&conn, ACCOUNT_A);

        assert_eq!(
            account_count(&conn, "sync_derived_vectors", ""),
            0,
            "no unbound row survives the first binding"
        );
        assert_eq!(
            account_count(&conn, "sync_derived_ocr_pages", ""),
            0,
            "no unbound page survives the first binding"
        );
        assert_eq!(account_count(&conn, "sync_derived_vectors", ACCOUNT_A), 2);
        assert_eq!(account_count(&conn, "sync_derived_ocr_pages", ACCOUNT_A), 1);
        assert_eq!(
            stored_embedding(&conn, ACCOUNT_A, "collides"),
            blob(1, 9),
            "on a key collision the account's row wins and the '' row is dropped"
        );
        assert_eq!(
            meta_get(&conn, BOUND_ACCOUNT_KEY).expect("read bound mark"),
            Some(ACCOUNT_A.to_string()),
            "the first binding seals the binding mark"
        );
        assert_eq!(
            meta_get(&conn, OWNER_ACCOUNT_KEY).expect("read owner mark"),
            Some(ACCOUNT_A.to_string()),
            "the first binding seals the owner mark"
        );
    }

    /// §2.9: later bindings move `derived_bound_account` only; the owner mark is
    /// written once and the `''` adoption never runs again.
    #[test]
    fn later_bindings_move_the_bound_mark_but_never_the_owner() {
        let conn = new_synced_test_db();
        login(&conn, ACCOUNT_A);
        clear_sync_state(&conn).expect("A logs out");
        login(&conn, ACCOUNT_B);

        assert_eq!(
            meta_get(&conn, BOUND_ACCOUNT_KEY).expect("read bound mark"),
            Some(ACCOUNT_B.to_string())
        );
        assert_eq!(
            meta_get(&conn, OWNER_ACCOUNT_KEY).expect("read owner mark"),
            Some(ACCOUNT_A.to_string()),
            "the owner mark is never overwritten"
        );

        // A stray unbound row written after the first binding stays inert: the
        // adoption runs once per archive, not once per account.
        conn.execute(
            "INSERT INTO sync_derived_vectors
               (account_id, contract_hash, input_hash, dimensions, embedding, origin, pushed, created_at)
             VALUES ('', ?1, 'stray', 1, ?2, 'local', 0, 1)",
            params![CONTRACT, blob(1, 5)],
        )
        .expect("stray unbound vector");
        login(&conn, ACCOUNT_A);
        assert_eq!(
            account_count(&conn, "sync_derived_vectors", ""),
            1,
            "only the archive's first binding adopts unbound rows"
        );
        assert_eq!(
            meta_get(&conn, BOUND_ACCOUNT_KEY).expect("read bound mark"),
            Some(ACCOUNT_A.to_string())
        );
        assert_eq!(
            meta_get(&conn, OWNER_ACCOUNT_KEY).expect("read owner mark"),
            Some(ACCOUNT_A.to_string())
        );
    }

    /// §2.9 upgrade path: an archive whose session predates the marks inherits
    /// both from the live session when the tables are created. A fresh archive
    /// learns them from its first login instead.
    #[test]
    fn an_upgraded_archive_inherits_the_marks_from_its_live_session() {
        let upgraded = new_app_schema_db();
        upgraded
            .execute_batch("CREATE TABLE IF NOT EXISTS sync_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);")
            .expect("pre-mark sync_meta");
        for (key, value) in [
            ("account_id", ACCOUNT_A),
            ("server_url", "https://sync.example.test"),
            ("device_id", "device-a"),
        ] {
            meta_set(&upgraded, key, value).expect("live pre-mark session");
        }

        ensure_sync_schema(&upgraded).expect("upgrade");

        assert_eq!(
            meta_get(&upgraded, BOUND_ACCOUNT_KEY).expect("read bound mark"),
            Some(ACCOUNT_A.to_string()),
            "a live session seeds the binding mark"
        );
        assert_eq!(
            meta_get(&upgraded, OWNER_ACCOUNT_KEY).expect("read owner mark"),
            Some(ACCOUNT_A.to_string()),
            "a live session seeds the owner mark"
        );

        let fresh = new_synced_test_db();
        assert_eq!(
            meta_get(&fresh, BOUND_ACCOUNT_KEY).expect("read bound mark"),
            None,
            "a fresh archive has no binding yet"
        );
        assert_eq!(
            meta_get(&fresh, OWNER_ACCOUNT_KEY).expect("read owner mark"),
            None,
            "a fresh archive has no owner yet"
        );
    }

    /// Writing is idempotent (first writer wins) and never stores a vector whose
    /// byte width disagrees with its declared dimensions.
    #[test]
    fn insert_helpers_are_idempotent_and_reject_unusable_input() {
        let conn = new_synced_test_db();
        login(&conn, ACCOUNT_A);

        insert_vector(&conn, ACCOUNT_A, CONTRACT, "input", 1, &blob(1, 1), "local")
            .expect("first vector");
        insert_vector(&conn, ACCOUNT_A, CONTRACT, "input", 1, &blob(1, 2), "remote")
            .expect("a duplicate is ignored, not an error");
        assert_eq!(account_count(&conn, "sync_derived_vectors", ACCOUNT_A), 1);
        assert_eq!(
            stored_embedding(&conn, ACCOUNT_A, "input"),
            blob(1, 1),
            "the first writer wins"
        );

        let error = insert_vector(&conn, ACCOUNT_A, CONTRACT, "narrow", 2, &blob(1, 3), "local")
            .expect_err("a wrong-width blob is rejected");
        assert!(error.contains("dimensions"), "{error}");
        assert_eq!(
            account_count(&conn, "sync_derived_vectors", ACCOUNT_A),
            1,
            "a rejected vector writes nothing"
        );

        insert_ocr_page(
            &conn,
            ACCOUNT_A,
            SOURCE_HASH,
            1,
            OCR_CONTRACT,
            "text",
            "page one",
            "local",
        )
        .expect("first page");
        insert_ocr_page(
            &conn,
            ACCOUNT_A,
            SOURCE_HASH,
            1,
            OCR_CONTRACT,
            "text",
            "other text",
            "remote",
        )
        .expect("a duplicate page is ignored");
        assert_eq!(account_count(&conn, "sync_derived_ocr_pages", ACCOUNT_A), 1);
        let (outcome, text, text_hash) = stored_page(&conn, ACCOUNT_A, 1);
        assert_eq!((outcome.as_str(), text.as_str()), ("text", "page one"));
        assert_eq!(
            text_hash,
            sha256_hex(b"page one"),
            "text_hash is the sha256 of the stored answer"
        );

        insert_ocr_page(
            &conn,
            ACCOUNT_A,
            SOURCE_HASH,
            2,
            OCR_CONTRACT,
            "empty",
            "",
            "local",
        )
        .expect("an empty answer is a paid result too");
        let (outcome, text, text_hash) = stored_page(&conn, ACCOUNT_A, 2);
        assert_eq!((outcome.as_str(), text.as_str()), ("empty", ""));
        assert_eq!(text_hash, sha256_hex(b""));

        let error = insert_ocr_page(
            &conn,
            ACCOUNT_A,
            SOURCE_HASH,
            3,
            OCR_CONTRACT,
            "failed",
            "boom",
            "local",
        )
        .expect_err("a failed call is not a result");
        assert!(error.contains("outcome"), "{error}");
        let error = insert_ocr_page(
            &conn,
            ACCOUNT_A,
            SOURCE_HASH,
            4,
            OCR_CONTRACT,
            "empty",
            "leaked text",
            "local",
        )
        .expect_err("an empty answer carries no text");
        assert!(error.contains("empty"), "{error}");
        assert_eq!(
            account_count(&conn, "sync_derived_ocr_pages", ACCOUNT_A),
            2,
            "rejected pages write nothing"
        );
    }

    /// §2.2 DDL: the tables are in `SYNC_TABLES`, both carry `account_id` in the
    /// primary key, and the CHECK constraints reject values the plan rules out.
    #[test]
    fn the_derived_tables_carry_the_plan_constraints() {
        let conn = new_synced_test_db();
        for table in [
            "sync_derived_vectors",
            "sync_derived_ocr_pages",
            "sync_derived_contracts",
        ] {
            assert!(SYNC_TABLES.contains(&table), "{table} is in SYNC_TABLES");
        }

        let error = conn
            .execute(
                "INSERT INTO sync_derived_vectors
                   (account_id, contract_hash, input_hash, dimensions, embedding, origin, pushed, created_at)
                 VALUES ('a', 'c', 'i', 1, ?1, 'bogus', 0, 1)",
                params![blob(1, 1)],
            )
            .expect_err("origin CHECK rejects values outside local/remote");
        assert!(
            error.to_string().contains("CHECK"),
            "origin is a CHECK constraint: {error}"
        );

        let error = conn
            .execute(
                "INSERT INTO sync_derived_ocr_pages
                   (account_id, source_hash, page_number, ocr_contract, outcome, text_content,
                    text_hash, origin, pushed, created_at)
                 VALUES ('a', 's', 1, 'c', 'failed', '', 'h', 'local', 0, 1)",
                [],
            )
            .expect_err("outcome CHECK rejects a failed call");
        assert!(
            error.to_string().contains("CHECK"),
            "outcome is a CHECK constraint: {error}"
        );

        conn.execute(
            "INSERT INTO sync_derived_contracts
               (contract_hash, provider, model, dimensions, chunking_contract)
             VALUES ('h', 'openrouter', 'model', 1024, 'chunk-v1')",
            [],
        )
        .expect("contract mirror row");
        let error = conn
            .execute(
                "INSERT INTO sync_derived_contracts
                   (contract_hash, provider, model, dimensions, chunking_contract)
                 VALUES ('h', 'openrouter', 'model', 1024, 'chunk-v1')",
                [],
            )
            .expect_err("the contract hash is the primary key");
        assert!(error.to_string().contains("UNIQUE"), "{error}");
    }

    /// Existing behavior this slice touches: `clear_sync_state` still empties
    /// every session state table and resets `sync_blob_index.uploaded`.
    #[test]
    fn clear_sync_state_still_wipes_session_tables_and_resets_blob_uploads() {
        let conn = new_synced_test_db();
        for table in [
            "sync_derived_vectors",
            "sync_derived_ocr_pages",
            "sync_derived_contracts",
        ] {
            assert!(
                !SESSION_STATE_TABLES.contains(&table),
                "{table} holds paid content, not session state"
            );
        }
        login(&conn, ACCOUNT_A);
        conn.execute_batch(
            "INSERT INTO sync_oplog(table_name,row_id,op,changed_at) VALUES('items','i1','U',1);
             INSERT INTO sync_row_versions(table_name,row_id,server_seq) VALUES('items','i1',5);
             INSERT INTO sync_conflicts(id,table_name,row_id,reason,created_at)
               VALUES('cf1','items','i1','lww_lost',1);
             INSERT INTO sync_pending_rows(table_name,row_id,server_seq,deleted,changed_at,device_id)
               VALUES('items','i2',7,0,1,'dev-2');
             INSERT INTO sync_pending_blobs(asset_id,sha256,rel_path,size)
               VALUES('a1','abc','assets/x',10);
             INSERT INTO sync_web_pending_blobs(capture_id,role,sha256,rel_path,size)
               VALUES('c1','file','abc','web/x',10);
             INSERT INTO sync_pending_fts(item_id) VALUES('i1');
             INSERT INTO sync_topic_aliases(remote_id,local_id) VALUES('r1','l1');
             INSERT INTO sync_blob_index(asset_id,sha256,size,file_mtime_ms,uploaded)
               VALUES('a1','deadbeef',10,1,1);",
        )
        .expect("populate session state");

        clear_sync_state(&conn).expect("logout");

        for table in SESSION_STATE_TABLES {
            assert_eq!(count(&conn, table), 0, "{table} is session state");
        }
        let (sha, uploaded): (String, i64) = conn
            .query_row(
                "SELECT sha256, uploaded FROM sync_blob_index WHERE asset_id = 'a1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("blob index row survives");
        assert_eq!(sha, "deadbeef", "content-derived hashes are retained");
        assert_eq!(uploaded, 0, "the uploaded flag is reset");
    }
}
