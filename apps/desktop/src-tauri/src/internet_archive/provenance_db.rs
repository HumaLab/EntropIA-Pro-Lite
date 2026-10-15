//! SQLite-backed provenance lookup for the Internet Archive import flow.
//!
//! [`SqliteProvenance`] loads, in one query per 500-identifier chunk, every
//! `items` row whose `metadata` carries a provenance identifier in a given
//! set, turns each `(id, collection_id, metadata)` row into a
//! [`ProvenanceRecord`] via [`from_row`](ProvenanceRecord::from_row), and
//! keeps the records in memory for the crate's [`summarize`](internet_archive::prepare::summarize)
//! to classify against. Rows with missing, malformed, or key-less metadata
//! are skipped — a single bad row must never fail the whole load (SQLite's
//! `json_extract` raises `malformed JSON` on invalid input, so the query
//! guards with `json_valid` first).
//!
//! [`ProvenanceSnapshot`] bundles the target-collection existence bit with
//! the loaded records so the command opens both under one SQLite lock and
//! never holds the mutex across an `.await`.

use internet_archive::prepare::{ProvenanceLookup, ProvenanceRecord};
use rusqlite::Connection;

/// How many identifiers go into a single `IN (...)` list. SQLite accepts far
/// more bind parameters, but small chunks keep each query cheap and let the
/// caller pass an arbitrarily long identifier set.
const IN_CHUNK_SIZE: usize = 500;

/// Provenance rows for one import preparation, loaded once from SQLite and
/// then filtered in memory by `(identifier, file_name)`.
#[derive(Debug, Default, Clone)]
pub struct SqliteProvenance {
    records: Vec<ProvenanceRecord>,
}

impl SqliteProvenance {
    /// Load every row whose `metadata` carries a provenance identifier in
    /// `identifiers`, in one query per [`IN_CHUNK_SIZE`] chunk. An empty set
    /// loads nothing without querying. Rows whose metadata is missing,
    /// malformed, or lacks the provenance key are skipped.
    pub fn load(conn: &Connection, identifiers: &[String]) -> rusqlite::Result<Self> {
        // Deduplicated first: a repeated identifier would only widen the
        // `IN` list and the in-memory pile for no reason.
        let mut unique: Vec<&str> = Vec::with_capacity(identifiers.len());
        {
            let mut seen = std::collections::HashSet::with_capacity(identifiers.len());
            for identifier in identifiers {
                if seen.insert(identifier.as_str()) {
                    unique.push(identifier.as_str());
                }
            }
        }
        let mut records = Vec::new();
        for chunk in unique.chunks(IN_CHUNK_SIZE) {
            // One query per chunk. `json_valid` guards the extract:
            // `json_extract` raises `malformed JSON` on invalid input
            // instead of returning NULL, and one bad row must not fail the
            // whole load. `CASE` (not `AND`) carries the guard so no plan
            // can evaluate the extract on a malformed row. Rows whose
            // metadata is NULL, scalar, or key-less extract to NULL and
            // never match the `IN` list.
            let placeholders = vec!["?"; chunk.len()].join(",");
            let sql = format!(
                "SELECT id, collection_id, metadata FROM items \
                 WHERE CASE WHEN json_valid(metadata) \
                 THEN json_extract(metadata, '$.__entropia_internet_archive.identifier') \
                 ELSE NULL END IN ({placeholders})"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })?;
            for row in rows {
                let (item_id, collection_id, metadata) = row?;
                // Malformed or key-less metadata parses to `None` and is
                // skipped; only usable provenance rows join the pile.
                if let Some(metadata) = metadata.as_deref() {
                    if let Some(record) =
                        ProvenanceRecord::from_row(&item_id, &collection_id, metadata)
                    {
                        records.push(record);
                    }
                }
            }
        }
        Ok(Self { records })
    }
}

impl ProvenanceLookup for SqliteProvenance {
    /// Every loaded record for `(identifier, file_name)` across all
    /// collections, like the crate's in-memory lookup in its own tests.
    fn find(&self, identifier: &str, file_name: &str) -> Vec<ProvenanceRecord> {
        self.records
            .iter()
            .filter(|record| record.identifier == identifier && record.file_name == file_name)
            .cloned()
            .collect()
    }
}

