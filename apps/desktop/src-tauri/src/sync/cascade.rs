//! Static `ON DELETE CASCADE` graph for the 18 synced tables (DESIGN §4.4), plus
//! the RESTRICT (plain `REFERENCES`, no cascade) edges among them.
//!
//! When a remote tombstone (delete) is applied, the apply path must FIRST check
//! that no cascade-reachable child of the row is locally dirty: deleting the
//! parent fires `ON DELETE CASCADE` on its children, which would destroy a local
//! edit that has not yet been pushed. If any reachable child is dirty, the whole
//! tombstone is deferred (skip-if-dirty, PROTOCOL "Semántica de apply" §6).
//!
//! The graph is derived from the real schema (see
//! `tests/fixtures/schema_full.sql`). Only `ON DELETE CASCADE` edges matter for
//! [`direct_cascade_edges`] — a plain `REFERENCES` (RESTRICT) edge cannot
//! destroy a child on parent delete, so `items → assets`, `items → notes` and
//! `collections → items` are NOT cascade edges. Those three RESTRICT edges are
//! tracked separately by [`direct_restrict_edges`]: a remote tombstone of the
//! parent does NOT cascade to them, so deleting the parent while they still
//! exist locally violates the FK. A locally dirty RESTRICT dependent defers
//! the whole tombstone; otherwise the dependent is deleted alongside the parent
//! and journaled `parent_deleted`, same as a pulled child whose parent is
//! already tombstoned.

/// The direct `ON DELETE CASCADE` children of each synced table. Tables not
/// listed here have no synced cascade children. Derived from the schema:
///
/// - `items`   → `entities`, `triples`, `item_topics`
/// - `assets`  → `extractions`, `transcriptions`, `layouts`, `annotations`, and
///   `assets` itself (`parent_asset_id`: a PDF container's page assets)
/// - `topics`  → `item_topics`
/// - `rag_conversations` → `rag_messages`
/// - `web_sources` → `web_captures`
///
/// `notes` references `items` and `assets` references `items` but WITHOUT
/// cascade (RESTRICT), so they are not children here.
fn cascade_children(table: &str) -> &'static [&'static str] {
    match table {
        "items" => &["entities", "triples", "item_topics"],
        "assets" => &[
            "extractions",
            "transcriptions",
            "layouts",
            "annotations",
            "assets",
        ],
        "topics" => &["item_topics"],
        "rag_conversations" => &["rag_messages"],
        "web_sources" => &["web_captures"],
        _ => &[],
    }
}

/// The foreign-key column on `child` that points back at `parent` along a
/// cascade edge. Returns `None` when `child` is not a cascade child of
/// `parent`. Used to enumerate the child rows reachable from a parent row.
fn cascade_fk_column(parent: &str, child: &str) -> Option<&'static str> {
    match (parent, child) {
        ("items", "entities") => Some("item_id"),
        ("items", "triples") => Some("item_id"),
        ("items", "item_topics") => Some("item_id"),
        ("assets", "extractions") => Some("asset_id"),
        ("assets", "transcriptions") => Some("asset_id"),
        ("assets", "layouts") => Some("asset_id"),
        ("assets", "annotations") => Some("asset_id"),
        ("assets", "assets") => Some("parent_asset_id"),
        ("topics", "item_topics") => Some("topic_id"),
        ("rag_conversations", "rag_messages") => Some("conversation_id"),
        ("web_sources", "web_captures") => Some("web_source_id"),
        _ => None,
    }
}

/// Every `(child_table, fk_column)` reachable from a delete of `(parent, row_id)`
/// via one cascade hop, plus the transitive closure (a child that is itself a
/// cascade parent expands further). Returns the list of
/// `(child_table, fk_column, parent_table)` edges so the caller can build the
/// `WHERE {fk_column} = {parent_row_id}` lookups. The parent row id is the same
/// for every direct child of a given parent, but transitive children need a
/// recursive walk — for the synced schema the depth is at most 2, so this
/// returns the direct edges and the caller recurses through resolved child ids.
pub fn direct_cascade_edges(parent: &str) -> Vec<(&'static str, &'static str)> {
    cascade_children(parent)
        .iter()
        .filter_map(|child| cascade_fk_column(parent, child).map(|col| (*child, col)))
        .collect()
}

