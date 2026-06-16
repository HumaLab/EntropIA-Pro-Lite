//! Static `ON DELETE CASCADE` graph for the 15 synced tables (DESIGN §4.4).
//!
//! When a remote tombstone (delete) is applied, the apply path must FIRST check
//! that no cascade-reachable child of the row is locally dirty: deleting the
//! parent fires `ON DELETE CASCADE` on its children, which would destroy a local
//! edit that has not yet been pushed. If any reachable child is dirty, the whole
//! tombstone is deferred (skip-if-dirty, PROTOCOL "Semántica de apply" §6).
//!
//! The graph is derived from the real schema (see
//! `tests/fixtures/schema_full.sql`). Only `ON DELETE CASCADE` edges matter — a
//! plain `REFERENCES` (RESTRICT) edge cannot destroy a child on parent delete,
//! so `items → assets`, `items → notes` and `collections → items` are NOT
//! cascade edges and are intentionally absent.

/// The direct `ON DELETE CASCADE` children of each synced table. Tables not
/// listed here have no synced cascade children. Derived from the schema:
///
/// - `items`   → `entities`, `triples`, `item_topics`
/// - `assets`  → `extractions`, `transcriptions`, `layouts`, `annotations`
/// - `topics`  → `item_topics`
/// - `rag_conversations` → `rag_messages`
///
/// `notes` references `items` and `assets` references `items` but WITHOUT
/// cascade (RESTRICT), so they are not children here.
fn cascade_children(table: &str) -> &'static [&'static str] {
    match table {
        "items" => &["entities", "triples", "item_topics"],
        "assets" => &["extractions", "transcriptions", "layouts", "annotations"],
        "topics" => &["item_topics"],
        "rag_conversations" => &["rag_messages"],
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
        ("topics", "item_topics") => Some("topic_id"),
        ("rag_conversations", "rag_messages") => Some("conversation_id"),
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
        // Every edge on assets uses asset_id.
        assert!(edges.iter().all(|(_, col)| *col == "asset_id"));
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
}