/// True when a collection with this id exists.
pub fn collection_exists(conn: &Connection, collection_id: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM collections WHERE id = ?1)",
        [collection_id],
        |row| row.get::<_, bool>(0),
    )
}

/// Existence bit plus provenance for one import target, opened under a
/// single SQLite lock by the command and handed to the prepare logic, so no
/// mutex is ever held across an `.await`.
#[derive(Debug, Default)]
pub struct ProvenanceSnapshot {
    /// Whether the target collection exists.
    pub collection_exists: bool,
    /// Records for the requested identifiers (possibly empty).
    pub provenance: SqliteProvenance,
}

impl ProvenanceSnapshot {
    /// Check the collection and load the provenance for `identifiers` on one
    /// connection. The provenance loads regardless of existence; the caller
    /// reports `collection_not_found` itself.
    pub fn open(
        conn: &Connection,
        collection_id: &str,
        identifiers: &[String],
    ) -> rusqlite::Result<Self> {
        Ok(Self {
            collection_exists: collection_exists(conn, collection_id)?,
            provenance: SqliteProvenance::load(conn, identifiers)?,
        })
    }
}

#[cfg(test)]
pub(crate) fn test_db() -> Connection {
    let conn = Connection::open_in_memory().expect("in-memory database");
    conn.execute_batch(
        "CREATE TABLE collections (
           id TEXT PRIMARY KEY,
           name TEXT NOT NULL,
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL
         );
         CREATE TABLE items (
           id TEXT PRIMARY KEY,
           title TEXT NOT NULL,
           collection_id TEXT NOT NULL,
           metadata TEXT,
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL
         );",
    )
    .expect("minimal corpus schema");
    conn
}

#[cfg(test)]
pub(crate) fn insert_test_collection(conn: &Connection, id: &str) {
    conn.execute(
        "INSERT INTO collections (id, name, created_at, updated_at)
         VALUES (?1, 'test collection', 0, 0)",
        [id],
    )
    .expect("insert collection");
}

#[cfg(test)]
pub(crate) fn insert_test_item(
    conn: &Connection,
    id: &str,
    collection_id: &str,
    metadata: Option<&str>,
) {
    conn.execute(
        "INSERT INTO items (id, title, collection_id, metadata, created_at, updated_at)
         VALUES (?1, 'test item', ?2, ?3, 0, 0)",
        rusqlite::params![id, collection_id, metadata],
    )
    .expect("insert item");
}

