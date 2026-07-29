import { DatabaseSync } from 'node:sqlite'
import { afterEach, describe, expect, it } from 'vitest'
import { COLLECTION_ACTIVITY_DDL } from './runner'

function createDatabase(): DatabaseSync {
  const db = new DatabaseSync(':memory:')
  db.exec(`
    CREATE TABLE collections (id TEXT PRIMARY KEY, updated_at INTEGER NOT NULL);
    CREATE TABLE items (id TEXT PRIMARY KEY, collection_id TEXT NOT NULL, updated_at INTEGER NOT NULL);
    CREATE TABLE assets (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, path TEXT NOT NULL);
    CREATE TABLE notes (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, asset_id TEXT, content TEXT);
    CREATE TABLE extractions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, text_content TEXT);
    CREATE TABLE layouts (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, blocks TEXT);
    CREATE TABLE transcriptions (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, text_content TEXT);
    CREATE TABLE annotations (id TEXT PRIMARY KEY, asset_id TEXT NOT NULL, kind TEXT);
    CREATE TABLE entities (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, asset_id TEXT, value TEXT);
    CREATE TABLE triples (id TEXT PRIMARY KEY, item_id TEXT NOT NULL, asset_id TEXT, object TEXT);
    CREATE TABLE vec_assets (asset_id TEXT PRIMARY KEY, item_id TEXT NOT NULL, embedding BLOB);
    CREATE TABLE llm_results (
      id TEXT PRIMARY KEY,
      target_id TEXT NOT NULL,
      target_type TEXT NOT NULL,
      result TEXT
    );

    ${COLLECTION_ACTIVITY_DDL}

    INSERT INTO collections (id, updated_at) VALUES ('collection-1', 1);
    INSERT INTO items (id, collection_id, updated_at) VALUES ('item-1', 'collection-1', 1);
    INSERT INTO assets (id, item_id, path) VALUES ('asset-1', 'item-1', 'source.pdf');
    UPDATE collections SET updated_at = 1 WHERE id = 'collection-1';
  `)
  return db
}

function activity(db: DatabaseSync): number {
  return db.prepare('SELECT updated_at FROM collections WHERE id = ?').get('collection-1')!
    .updated_at as number
}

function resetActivity(db: DatabaseSync): void {
  db.prepare('UPDATE collections SET updated_at = 1 WHERE id = ?').run('collection-1')
}

function expectActivity(db: DatabaseSync, sql: string): void {
  resetActivity(db)
  db.exec(sql)
  expect(activity(db)).toBeGreaterThan(1)
}

describe('collection activity triggers', () => {
  let db: DatabaseSync | undefined

  afterEach(() => {
    db?.close()
    db = undefined
  })

  it('tracks asset creation, edits, replacement and deletion', () => {
    db = createDatabase()

    expectActivity(
      db,
      "INSERT INTO assets (id, item_id, path) VALUES ('asset-2', 'item-1', 'imported.pdf')"
    )
    expectActivity(db, "UPDATE assets SET path = 'replacement.pdf' WHERE id = 'asset-2'")
    expectActivity(db, "DELETE FROM assets WHERE id = 'asset-2'")
  })

  it('tracks content, metadata, processing results and status linked to assets', () => {
    db = createDatabase()
    const mutations = [
      {
        insert:
          "INSERT INTO notes (id, item_id, asset_id, content) VALUES ('note-1', 'item-1', 'asset-1', 'note')",
        update: "UPDATE notes SET content = 'edited' WHERE id = 'note-1'",
        delete: "DELETE FROM notes WHERE id = 'note-1'",
      },
      {
        insert:
          "INSERT INTO extractions (id, asset_id, text_content) VALUES ('extraction-1', 'asset-1', 'ocr')",
        update: "UPDATE extractions SET text_content = 'edited ocr' WHERE id = 'extraction-1'",
        delete: "DELETE FROM extractions WHERE id = 'extraction-1'",
      },
      {
        insert: "INSERT INTO layouts (id, asset_id, blocks) VALUES ('layout-1', 'asset-1', '[]')",
        update: "UPDATE layouts SET blocks = '[1]' WHERE id = 'layout-1'",
        delete: "DELETE FROM layouts WHERE id = 'layout-1'",
      },
      {
        insert:
          "INSERT INTO transcriptions (id, asset_id, text_content) VALUES ('transcription-1', 'asset-1', 'audio')",
        update:
          "UPDATE transcriptions SET text_content = 'edited audio' WHERE id = 'transcription-1'",
        delete: "DELETE FROM transcriptions WHERE id = 'transcription-1'",
      },
      {
        insert:
          "INSERT INTO annotations (id, asset_id, kind) VALUES ('annotation-1', 'asset-1', 'crop')",
        update: "UPDATE annotations SET kind = 'rotation' WHERE id = 'annotation-1'",
        delete: "DELETE FROM annotations WHERE id = 'annotation-1'",
      },
      {
        insert:
          "INSERT INTO entities (id, item_id, asset_id, value) VALUES ('entity-1', 'item-1', 'asset-1', 'person')",
        update: "UPDATE entities SET value = 'place' WHERE id = 'entity-1'",
        delete: "DELETE FROM entities WHERE id = 'entity-1'",
      },
      {
        insert:
          "INSERT INTO triples (id, item_id, asset_id, object) VALUES ('triple-1', 'item-1', 'asset-1', 'object')",
        update: "UPDATE triples SET object = 'edited' WHERE id = 'triple-1'",
        delete: "DELETE FROM triples WHERE id = 'triple-1'",
      },
      {
        insert:
          "INSERT INTO vec_assets (asset_id, item_id, embedding) VALUES ('asset-1', 'item-1', X'01')",
        update: "UPDATE vec_assets SET embedding = X'02' WHERE asset_id = 'asset-1'",
        delete: "DELETE FROM vec_assets WHERE asset_id = 'asset-1'",
      },
      {
        insert:
          "INSERT INTO llm_results (id, target_id, target_type, result) VALUES ('llm-1', 'asset-1', 'asset', 'analysis')",
        update: "UPDATE llm_results SET result = 'edited analysis' WHERE id = 'llm-1'",
        delete: "DELETE FROM llm_results WHERE id = 'llm-1'",
      },
    ]

    for (const mutation of mutations) {
      expectActivity(db, mutation.insert)
      expectActivity(db, mutation.update)
      expectActivity(db, mutation.delete)
    }
  })
})
