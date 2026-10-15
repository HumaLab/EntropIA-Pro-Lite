/// Shared text retrieval for NLP modules.
///
/// Concatenates text from both `extractions` and `transcriptions` tables
/// for a given `item_id`, with graceful degradation if the transcriptions
/// table doesn't exist (pre-migration databases).
use rusqlite::{params, Connection, OptionalExtension};
use std::sync::OnceLock;

/// Log the "transcriptions table missing" warning only once, not per-item.
static TRANSCRIPTIONS_TABLE_MISSING_LOGGED: OnceLock<()> = OnceLock::new();

// ── Public API ───────────────────────────────────────────────────────────────

/// Concatenate all text content for `item_id` from extractions + transcriptions.
///
/// - Extractions text first (ASC by created_at), then transcriptions (ASC by created_at),
///   separated by single spaces.
/// - Returns `Ok("")` if no text found.
/// - Degrades gracefully if `transcriptions` table doesn't exist.
/// - Propagates non-"no such table" errors.
pub fn get_item_text(conn: &Connection, item_id: &str) -> Result<String, String> {
    // Query 1: extractions text (always expected to exist)
    // Order by asset sort_index first (for multi-page scanned PDFs page order),
    // then by created_at for stable ordering within the same asset.
    let mut extraction_texts: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT COALESCE(e.text_content, '') FROM extractions e JOIN assets a ON e.asset_id = a.id WHERE a.item_id = ?1 ORDER BY a.sort_index ASC, e.created_at ASC",
            )
            .map_err(|e| format!("Failed to prepare extractions query: {e}"))?;

        let rows = stmt
            .query_map(params![item_id], |row| row.get::<_, String>(0))
            .map_err(|e| format!("Failed to query extractions: {e}"))?;

        rows.filter_map(|r| r.ok()).collect()
    };

    // Query 2: transcriptions text (graceful degradation if table missing)
    // Also ordered by sort_index for consistent multi-asset aggregation.
    let transcription_texts: Vec<String> = match try_query_transcriptions(conn, item_id) {
        Ok(texts) => texts,
        Err(e) if e.contains("no such table") => {
            // Old DB without transcriptions table — log once, not per-item
            let _ = TRANSCRIPTIONS_TABLE_MISSING_LOGGED.get_or_init(|| {
                log::warn!(
                    target: "nlp/text_provider",
                    "transcriptions table not found — degrading to extraction-only text for all items"
                );
            });
            Vec::new() // No transcription text available
        }
        Err(e) => return Err(e),
    };

    extraction_texts.extend(transcription_texts);

    let combined = extraction_texts.join(" ");
    Ok(combined)
}