/// The direct RESTRICT (plain `REFERENCES`, no `ON DELETE CASCADE`) children of
/// each synced table. Tables not listed here have no synced RESTRICT children.
/// Derived from the schema:
///
/// - `collections` → `items` (`collection_id`)
/// - `items`        → `assets` (`item_id`), `notes` (`item_id`)
fn restrict_children(table: &str) -> &'static [&'static str] {
    match table {
        "collections" => &["items"],
        "items" => &["assets", "notes"],
        _ => &[],
    }
}

/// The foreign-key column on `child` that points back at `parent` along a
/// RESTRICT edge. Returns `None` when `child` is not a RESTRICT child of
/// `parent`.
fn restrict_fk_column(parent: &str, child: &str) -> Option<&'static str> {
    match (parent, child) {
        ("collections", "items") => Some("collection_id"),
        ("items", "assets") => Some("item_id"),
        ("items", "notes") => Some("item_id"),
        _ => None,
    }
}

/// Every `(child_table, fk_column)` RESTRICT edge reachable from a delete of
/// `parent` via one hop. Unlike [`direct_cascade_edges`], deleting `parent`
/// does NOT propagate to these rows in SQLite — the apply path must delete them
/// explicitly (depth-first, dependents' own dependents first) before the parent
/// itself, or defer the whole tombstone if any of them is locally dirty.
pub fn direct_restrict_edges(parent: &str) -> Vec<(&'static str, &'static str)> {
    restrict_children(parent)
        .iter()
        .filter_map(|child| restrict_fk_column(parent, child).map(|col| (*child, col)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_cascade_children_match_schema() {
        let edges = direct_cascade_edges("items");
        let tables: Vec<&str> = edges.iter().map(|(t, _)| *t).collect();
        assert!(tables.contains(&"entities"));
        assert!(tables.contains(&"triples"));
        assert!(tables.contains(&"item_topics"));
        // assets / notes are RESTRICT, never cascade children of items.
        assert!(!tables.contains(&"assets"));
        assert!(!tables.contains(&"notes"));
    }

    #[test]
    fn assets_cascade_children_match_schema() {
        let edges = direct_cascade_edges("assets");
        let tables: Vec<&str> = edges.iter().map(|(t, _)| *t).collect();
        for child in ["extractions", "transcriptions", "layouts", "annotations"] {
            assert!(tables.contains(&child), "missing cascade child {child}");
        }
        // Every edge on assets uses asset_id, except the page self-edge.
        assert!(edges.iter().all(|(t, col)| *col
            == if *t == "assets" {
                "parent_asset_id"
            } else {
                "asset_id"
            }));
    }

    #[test]
    fn assets_cascade_to_their_page_assets() {
        assert!(direct_cascade_edges("assets").contains(&("assets", "parent_asset_id")));
    }

    #[test]
    fn topics_and_conversations_cascade_to_junctions() {
        assert_eq!(
            direct_cascade_edges("topics"),
            vec![("item_topics", "topic_id")]
        );
        assert_eq!(
            direct_cascade_edges("rag_conversations"),
            vec![("rag_messages", "conversation_id")]
        );
    }

    #[test]
    fn web_sources_cascade_to_their_captures() {
        assert_eq!(
            direct_cascade_edges("web_sources"),
            vec![("web_captures", "web_source_id")]
        );
        assert!(direct_cascade_edges("web_captures").is_empty());
        assert!(direct_restrict_edges("web_sources").is_empty());
    }

    /// Mirror of the restrict guard for the CASCADE edges: every
    /// `ON DELETE CASCADE` FK between synced tables in the real schema must be
    /// in [`direct_cascade_edges`], and nothing else. A new cascade FK that is
    /// not mapped would let a remote tombstone destroy a dirty child unnoticed.
    #[test]
    fn cascade_edge_map_matches_schema_fixture() {
        use crate::sync::capture::SYNCED_TABLES;
        use crate::sync::test_support::new_app_schema_db;
        use std::collections::BTreeSet;

        let conn = new_app_schema_db();

        let mut from_schema: BTreeSet<(String, String, String)> = BTreeSet::new();
        for &child in SYNCED_TABLES {
            let sql = format!("PRAGMA foreign_key_list({child})");
            let mut stmt = conn.prepare(&sql).expect("prepare foreign_key_list");
            let rows = stmt
                .query_map([], |row| {
                    let parent: String = row.get(2)?;
                    let from: String = row.get(3)?;
                    let on_delete: String = row.get(6)?;
                    Ok((parent, from, on_delete))
                })
                .expect("query foreign_key_list");
            for row in rows {
                let (parent, from_col, on_delete) = row.expect("read fk row");
                if SYNCED_TABLES.contains(&parent.as_str()) && on_delete.to_uppercase() == "CASCADE"
                {
                    from_schema.insert((parent, child.to_string(), from_col));
                }
            }
        }

        let mut from_map: BTreeSet<(String, String, String)> = BTreeSet::new();
        for &parent in SYNCED_TABLES {
            for (child, col) in direct_cascade_edges(parent) {
                from_map.insert((parent.to_string(), child.to_string(), col.to_string()));
            }
        }

        assert_eq!(
            from_map, from_schema,
            "direct_cascade_edges must equal every CASCADE FK edge between synced tables"
        );
    }

    #[test]
    fn leaf_tables_have_no_cascade_children() {
        for table in [
            "notes",
            "entities",
            "extractions",
            "item_topics",
            "collections",
        ] {
            assert!(
                direct_cascade_edges(table).is_empty(),
                "{table} should have no cascade children"
            );
        }
    }

    #[test]
    fn collections_restrict_child_is_items() {
        assert_eq!(
            direct_restrict_edges("collections"),
            vec![("items", "collection_id")]
        );
    }

    #[test]
    fn items_restrict_children_are_assets_and_notes() {
        let edges = direct_restrict_edges("items");
        let tables: Vec<&str> = edges.iter().map(|(t, _)| *t).collect();
        assert!(tables.contains(&"assets"));
        assert!(tables.contains(&"notes"));
        assert!(edges.iter().all(|(_, col)| *col == "item_id"));
        // These are RESTRICT, never cascade children of items.
        let cascade_tables: Vec<&str> = direct_cascade_edges("items")
            .iter()
            .map(|(t, _)| *t)
            .collect();
        assert!(!cascade_tables.contains(&"assets"));
        assert!(!cascade_tables.contains(&"notes"));
    }

    #[test]
    fn leaf_tables_have_no_restrict_children() {
        for table in ["assets", "notes", "topics", "rag_conversations"] {
            assert!(
                direct_restrict_edges(table).is_empty(),
                "{table} should have no restrict children"
            );
        }
    }

    /// Guards against drift between the hand-maintained restrict-edge map and
    /// the real schema fixture: builds the actual application schema and reads
    /// every FK edge between synced tables via `PRAGMA foreign_key_list`, then
    /// asserts the set of non-`CASCADE` edges is EXACTLY the set encoded by
    /// [`direct_restrict_edges`].
    #[test]
    fn restrict_edge_map_matches_schema_fixture() {
        use crate::sync::capture::SYNCED_TABLES;
        use crate::sync::test_support::new_app_schema_db;
        use std::collections::BTreeSet;

        let conn = new_app_schema_db();

        let mut from_schema: BTreeSet<(String, String, String)> = BTreeSet::new();
        for &child in SYNCED_TABLES {
            let sql = format!("PRAGMA foreign_key_list({child})");
            let mut stmt = conn.prepare(&sql).expect("prepare foreign_key_list");
            let rows = stmt
                .query_map([], |row| {
                    let parent: String = row.get(2)?; // "table" (referenced table)
                    let from: String = row.get(3)?; // "from" (fk column on child)
                    let on_delete: String = row.get(6)?; // "on_delete" action
                    Ok((parent, from, on_delete))
                })
                .expect("query foreign_key_list");
            for row in rows {
                let (parent, from_col, on_delete) = row.expect("read fk row");
                if SYNCED_TABLES.contains(&parent.as_str()) && on_delete.to_uppercase() != "CASCADE"
                {
                    from_schema.insert((parent, child.to_string(), from_col));
                }
            }
        }

        let mut from_map: BTreeSet<(String, String, String)> = BTreeSet::new();
        for &parent in SYNCED_TABLES {
            for (child, col) in direct_restrict_edges(parent) {
                from_map.insert((parent.to_string(), child.to_string(), col.to_string()));
            }
        }

        assert_eq!(
            from_map, from_schema,
            "direct_restrict_edges must equal every non-CASCADE FK edge between synced tables"
        );
    }
}
