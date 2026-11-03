//! Domain services for the corpus: collections and items (stage 3 of the
//! agents plan, `odd/tasks/agent-corpus-tools.md` D6).
//!
//! Until now these writes were SQL built in TypeScript and sent through the
//! generic `db_execute*` commands. They live here instead, over a caller-owned
//! connection, so the app's views and the agent endpoint share one set of
//! rules: validation, revision fencing, the reserved metadata keys and the
//! search index refresh. Writes go through the tables' sync triggers like any
//! other write, so they reach the other devices unchanged (D7).

pub mod commands;
pub mod delete;
pub mod ingest;

pub use commands::{
    create_collection, import_path, reindex_item, replace_page_text, update_collection,
    update_item, CollectionDto, CollectionPatch, CorpusError, ImportedPath, ItemDto, ItemPatch,
    ReplacedText, MAX_PAGE_TEXT_CHARS,
};
pub use delete::{
    delete_collection, delete_collection_if_unchanged, delete_fence, delete_impact, delete_items,
    delete_items_if_unchanged, normalized_item_ids, DeleteImpact, DeleteScope,
};
pub use ingest::ImportOverrides;

#[cfg(test)]
mod delete_tests;
#[cfg(test)]
mod ingest_tests;
#[cfg(test)]
mod tests;