/// Retrieve extraction text for a single `asset_id`.
///
/// Returns `Ok("")` if no extraction exists for the asset.
/// This is used for asset-level NLP processing (NER, embeddings, triples)
/// where we only want to analyze the currently selected page/asset.
pub fn get_asset_text(conn: &Connection, asset_id: &str) -> Result<String, String> {
    // 1. Check for extraction text from the extractions table
    let extraction_text: String = conn
        .query_row(
            "SELECT COALESCE(text_content, '') FROM extractions WHERE asset_id = ?1 ORDER BY created_at ASC LIMIT 1",
            params![asset_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| format!("Failed to query extraction for asset {asset_id}: {e}"))?
        .unwrap_or_default();

    // 2. Check for transcription text (audio assets)
    let transcription_text: String = match conn
        .query_row(
            "SELECT COALESCE(text_content, '') FROM transcriptions WHERE asset_id = ?1 ORDER BY created_at ASC LIMIT 1",
            params![asset_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
    {
        Ok(Some(text)) => text,
        Ok(None) => String::new(),
        Err(e) if e.to_string().contains("no such table") => {
            let _ = TRANSCRIPTIONS_TABLE_MISSING_LOGGED.get_or_init(|| {
                log::warn!(
                    target: "nlp/text_provider",
                    "transcriptions table not found — degrading to extraction-only text"
                );
            });
            String::new()
        }
        Err(e) => return Err(format!("Failed to query transcription for asset {asset_id}: {e}")),
    };

    // Combine: extraction text first, then transcription
    let parts: Vec<&str> = [&extraction_text, &transcription_text]
        .iter()
        .filter(|s| !s.is_empty())
        .map(|s| s.as_str())
        .collect();

    Ok(parts.join(" "))
}

/// Attempt to query transcriptions text for an item.
/// Returns `Ok(texts)` on success, or `Err` with the rusqlite error message.
fn try_query_transcriptions(conn: &Connection, item_id: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT COALESCE(t.text_content, '') FROM transcriptions t JOIN assets a ON t.asset_id = a.id WHERE a.item_id = ?1 ORDER BY a.sort_index ASC, t.created_at ASC",
        )
        .map_err(|e| format!("{e}"))?;

    let rows = stmt
        .query_map(params![item_id], |row| row.get::<_, String>(0))
        .map_err(|e| format!("{e}"))?;

    Ok(rows.filter_map(|r| r.ok()).collect())
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Create an in-memory DB with items, assets, extractions, and transcriptions tables.
    fn setup_full_db() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        conn.execute_batch(
            r#"
            CREATE TABLE items (
              id TEXT PRIMARY KEY,
              collection_id TEXT,
              title TEXT NOT NULL,
              metadata TEXT
            );

            CREATE TABLE assets (
              id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              path TEXT NOT NULL,
              type TEXT NOT NULL,
              sort_index INTEGER NOT NULL DEFAULT 0,
              created_at INTEGER NOT NULL
            );

            CREATE TABLE extractions (
              id TEXT PRIMARY KEY,
              asset_id TEXT NOT NULL,
              text_content TEXT,
              created_at INTEGER NOT NULL
            );

            CREATE TABLE transcriptions (
              id TEXT PRIMARY KEY,
              asset_id TEXT NOT NULL,
              text_content TEXT NOT NULL,
              language TEXT,
              duration_ms INTEGER,
              model TEXT NOT NULL,
              segments TEXT,
              confidence REAL,
              created_at INTEGER NOT NULL
            );
            "#,
        )
        .expect("full schema should be created");
        conn
    }

    /// Create an in-memory DB WITHOUT the transcriptions table (old DB).
    fn setup_db_without_transcriptions() -> Connection {
        let conn = Connection::open_in_memory().expect("in-memory db should open");
        conn.execute_batch(
            r#"
            CREATE TABLE items (
              id TEXT PRIMARY KEY,
              collection_id TEXT,
              title TEXT NOT NULL,
              metadata TEXT
            );

            CREATE TABLE assets (
              id TEXT PRIMARY KEY,
              item_id TEXT NOT NULL,
              path TEXT NOT NULL,
              type TEXT NOT NULL,
              sort_index INTEGER NOT NULL DEFAULT 0,
              created_at INTEGER NOT NULL
            );

            CREATE TABLE extractions (
              id TEXT PRIMARY KEY,
              asset_id TEXT NOT NULL,
              text_content TEXT,
              created_at INTEGER NOT NULL
            );
            "#,
        )
        .expect("schema without transcriptions should be created");
        conn
    }

    fn seed_item(conn: &Connection, item_id: &str, asset_id: &str, title: &str) {
        conn.execute(
            "INSERT INTO items(id, collection_id, title, metadata) VALUES (?1, ?2, ?3, ?4)",
            params![item_id, "col-1", title, "{}"],
        )
        .expect("item insert should succeed");

        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, sort_index, created_at) VALUES (?1, ?2, ?3, ?4, 0, ?5)",
            params![asset_id, item_id, "asset.txt", "text", 1_i64],
        )
        .expect("asset insert should succeed");
    }

    fn seed_extraction(
        conn: &Connection,
        ext_id: &str,
        asset_id: &str,
        text: &str,
        created_at: i64,
    ) {
        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![ext_id, asset_id, text, created_at],
        )
        .expect("extraction insert should succeed");
    }

    fn seed_transcription(
        conn: &Connection,
        trans_id: &str,
        asset_id: &str,
        text: &str,
        created_at: i64,
    ) {
        conn.execute(
            "INSERT INTO transcriptions(id, asset_id, text_content, language, duration_ms, model, segments, confidence, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![trans_id, asset_id, text, "es", 1000_i64, "base", "[]", 0.9_f64, created_at],
        )
        .expect("transcription insert should succeed");
    }

    // ── Scenario: Item with both extractions and transcriptions ──────────────

    #[test]
    fn get_item_text_concatenates_extractions_and_transcriptions() {
        let conn = setup_full_db();
        seed_item(&conn, "item-1", "asset-1", "Title");

        // 2 extractions (ASC by created_at)
        seed_extraction(&conn, "ext-1", "asset-1", "alpha", 10);
        seed_extraction(&conn, "ext-2", "asset-1", "beta", 20);

        // 1 transcription
        seed_transcription(&conn, "trans-1", "asset-1", "gamma", 30);

        let result = get_item_text(&conn, "item-1").expect("should succeed");
        assert_eq!(
            result, "alpha beta gamma",
            "extractions should come first, then transcriptions, space-joined"
        );
    }

    // ── Scenario: Item with only extrations ─────────────────────────────────

    #[test]
    fn get_item_text_returns_extraction_only_text_when_no_transcriptions() {
        let conn = setup_full_db();
        seed_item(&conn, "item-2", "asset-2", "Title");

        seed_extraction(&conn, "ext-3", "asset-2", "ocr text one", 5);
        seed_extraction(&conn, "ext-4", "asset-2", "ocr text two", 15);

        let result = get_item_text(&conn, "item-2").expect("should succeed");
        assert_eq!(
            result, "ocr text one ocr text two",
            "should return extraction text only when no transcriptions exist"
        );
    }

    // ── Scenario: Item with only transcriptions ─────────────────────────────

    #[test]
    fn get_item_text_returns_transcription_only_text_when_no_extractions() {
        let conn = setup_full_db();
        seed_item(&conn, "item-3", "asset-3", "Title");

        seed_transcription(&conn, "trans-2", "asset-3", "audio transcript", 50);

        let result = get_item_text(&conn, "item-3").expect("should succeed");
        assert_eq!(
            result, "audio transcript",
            "should return transcription text when no extractions exist"
        );
    }

    // ── Scenario: Item with no text content at all ──────────────────────────

    #[test]
    fn get_item_text_returns_empty_string_for_item_with_no_text() {
        let conn = setup_full_db();
        seed_item(&conn, "item-empty", "asset-empty", "Title");

        let result = get_item_text(&conn, "item-empty").expect("should succeed");
        assert_eq!(
            result, "",
            "should return Ok(\"\") for items with no extractions or transcriptions"
        );
    }

    // ── Scenario: Item that doesn't exist at all ────────────────────────────

    #[test]
    fn get_item_text_returns_empty_string_for_nonexistent_item() {
        let conn = setup_full_db();

        let result = get_item_text(&conn, "nonexistent-item").expect("should succeed");
        assert_eq!(
            result, "",
            "should return Ok(\"\") for items that don't exist in the DB"
        );
    }

    // ── Scenario: Graceful degradation on missing transcriptions table ──────

    #[test]
    fn get_item_text_degrades_gracefully_when_transcriptions_table_missing() {
        let conn = setup_db_without_transcriptions();
        seed_item(&conn, "item-old", "asset-old", "Title");

        seed_extraction(&conn, "ext-old-1", "asset-old", "extraction only text", 100);

        let result =
            get_item_text(&conn, "item-old").expect("should succeed with extraction-only text");
        assert_eq!(
            result, "extraction only text",
            "should return extraction text when transcriptions table is absent"
        );
    }

    #[test]
    fn get_item_text_degrades_to_empty_when_transcriptions_table_missing_and_no_extractions() {
        let conn = setup_db_without_transcriptions();
        seed_item(&conn, "item-old-empty", "asset-old-empty", "Title");

        let result = get_item_text(&conn, "item-old-empty").expect("should succeed");
        assert_eq!(
            result, "",
            "should return Ok(\"\") when transcriptions missing and no extractions"
        );
    }

    // ── Scenario: Non-table-related error propagation ───────────────────────

    #[test]
    fn get_item_text_propagates_non_table_database_errors() {
        // Use a DB with a constraints violation to trigger a non-"no such table" error
        let conn = setup_full_db();
        seed_item(&conn, "item-err", "asset-err", "Title");
        seed_extraction(&conn, "ext-err", "asset-err", "some text", 1);

        // Insert a duplicate extraction id to cause a constraint violation on next query
        // This won't directly test get_item_text error propagation, so instead we test
        // with a closed connection scenario or corrupt query.

        // The simplest test: pass an item_id that causes a DB error.
        // We'll add a NOT NULL constraint violation by inserting NULL text_content
        // and testing COALESCE handles it — but COALESCE prevents that.
        // Better approach: test with a connection where extractions table has
        // a different schema that causes a real error.

        // Actually, the simplest reliable test for error propagation is:
        // drop the extractions table and verify we get Err (not "no such table"
        // error on extractions — that's the table we always expect to exist).
        conn.execute("DROP TABLE extractions", [])
            .expect("drop extractions");

        let result = get_item_text(&conn, "item-err");
        assert!(
            result.is_err(),
            "should return Err when extractions table is missing (not graceful degradation)"
        );
    }

    // ── Scenario: Ordering is correct (ASC by created_at) ───────────────────

    #[test]
    fn get_item_text_orders_extractions_by_created_at_asc() {
        let conn = setup_full_db();
        seed_item(&conn, "item-order", "asset-order", "Title");

        // Insert in reverse order — should be returned in ASC order
        seed_extraction(&conn, "ext-newer", "asset-order", "second", 200);
        seed_extraction(&conn, "ext-older", "asset-order", "first", 100);

        let result = get_item_text(&conn, "item-order").expect("should succeed");
        assert_eq!(
            result, "first second",
            "extractions should be ordered by created_at ASC"
        );
    }

    #[test]
    fn get_item_text_orders_transcriptions_by_created_at_asc() {
        let conn = setup_full_db();
        seed_item(&conn, "item-order2", "asset-order2", "Title");

        seed_transcription(&conn, "trans-newer", "asset-order2", "second audio", 200);
        seed_transcription(&conn, "trans-older", "asset-order2", "first audio", 100);

        let result = get_item_text(&conn, "item-order2").expect("should succeed");
        assert_eq!(
            result, "first audio second audio",
            "transcriptions should be ordered by created_at ASC"
        );
    }

    // ── Scenario: NULL text_content is handled via COALESCE ─────────────────

    #[test]
    fn get_item_text_handles_null_text_content_in_extractions() {
        let conn = setup_full_db();
        seed_item(&conn, "item-null", "asset-null", "Title");

        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, created_at) VALUES (?1, ?2, NULL, ?3)",
            params!["ext-null", "asset-null", 1_i64],
        )
        .expect("insert with NULL text_content");

        let result = get_item_text(&conn, "item-null").expect("should succeed");
        // COALESCE(NULL, '') = '', and empty strings are filtered by join
        assert_eq!(
            result, "",
            "NULL text_content should be treated as empty string"
        );
    }

    // ── Scenario: Multi-asset text aggregation respects sort_index ──────────

    #[test]
    fn get_item_text_orders_by_sort_index_then_created_at() {
        let conn = setup_full_db();

        // Item with 3 assets, sort_index determines order
        conn.execute(
            "INSERT INTO items(id, collection_id, title, metadata) VALUES (?1, ?2, ?3, ?4)",
            params!["item-multi", "col-1", "Multi", "{}"],
        )
        .expect("item insert");

        // Assets in "wrong" chronological order but correct sort_index
        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, sort_index, created_at) VALUES (?1, ?2, ?3, 'image', 2, 100)",
            params!["asset-page3", "item-multi", "page3.png"],
        )
        .expect("asset page3 insert");

        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, sort_index, created_at) VALUES (?1, ?2, ?3, 'image', 0, 300)",
            params!["asset-page1", "item-multi", "page1.png"],
        )
        .expect("asset page1 insert");

        conn.execute(
            "INSERT INTO assets(id, item_id, path, type, sort_index, created_at) VALUES (?1, ?2, ?3, 'image', 1, 200)",
            params!["asset-page2", "item-multi", "page2.png"],
        )
        .expect("asset page2 insert");

        // Extractions for each asset
        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, created_at) VALUES (?1, ?2, ?3, ?4)",
            params!["ext-3", "asset-page3", "third page", 300],
        )
        .expect("extraction page3");

        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, created_at) VALUES (?1, ?2, ?3, ?4)",
            params!["ext-1", "asset-page1", "first page", 100],
        )
        .expect("extraction page1");

        conn.execute(
            "INSERT INTO extractions(id, asset_id, text_content, created_at) VALUES (?1, ?2, ?3, ?4)",
            params!["ext-2", "asset-page2", "second page", 200],
        )
        .expect("extraction page2");

        let result = get_item_text(&conn, "item-multi").expect("should succeed");
        assert_eq!(
            result, "first page second page third page",
            "text should be aggregated in sort_index order, not created_at order"
        );
    }
}
