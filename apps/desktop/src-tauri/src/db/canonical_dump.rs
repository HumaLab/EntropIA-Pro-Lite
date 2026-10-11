//! Canonical schema dump — the equivalence oracle for the Phase 4 Rust
//! migration runner (odd/plans/plan-migraciones-rust.md §3).
//!
//! Mirrors `packages/store/src/schema-canonical.ts` field for field: the
//! TypeScript runner migrates an empty database and exports
//! `tests/fixtures/schema_canonical.json`; 4.2b asserts that a database
//! migrated by the Rust runner produces the same dump. Keep the serde names
//! and ordering in lockstep with the TypeScript side.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CanonicalSchemaDump {
    pub(crate) objects: Vec<CanonicalSchemaObject>,
    pub(crate) tables: Vec<CanonicalSchemaTable>,
    pub(crate) indexes: Vec<CanonicalSchemaIndex>,
    pub(crate) migrations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CanonicalSchemaObject {
    #[serde(rename = "type")]
    pub(crate) object_type: String,
    pub(crate) name: String,
    pub(crate) tbl_name: String,
    pub(crate) sql: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CanonicalSchemaColumn {
    pub(crate) cid: i64,
    pub(crate) name: String,
    #[serde(rename = "type")]
    pub(crate) column_type: String,
    pub(crate) notnull: i64,
    pub(crate) dflt_value: Option<String>,
    pub(crate) pk: i64,
    pub(crate) hidden: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CanonicalSchemaForeignKey {
    pub(crate) id: i64,
    pub(crate) seq: i64,
    pub(crate) table: String,
    pub(crate) from: String,
    pub(crate) to: Option<String>,
    pub(crate) on_update: String,
    pub(crate) on_delete: String,
    #[serde(rename = "match")]
    pub(crate) match_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CanonicalSchemaTable {
    pub(crate) name: String,
    pub(crate) columns: Vec<CanonicalSchemaColumn>,
    pub(crate) foreign_keys: Vec<CanonicalSchemaForeignKey>,
    // Set when the PRAGMA itself fails (exotic virtual tables); the matching
    // list stays empty so the failure is recorded instead of silently skipped.
    pub(crate) columns_error: Option<String>,
    pub(crate) foreign_keys_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CanonicalSchemaIndexColumn {
    pub(crate) seqno: i64,
    pub(crate) cid: i64,
    pub(crate) name: Option<String>,
    pub(crate) desc: i64,
    pub(crate) coll: Option<String>,
    pub(crate) key: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CanonicalSchemaIndex {
    pub(crate) name: String,
    pub(crate) columns: Vec<CanonicalSchemaIndexColumn>,
    pub(crate) columns_error: Option<String>,
}

/// Full dump of the observable schema: every `sqlite_master` object (minus
/// SQLite-internal `sqlite_%` names) plus the column, foreign-key and index
/// details needed to distinguish two databases. `sqlite_master.sql` is kept
/// verbatim; `_migrations` contributes names only, so two runs on different
/// days produce the same JSON.
pub(crate) fn canonical_schema_dump(conn: &Connection) -> rusqlite::Result<CanonicalSchemaDump> {
    let objects = read_objects(conn)?;

    let mut tables = Vec::new();
    let mut indexes = Vec::new();
    for object in &objects {
        match object.object_type.as_str() {
            "table" => tables.push(read_table(conn, &object.name)?),
            "index" => indexes.push(read_index(conn, &object.name)?),
            _ => {}
        }
    }

    Ok(CanonicalSchemaDump {
        objects,
        tables,
        indexes,
        migrations: read_migrations(conn)?,
    })
}

fn read_objects(conn: &Connection) -> rusqlite::Result<Vec<CanonicalSchemaObject>> {
    let mut statement = conn.prepare(
        "SELECT type, name, tbl_name, sql FROM sqlite_master
         WHERE name NOT LIKE 'sqlite_%'
         ORDER BY type, name",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(CanonicalSchemaObject {
            object_type: row.get(0)?,
            name: row.get(1)?,
            tbl_name: row.get(2)?,
            sql: row.get(3)?,
        })
    })?;
    rows.collect()
}

fn read_table(conn: &Connection, name: &str) -> rusqlite::Result<CanonicalSchemaTable> {
    let quoted = quote_identifier(name);
    let (columns, columns_error) = match read_columns(conn, &quoted) {
        Ok(columns) => (columns, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    let (foreign_keys, foreign_keys_error) = match read_foreign_keys(conn, &quoted) {
        Ok(foreign_keys) => (foreign_keys, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };

    Ok(CanonicalSchemaTable {
        name: name.to_owned(),
        columns,
        foreign_keys,
        columns_error,
        foreign_keys_error,
    })
}

fn read_index(conn: &Connection, name: &str) -> rusqlite::Result<CanonicalSchemaIndex> {
    let quoted = quote_identifier(name);
    let (columns, columns_error) = match read_index_columns(conn, &quoted) {
        Ok(columns) => (columns, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };

    Ok(CanonicalSchemaIndex {
        name: name.to_owned(),
        columns,
        columns_error,
    })
}

fn read_columns(conn: &Connection, quoted: &str) -> rusqlite::Result<Vec<CanonicalSchemaColumn>> {
    let mut statement = conn.prepare(&format!("PRAGMA table_xinfo({quoted})"))?;
    let rows = statement.query_map([], |row| {
        Ok(CanonicalSchemaColumn {
            cid: row.get(0)?,
            name: row.get(1)?,
            column_type: row.get(2)?,
            notnull: row.get(3)?,
            dflt_value: row.get(4)?,
            pk: row.get(5)?,
            hidden: row.get(6)?,
        })
    })?;
    rows.collect()
}

fn read_foreign_keys(
    conn: &Connection,
    quoted: &str,
) -> rusqlite::Result<Vec<CanonicalSchemaForeignKey>> {
    let mut statement = conn.prepare(&format!("PRAGMA foreign_key_list({quoted})"))?;
    let rows = statement.query_map([], |row| {
        Ok(CanonicalSchemaForeignKey {
            id: row.get(0)?,
            seq: row.get(1)?,
            table: row.get(2)?,
            from: row.get(3)?,
            to: row.get(4)?,
            on_update: row.get(5)?,
            on_delete: row.get(6)?,
            match_kind: row.get(7)?,
        })
    })?;
    let mut foreign_keys: Vec<CanonicalSchemaForeignKey> = rows.collect::<rusqlite::Result<_>>()?;
    foreign_keys.sort_by_key(|foreign_key| (foreign_key.id, foreign_key.seq));
    Ok(foreign_keys)
}

fn read_index_columns(
    conn: &Connection,
    quoted: &str,
) -> rusqlite::Result<Vec<CanonicalSchemaIndexColumn>> {
    let mut statement = conn.prepare(&format!("PRAGMA index_xinfo({quoted})"))?;
    let rows = statement.query_map([], |row| {
        Ok(CanonicalSchemaIndexColumn {
            seqno: row.get(0)?,
            cid: row.get(1)?,
            name: row.get(2)?,
            desc: row.get(3)?,
            coll: row.get(4)?,
            key: row.get(5)?,
        })
    })?;
    rows.collect()
}

fn read_migrations(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut statement = conn.prepare("SELECT name FROM _migrations ORDER BY name")?;
    let rows = statement.query_map([], |row| row.get(0))?;
    rows.collect()
}

/// Doubles embedded quotes, unlike `db::util::quote_identifier` which only adds
/// the wrapping quotes to an already-validated identifier; schema object names
/// come from `sqlite_master` and are not pre-validated.
fn quote_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    const CANONICAL_FIXTURE: &str = include_str!("../../tests/fixtures/schema_canonical.json");
    const SCHEMA_FIXTURE: &str = include_str!("../../tests/fixtures/schema_full.sql");

    /// The TS runner creates `_migrations` inline before applying 0001, and
    /// 0020 creates `layouts` from `applyLayoutsMigration()` rather than from
    /// its `.sql` file, whose text `schema_full.sql` carries. Those two
    /// statements are copied verbatim from `packages/store/src/runner.ts` so a
    /// real TS-migrated database's `sqlite_master.sql` text is reproduced; the
    /// `CREATE TABLE IF NOT EXISTS` in the migration files then becomes the
    /// no-op it is at runtime. Everything else still comes from
    /// `schema_full.sql`.
    const RUNTIME_BOOTSTRAP: &str = r#"CREATE TABLE IF NOT EXISTS _migrations (
      id    INTEGER PRIMARY KEY AUTOINCREMENT,
      name  TEXT    NOT NULL UNIQUE,
      applied_at INTEGER NOT NULL
    );
CREATE TABLE IF NOT EXISTS layouts (
      id TEXT PRIMARY KEY,
      asset_id TEXT NOT NULL REFERENCES assets(id) ON DELETE CASCADE,
      regions TEXT NOT NULL,
      blocks TEXT NOT NULL DEFAULT '[]',
      model TEXT NOT NULL,
      image_width INTEGER NOT NULL,
      image_height INTEGER NOT NULL,
      created_at INTEGER NOT NULL
    );
"#;

    /// Reports the first differences instead of dumping two 300 KB JSON
    /// blobs; arrays are deterministically ordered on both sides, so positions
    /// align.
    fn collect_diffs(path: &str, expected: &Value, actual: &Value, out: &mut Vec<String>) {
        const MAX_DIFFS: usize = 40;
        if out.len() >= MAX_DIFFS {
            return;
        }
        match (expected, actual) {
            (Value::Object(expected_map), Value::Object(actual_map)) => {
                for (key, expected_value) in expected_map {
                    match actual_map.get(key) {
                        Some(actual_value) => collect_diffs(
                            &format!("{path}.{key}"),
                            expected_value,
                            actual_value,
                            out,
                        ),
                        None => out.push(format!("{path}.{key}: missing from the Rust dump")),
                    }
                }
                for key in actual_map.keys() {
                    if !expected_map.contains_key(key) {
                        out.push(format!("{path}.{key}: unexpected in the Rust dump"));
                    }
                }
            }
            (Value::Array(expected_items), Value::Array(actual_items)) => {
                if expected_items.len() != actual_items.len() {
                    out.push(format!(
                        "{path}: {} entries in the fixture vs {} in the Rust dump",
                        expected_items.len(),
                        actual_items.len()
                    ));
                }
                for (index, (expected_item, actual_item)) in
                    expected_items.iter().zip(actual_items).enumerate()
                {
                    collect_diffs(&format!("{path}[{index}]"), expected_item, actual_item, out);
                }
            }
            _ if expected != actual => {
                out.push(format!("{path}: fixture={expected} Rust={actual}"));
            }
            _ => {}
        }
    }

    #[test]
    fn fixture_deserializes_and_round_trips() {
        let parsed: CanonicalSchemaDump =
            serde_json::from_str(CANONICAL_FIXTURE).expect("parse the canonical fixture");
        let original: Value =
            serde_json::from_str(CANONICAL_FIXTURE).expect("parse the canonical fixture as JSON");
        let reserialized: Value = serde_json::to_value(&parsed).expect("serialize the parsed dump");
        assert_eq!(
            original, reserialized,
            "re-serializing the parsed fixture changed its structure"
        );
        assert_eq!(
            format!("{}\n", serde_json::to_string_pretty(&parsed).unwrap()),
            CANONICAL_FIXTURE,
            "pretty round-trip is not byte-identical to the fixture"
        );
    }

    #[test]
    fn rust_dump_matches_the_ts_fixture() {
        let expected: CanonicalSchemaDump =
            serde_json::from_str(CANONICAL_FIXTURE).expect("parse the canonical fixture");

        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(RUNTIME_BOOTSTRAP)
            .expect("apply the runtime bootstrap statements");
        conn.execute_batch(SCHEMA_FIXTURE)
            .expect("apply schema_full.sql");
        let sqlite_version: String = conn
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))
            .expect("read sqlite_version()");
        println!("canonical_dump cross-check: rusqlite bundled SQLite {sqlite_version}");

        let actual = canonical_schema_dump(&conn).expect("dump the Rust schema");

        let mut expected_value = serde_json::to_value(&expected).unwrap();
        let mut actual_value = serde_json::to_value(&actual).unwrap();
        // `schema_full.sql` carries the `_migrations` table but no rows.
        expected_value.as_object_mut().unwrap().remove("migrations");
        actual_value.as_object_mut().unwrap().remove("migrations");

        let mut diffs = Vec::new();
        collect_diffs("$", &expected_value, &actual_value, &mut diffs);
        assert!(
            diffs.is_empty(),
            "Rust canonical dump differs from the TS fixture:\n{}",
            diffs.join("\n")
        );
    }

    #[test]
    fn dump_is_deterministic_and_excludes_sqlite_internal_names() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(SCHEMA_FIXTURE)
            .expect("apply schema_full.sql");

        let first = canonical_schema_dump(&conn).expect("first dump");
        let second = canonical_schema_dump(&conn).expect("second dump");
        assert_eq!(
            first, second,
            "two dumps of the same connection must be equal"
        );

        let internal: Vec<&str> = first
            .objects
            .iter()
            .map(|object| object.name.as_str())
            .filter(|name| name.starts_with("sqlite_"))
            .collect();
        assert!(internal.is_empty(), "sqlite_% objects leaked: {internal:?}");
    }
}
