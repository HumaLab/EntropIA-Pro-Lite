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

use std::collections::HashMap;

use rusqlite::{params, params_from_iter, Connection};
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
    conn.execute(
        "DELETE FROM sync_derived_ocr_pages WHERE account_id = ''",
        [],
    )
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

/// True when this connection carries the derived cache tables.
///
/// [`crate::sync::schema::ensure_sync_schema`] creates them from the app's
/// `setup()` bootstrap (and from `sync_ensure_capture`), on the sync module's
/// own connection. A processing connection can still reach a publish before
/// that bootstrap ran — a fresh archive whose first import overlaps startup, a
/// repaired database, an archive that never synced — so every write below asks
/// first instead of failing on a missing table.
fn cache_tables_present(conn: &Connection) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master
          WHERE type = 'table'
            AND name IN ('sync_derived_vectors', 'sync_derived_ocr_pages',
                         'sync_derived_contracts')",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count == 3)
    .unwrap_or(false)
}

/// Copies the immutable embedding contract row into
/// `sync_derived_contracts` (§2.2), so the device that reads a vector also
/// knows its provider, model, width and chunking contract. First writer wins:
/// a contract hash names one space for good. The caller has already checked
/// [`cache_tables_present`].
fn mirror_contract(conn: &Connection, contract_hash: &str) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO sync_derived_contracts
           (contract_hash, provider, model, dimensions, chunking_contract)
         SELECT contract_hash, provider, model, dimensions, chunking_contract
           FROM bibliographic_embedding_contracts WHERE contract_hash = ?1",
        [contract_hash],
    )
    .map(|_| ())
    .map_err(|e| format!("[sync] failed to mirror derived contract {contract_hash}: {e}"))
}

/// Records one paid embedding from the transaction that publishes it (§2.3):
/// `origin='local'`, `pushed=0`, the account [`write_account`] names, and the
/// contract row mirrored beside it. A no-op on a connection without the
/// derived tables.
pub(crate) fn cache_paid_vector(
    conn: &Connection,
    contract_hash: &str,
    input_hash: &str,
    dimensions: i64,
    embedding: &[u8],
) -> Result<(), String> {
    if !cache_tables_present(conn) {
        return Ok(());
    }
    mirror_contract(conn, contract_hash)?;
    insert_vector(
        conn,
        &write_account(conn),
        contract_hash,
        input_hash,
        dimensions,
        embedding,
        "local",
    )
}

/// How many input hashes one cache read names in its `IN` list. Far below
/// SQLite's parameter limit, and enough to read a whole work's chunks in a
/// handful of index lookups.
const READ_BATCH: usize = 400;

