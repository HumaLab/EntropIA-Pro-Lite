import { sqliteTable, text, integer, real, index, foreignKey } from 'drizzle-orm/sqlite-core'

// ---------------------------------------------------------------------------
// Collections — top-level grouping of items
// ---------------------------------------------------------------------------
export const collections = sqliteTable('collections', {
  id: text('id').primaryKey(),
  name: text('name').notNull(),
  description: text('description'),
  createdAt: integer('created_at').notNull(),
  updatedAt: integer('updated_at').notNull(),
})

// ---------------------------------------------------------------------------
// Items — documents / artifacts within a collection
// ---------------------------------------------------------------------------
export const items = sqliteTable('items', {
  id: text('id').primaryKey(),
  title: text('title').notNull(),
  collectionId: text('collection_id')
    .notNull()
    .references(() => collections.id),
  metadata: text('metadata'), // JSON blob
  createdAt: integer('created_at').notNull(),
  updatedAt: integer('updated_at').notNull(),
})

// ---------------------------------------------------------------------------
// Assets — files (images, PDFs) attached to an item
// ---------------------------------------------------------------------------
export const assets = sqliteTable(
  'assets',
  {
    id: text('id').primaryKey(),
    itemId: text('item_id')
      .notNull()
      .references(() => items.id),
    path: text('path').notNull(),
    type: text('type').notNull(), // 'image' | 'pdf' | 'audio'
    sortIndex: integer('sort_index').notNull().default(0),
    size: integer('size'),
    parentAssetId: text('parent_asset_id'),
    pageNumber: integer('page_number'),
    createdAt: integer('created_at').notNull(),
  },
  (table) => ({
    parentAssetFk: foreignKey({
      columns: [table.parentAssetId],
      foreignColumns: [table.id],
      name: 'assets_parent_asset_id_fkey',
    }).onDelete('cascade'),
    parentAssetIdx: index('idx_assets_parent_asset_id').on(table.parentAssetId),
    parentPageIdx: index('idx_assets_parent_page').on(table.parentAssetId, table.pageNumber),
  })
)

// ---------------------------------------------------------------------------
// Notes — textual annotations on an item (optionally scoped to an asset/page)
// ---------------------------------------------------------------------------
export const notes = sqliteTable('notes', {
  id: text('id').primaryKey(),
  itemId: text('item_id')
    .notNull()
    .references(() => items.id),
  assetId: text('asset_id'),
  content: text('content').notNull(),
  createdAt: integer('created_at').notNull(),
  updatedAt: integer('updated_at').notNull(),
})

// ---------------------------------------------------------------------------
// Extractions — OCR / native text extraction results for an asset
// ---------------------------------------------------------------------------
export const extractions = sqliteTable('extractions', {
  id: text('id').primaryKey(),
  assetId: text('asset_id')
    .notNull()
    .references(() => assets.id),
  textContent: text('text_content').notNull(),
  method: text('method').notNull(), // 'native' | 'ocr'
  confidence: real('confidence'),
  createdAt: integer('created_at').notNull(),
})

// ---------------------------------------------------------------------------
// Layouts — persisted OCRH/PaddleVL structure results for an asset
// ---------------------------------------------------------------------------
export const layouts = sqliteTable('layouts', {
  id: text('id').primaryKey(),
  assetId: text('asset_id')
    .notNull()
    .references(() => assets.id),
  regions: text('regions').notNull(), // JSON array/object
  blocks: text('blocks').notNull(), // JSON array/object
  model: text('model').notNull(),
  imageWidth: integer('image_width').notNull(),
  imageHeight: integer('image_height').notNull(),
  createdAt: integer('created_at').notNull(),
})

// ---------------------------------------------------------------------------
// Entities — NER results linked to an item (optionally scoped to an asset)
// ---------------------------------------------------------------------------
export const entities = sqliteTable('entities', {
  id: text('id').primaryKey().notNull(),
  itemId: text('item_id')
    .notNull()
    .references(() => items.id),
  assetId: text('asset_id'),
  entityType: text('entity_type').notNull(), // 'person' | 'place' | 'date' | 'institution' | 'organization' | 'misc' | 'custom'
  value: text('value').notNull(),
  startOffset: integer('start_offset').notNull().default(0),
  endOffset: integer('end_offset').notNull().default(0),
  confidence: real('confidence').notNull().default(1.0),
  source: text('source'),
  modelName: text('model_name'),
  latitude: real('latitude'),
  longitude: real('longitude'),
  manualLatitude: real('manual_lat'),
  manualLongitude: real('manual_lon'),
  geoStatus: text('geo_status').notNull().default('pending'),
  createdAt: integer('created_at').notNull(),
})

