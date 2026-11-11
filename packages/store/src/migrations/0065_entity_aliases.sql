-- 0065_entity_aliases: the owner's global alias list for entity surface forms.
--
-- alias_key: the normalized surface form (trimmed, whitespace collapsed,
--            lowercased — the same normalization as normalize_entity_value in
--            src-tauri/src/nlp/ner/mod.rs and the relation-graph key in
--            apps/desktop/src/lib/relation-graph.ts). UNIQUE: one canonical
--            per surface form, archive-wide.
-- alias:     the form as the owner typed it (kept verbatim for display).
-- canonical: the name the alias resolves to at read time (T2). Resolution is
--            single-hop: nothing rewrites entities rows (S2), so changing an
--            alias is one UPDATE here.
--
-- Global on purpose (S1): no collection column. The alias list is curated by
-- the owner (S6): upserts that would silently merge (empty values, alias equal
-- to canonical, chains) are rejected by EntityAliasRepo, not by the schema.
--
-- Additive only: one new table, no change to any existing table. Sync does
-- NOT carry this table yet (T4): the desktop must not sync it until
-- EntropIA-Cloud accepts it, so the first push from a device with this
-- migration raises the account schema_tag. Devices that have not updated drop
-- the unknown table on apply and journal the drift.
--
-- Runs through the single-batch path with its _migrations row (same as 0062,
-- 0063 and 0064), so a second runMigrations() pass skips it. Do not apply
-- this file twice by hand.

CREATE TABLE IF NOT EXISTS entity_aliases (
  id TEXT PRIMARY KEY,
  alias_key TEXT NOT NULL UNIQUE,
  alias TEXT NOT NULL,
  canonical TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