/// The vectors the effective account already paid for under one contract,
/// keyed by input hash (§2.4). This is the read side of the cache: the device
/// that did not pay publishes these exactly as if it had computed them.
///
/// Only rows of [`effective_account`] answer — the session account or, without
/// a session, the archive's bound account (§2.9). Rows of `''` and rows of any
/// other account are inert, so a second device of the same account never pays
/// twice (S3) and a changed account never installs the previous one's results
/// (S7/D6).
///
/// Every returned blob is exactly `dimensions * 4` bytes and holds finite f32
/// values — the same validation the publisher applies to a computed vector.
/// These rows arrive from the server too (T8), so they are untrusted input: a
/// row that breaks either rule is unusable and skipped, never repaired, and a
/// read reports all of its skips in one summary line instead of one line per
/// row. A missing derived table, an unreadable row or an archive that cannot
/// name an account yields no hits: the caller then pays as it did before, and
/// the cache can never become a source of unknown or foreign vectors.
pub(crate) fn cached_vectors(
    conn: &Connection,
    contract_hash: &str,
    dimensions: usize,
    input_hashes: &[String],
) -> HashMap<String, Vec<u8>> {
    let mut hits = HashMap::new();
    if input_hashes.is_empty() || contract_hash.is_empty() || dimensions == 0 {
        return hits;
    }
    if !cache_tables_present(conn) {
        return hits;
    }
    let Some(account) = effective_account(conn) else {
        return hits;
    };
    let expected = dimensions * 4;
    let mut skipped_width = 0_usize;
    let mut skipped_non_finite = 0_usize;
    'batches: for batch in input_hashes.chunks(READ_BATCH) {
        let placeholders = vec!["?"; batch.len()].join(",");
        let sql = format!(
            "SELECT input_hash, embedding FROM sync_derived_vectors
              WHERE account_id = ?1 AND contract_hash = ?2
                AND input_hash IN ({placeholders})"
        );
        let mut statement = match conn.prepare(&sql) {
            Ok(statement) => statement,
            Err(error) => {
                log::warn!(target: "sync", "derived vector cache read skipped: {error}");
                break 'batches;
            }
        };
        let parameters = params_from_iter(
            std::iter::once(account.as_str())
                .chain(std::iter::once(contract_hash))
                .chain(batch.iter().map(String::as_str)),
        );
        let rows = match statement.query_map(parameters, |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
        }) {
            Ok(rows) => rows,
            Err(error) => {
                log::warn!(target: "sync", "derived vector cache read skipped: {error}");
                break 'batches;
            }
        };
        for row in rows {
            let (input_hash, embedding) = match row {
                Ok(row) => row,
                Err(error) => {
                    log::warn!(target: "sync", "derived vector row skipped: {error}");
                    continue;
                }
            };
            if embedding.len() != expected {
                skipped_width += 1;
                continue;
            }
            // A row can be the right width and still hold values no computation
            // would ever produce: the vector is only usable when every f32 is
            // finite, exactly as the computed path checks before publishing.
            let finite = embedding.chunks_exact(4).all(|value| {
                f32::from_le_bytes([value[0], value[1], value[2], value[3]]).is_finite()
            });
            if !finite {
                skipped_non_finite += 1;
                continue;
            }
            hits.insert(input_hash, embedding);
        }
    }
    let skipped = skipped_width + skipped_non_finite;
    if skipped > 0 {
        log::warn!(
            target: "sync",
            "skipped {skipped} derived vector row(s) under contract {}: {skipped_width} with a wrong width, {skipped_non_finite} with non-finite values",
            contract_hash.chars().take(12).collect::<String>()
        );
    }
    hits
}