// ---------------------------------------------------------------------------
// Triples — semantic triples (S|P|O) linked to an item (optionally scoped to an asset)
// ---------------------------------------------------------------------------
export const triples = sqliteTable('triples', {
  id: text('id').primaryKey().notNull(),
  itemId: text('item_id')
    .notNull()
    .references(() => items.id),
  assetId: text('asset_id'),
  subject: text('subject').notNull(),
  predicate: text('predicate').notNull(),
  object: text('object').notNull(),
  createdAt: integer('created_at').notNull(),
})

// ---------------------------------------------------------------------------
// Transcriptions — Whisper-based audio transcription results for an asset
// ---------------------------------------------------------------------------
export const transcriptions = sqliteTable('transcriptions', {
  id: text('id').primaryKey(),
  assetId: text('asset_id')
    .notNull()
    .references(() => assets.id, { onDelete: 'cascade' }),
  textContent: text('text_content').notNull(),
  language: text('language'),
  durationMs: integer('duration_ms'),
  model: text('model').notNull(),
  segments: text('segments'), // JSON array of { start_ms, end_ms, text }
  confidence: real('confidence'),
  createdAt: integer('created_at').notNull(),
})

// ---------------------------------------------------------------------------
// Annotations — visual overlays linked to an asset/page
// ---------------------------------------------------------------------------
export const annotations = sqliteTable(
  'annotations',
  {
    id: text('id').primaryKey().notNull(),
    assetId: text('asset_id')
      .notNull()
      .references(() => assets.id, { onDelete: 'cascade' }),
    page: integer('page').notNull().default(1),
    kind: text('kind').notNull(), // annotations plus non-destructive document view edits
    color: text('color').notNull(),
    x: real('x').notNull(),
    y: real('y').notNull(),
    width: real('width').notNull(),
    height: real('height').notNull(),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => ({
    assetIdIdx: index('annotations_asset_id_idx').on(table.assetId),
    assetPageIdx: index('annotations_asset_page_idx').on(table.assetId, table.page),
  })
)

// ---------------------------------------------------------------------------
// Topics — reusable tags for categorizing items
// ---------------------------------------------------------------------------
export const topics = sqliteTable('topics', {
  id: text('id').primaryKey(),
  name: text('name').notNull().unique(),
  createdAt: integer('created_at').notNull(),
})

// ---------------------------------------------------------------------------
// Item Topics — many-to-many relationship between items and topics
// ---------------------------------------------------------------------------
export const itemTopics = sqliteTable(
  'item_topics',
  {
    id: text('id').primaryKey(),
    itemId: text('item_id')
      .notNull()
      .references(() => items.id, { onDelete: 'cascade' }),
    topicId: text('topic_id')
      .notNull()
      .references(() => topics.id, { onDelete: 'cascade' }),
    createdAt: integer('created_at').notNull(),
  },
  (table) => ({
    itemTopicIdx: index('idx_item_topics_item_topic').on(table.itemId, table.topicId),
    topicIdx: index('idx_item_topics_topic_id').on(table.topicId),
  })
)

// ---------------------------------------------------------------------------
// RAG Conversations — persisted research chat threads
// ---------------------------------------------------------------------------
export const ragConversations = sqliteTable('rag_conversations', {
  id: text('id').primaryKey(),
  title: text('title').notNull(),
  createdAt: integer('created_at').notNull(),
  updatedAt: integer('updated_at').notNull(),
})

// ---------------------------------------------------------------------------
// RAG Messages — ordered turns within a RAG conversation
// ---------------------------------------------------------------------------
export const ragMessages = sqliteTable(
  'rag_messages',
  {
    id: text('id').primaryKey(),
    conversationId: text('conversation_id')
      .notNull()
      .references(() => ragConversations.id, { onDelete: 'cascade' }),
    sortIndex: integer('sort_index').notNull(),
    role: text('role').notNull(), // 'user' | 'assistant'
    content: text('content').notNull(),
    sources: text('sources'), // JSON array of cited sources
    model: text('model'),
    createdAt: integer('created_at').notNull(),
  },
  (table) => ({
    conversationIdx: index('idx_rag_messages_conversation').on(
      table.conversationId,
      table.sortIndex
    ),
  })
)

// ---------------------------------------------------------------------------
// LLM Results — persisted outputs from Gemma/local LLM jobs
// ---------------------------------------------------------------------------
export const llmResults = sqliteTable(
  'llm_results',
  {
    id: text('id').primaryKey(),
    targetId: text('target_id').notNull(),
    jobType: text('job_type').notNull(),
    result: text('result').notNull(),
    createdAt: integer('created_at').notNull(),
  },
  (table) => ({
    targetIdx: index('idx_llm_results_target').on(table.targetId),
  })
)