/// Minimal provenance metadata value for tests: `{ PROVENANCE_KEY: {...} }`.
#[cfg(test)]
pub(crate) fn provenance_metadata(identifier: &str, file_name: &str, md5: Option<&str>) -> String {
    use internet_archive::provenance::PROVENANCE_KEY;
    let mut value = serde_json::json!({
        "identifier": identifier,
        "file_name": file_name,
    });
    if let Some(md5) = md5 {
        value["md5"] = serde_json::Value::String(md5.to_string());
    }
    serde_json::json!({ PROVENANCE_KEY: value }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_by_identifier_and_file_name_across_collections() {
        let conn = test_db();
        insert_test_collection(&conn, "col-a");
        insert_test_collection(&conn, "col-b");
        insert_test_item(
            &conn,
            "item-1",
            "col-a",
            Some(&provenance_metadata("ident-1", "a.pdf", Some("aa"))),
        );
        insert_test_item(
            &conn,
            "item-2",
            "col-b",
            Some(&provenance_metadata("ident-1", "a.pdf", Some("aa"))),
        );
        insert_test_item(
            &conn,
            "item-3",
            "col-a",
            Some(&provenance_metadata("ident-2", "b.pdf", Some("bb"))),
        );

        let provenance =
            SqliteProvenance::load(&conn, &["ident-1".to_string()]).expect("load succeeds");

        let mut found = provenance.find("ident-1", "a.pdf");
        found.sort_by(|a, b| a.item_id.cmp(&b.item_id));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].item_id, "item-1");
        assert_eq!(found[0].collection_id, "col-a");
        assert_eq!(found[1].item_id, "item-2");
        assert_eq!(found[1].collection_id, "col-b");
        // The key is (identifier, file_name): either half alone mismatches.
        assert!(provenance.find("ident-1", "other.pdf").is_empty());
        assert!(provenance.find("ident-2", "a.pdf").is_empty());
        // ident-2 was never loaded, so even its exact row is absent.
        assert!(provenance.find("ident-2", "b.pdf").is_empty());
    }

    #[test]
    fn skips_rows_without_the_key_malformed_or_null_metadata() {
        let conn = test_db();
        insert_test_collection(&conn, "col-a");
        insert_test_item(
            &conn,
            "good",
            "col-a",
            Some(&provenance_metadata("ident-1", "a.pdf", Some("aa"))),
        );
        // Valid JSON without the provenance key.
        insert_test_item(&conn, "no-key", "col-a", Some(r#"{"title": "plain"}"#));
        // Malformed JSON: must not fail the whole load.
        insert_test_item(&conn, "bad-json", "col-a", Some("{oops"));
        // NULL metadata.
        insert_test_item(&conn, "null-meta", "col-a", None);
        // Valid JSON scalar, no object to hold the key.
        insert_test_item(&conn, "scalar", "col-a", Some("42"));

        let provenance =
            SqliteProvenance::load(&conn, &["ident-1".to_string()]).expect("load succeeds");

        let found = provenance.find("ident-1", "a.pdf");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].item_id, "good");
    }

    #[test]
    fn chunking_loads_more_than_500_identifiers() {
        let conn = test_db();
        insert_test_collection(&conn, "col-a");
        let identifiers: Vec<String> = (0..600).map(|i| format!("ident-{i:04}")).collect();
        for (index, identifier) in identifiers.iter().enumerate() {
            insert_test_item(
                &conn,
                &format!("item-{index:04}"),
                "col-a",
                Some(&provenance_metadata(identifier, "doc.pdf", None)),
            );
        }

        let provenance = SqliteProvenance::load(&conn, &identifiers).expect("chunked load");

        // Every identifier made it through the 500-row chunk boundary.
        for identifier in &identifiers {
            assert_eq!(
                provenance.find(identifier, "doc.pdf").len(),
                1,
                "missing {identifier}"
            );
        }
        assert!(provenance.find("ident-9999", "doc.pdf").is_empty());
    }

    #[test]
    fn empty_identifier_set_loads_nothing_without_querying() {
        let conn = test_db();
        insert_test_collection(&conn, "col-a");
        let provenance = SqliteProvenance::load(&conn, &[]).expect("empty load");
        assert!(provenance.find("anything", "a.pdf").is_empty());
    }

    #[test]
    fn collection_exists_reports_true_and_false() {
        let conn = test_db();
        insert_test_collection(&conn, "col-a");
        assert!(collection_exists(&conn, "col-a").expect("query"));
        assert!(!collection_exists(&conn, "col-missing").expect("query"));
        assert!(!collection_exists(&conn, "").expect("query"));
    }

    #[test]
    fn snapshot_open_combines_existence_and_provenance() {
        let conn = test_db();
        insert_test_collection(&conn, "col-a");
        insert_test_item(
            &conn,
            "item-1",
            "col-a",
            Some(&provenance_metadata("ident-1", "a.pdf", Some("aa"))),
        );

        let snapshot =
            ProvenanceSnapshot::open(&conn, "col-a", &["ident-1".to_string()]).expect("open");
        assert!(snapshot.collection_exists);
        assert_eq!(snapshot.provenance.find("ident-1", "a.pdf").len(), 1);

        let missing =
            ProvenanceSnapshot::open(&conn, "col-gone", &["ident-1".to_string()]).expect("open");
        assert!(!missing.collection_exists);
        // The provenance still loads; the caller reports the missing target.
        assert_eq!(missing.provenance.find("ident-1", "a.pdf").len(), 1);
    }
}