/// Records one paid OCR answer — including an empty one — from the transaction
/// that publishes its page (§2.3). A no-op on a connection without the derived
/// tables.
#[allow(clippy::too_many_arguments)]
pub(crate) fn cache_paid_ocr_page(
    conn: &Connection,
    source_hash: &str,
    page_number: i64,
    ocr_contract: &str,
    outcome: &str,
    text: &str,
) -> Result<(), String> {
    if !cache_tables_present(conn) {
        return Ok(());
    }
    insert_ocr_page(
        conn,
        &write_account(conn),
        source_hash,
        page_number,
        ocr_contract,
        outcome,
        text,
        "local",
    )
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
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
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
        insert_vector(
            &conn,
            ACCOUNT_A,
            CONTRACT,
            "input-1",
            1,
            &blob(1, 7),
            "local",
        )
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
            meta_get(&conn, &backfill_mark)
                .expect("read backfill mark")
                .is_some(),
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
        insert_vector(
            &conn,
            &write_account(&conn),
            CONTRACT,
            "b-input",
            1,
            &blob(1, 2),
            "local",
        )
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
        insert_vector(
            &conn,
            ACCOUNT_A,
            CONTRACT,
            "collides",
            1,
            &blob(1, 9),
            "local",
        )
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
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS sync_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
            )
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
        insert_vector(
            &conn,
            ACCOUNT_A,
            CONTRACT,
            "input",
            1,
            &blob(1, 2),
            "remote",
        )
        .expect("a duplicate is ignored, not an error");
        assert_eq!(account_count(&conn, "sync_derived_vectors", ACCOUNT_A), 1);
        assert_eq!(
            stored_embedding(&conn, ACCOUNT_A, "input"),
            blob(1, 1),
            "the first writer wins"
        );

        let error = insert_vector(
            &conn,
            ACCOUNT_A,
            CONTRACT,
            "narrow",
            2,
            &blob(1, 3),
            "local",
        )
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

    /// §2.4 read contract: only the effective account's own rows, only under
    /// the asked contract, only blobs whose width matches and whose f32 values
    /// are all finite, and nothing at all for an archive that cannot name an
    /// account or has no derived tables.
    #[test]
    fn cached_vectors_only_reads_usable_rows_of_the_effective_account() {
        // No derived tables at all (an archive whose bootstrap never ran).
        let bare = Connection::open_in_memory().expect("bare db");
        assert!(
            cached_vectors(&bare, CONTRACT, 1, &["anything".to_string()]).is_empty(),
            "a missing cache reads as an empty cache"
        );

        let conn = new_synced_test_db();
        insert_vector(&conn, "", CONTRACT, "unbound", 1, &blob(1, 1), "local")
            .expect("unbound vector");
        insert_vector(
            &conn,
            ACCOUNT_A,
            CONTRACT,
            "a-input",
            1,
            &blob(1, 2),
            "local",
        )
        .expect("account vector");
        assert!(
            cached_vectors(
                &conn,
                CONTRACT,
                1,
                &["unbound".to_string(), "a-input".to_string()]
            )
            .is_empty(),
            "an archive that names no account reads nothing"
        );

        login(&conn, ACCOUNT_A);
        // The first binding adopted the `''` row; the account's own row stays.
        insert_vector(
            &conn,
            ACCOUNT_B,
            CONTRACT,
            "b-input",
            1,
            &blob(1, 9),
            "local",
        )
        .expect("other account vector");
        insert_vector(
            &conn,
            ACCOUNT_A,
            "other-contract",
            "a-input",
            1,
            &blob(1, 5),
            "local",
        )
        .expect("other contract vector");
        // A row whose declared width disagrees with its blob never landed
        // through the writer; a damaged or foreign archive can still hold one.
        conn.execute(
            "INSERT INTO sync_derived_vectors
               (account_id, contract_hash, input_hash, dimensions, embedding, origin, pushed, created_at)
             VALUES (?1, ?2, 'a-wide', 1, ?3, 'remote', 1, 1)",
            params![ACCOUNT_A, CONTRACT, vec![7_u8; 8]],
        )
        .expect("wrong-width vector");
        // Rows travel through the server (T8), so a right-width blob can still
        // carry values no computation would ever produce.
        insert_vector(
            &conn,
            ACCOUNT_A,
            CONTRACT,
            "a-nan",
            1,
            &f32::NAN.to_le_bytes(),
            "remote",
        )
        .expect("NaN vector");
        insert_vector(
            &conn,
            ACCOUNT_A,
            CONTRACT,
            "a-inf",
            1,
            &f32::INFINITY.to_le_bytes(),
            "remote",
        )
        .expect("infinite vector");

        let asked: Vec<String> = [
            "unbound", "a-input", "b-input", "a-wide", "a-nan", "a-inf", "missing",
        ]
        .iter()
        .map(|hash| hash.to_string())
        .collect();
        let hits = cached_vectors(&conn, CONTRACT, 1, &asked);
        assert_eq!(
            hits.get("a-input"),
            Some(&blob(1, 2)),
            "the account's own row is a hit"
        );
        assert_eq!(
            hits.get("unbound"),
            Some(&blob(1, 1)),
            "the adopted row belongs to the account now"
        );
        assert_eq!(hits.len(), 2, "no other row answers: {hits:?}");
        assert!(
            !hits.contains_key("b-input"),
            "another account's row is never served"
        );
        assert!(
            !hits.contains_key("a-wide"),
            "a wrong-width blob is unusable"
        );
        assert!(
            !hits.contains_key("a-nan"),
            "a NaN is not a vector: an untrusted row never answers"
        );
        assert!(
            !hits.contains_key("a-inf"),
            "an infinite value is not a vector either"
        );
        assert!(!hits.contains_key("missing"), "an absent key is a miss");

        assert!(
            cached_vectors(&conn, CONTRACT, 2, &["a-input".to_string()]).is_empty(),
            "the contract's dimensions decide the width, not the stored column"
        );
        assert!(
            cached_vectors(&conn, CONTRACT, 1, &[]).is_empty(),
            "no input hashes, no read"
        );

        // Without a session the archive reads through its bound account, so
        // the owner never pays twice for its own material (§2.9, S8).
        clear_sync_state(&conn).expect("logout");
        let offline = cached_vectors(&conn, CONTRACT, 1, &["a-input".to_string()]);
        assert_eq!(offline.get("a-input"), Some(&blob(1, 2)));
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
