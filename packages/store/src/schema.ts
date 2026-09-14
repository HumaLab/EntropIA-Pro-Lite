import { sql } from 'drizzle-orm'
import {
  sqliteTable,
  text,
  integer,
  real,
  blob,
  index,
  uniqueIndex,
  foreignKey,
  primaryKey,
} from 'drizzle-orm/sqlite-core'

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
export const items = sqliteTable(
  'items',
  {
    id: text('id').primaryKey(),
    title: text('title').notNull(),
    collectionId: text('collection_id')
      .notNull()
      .references(() => collections.id),
    metadata: text('metadata'), // JSON blob
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => ({
    // Covers the collection filter, the `title COLLATE NOCASE, id` ordering
    // every card list uses, and the keyset cursor built on that same pair.
    // Mirrors migration 0030; the collation has to be spelled out in SQL
    // because the column itself keeps the default BINARY collation.
    collectionTitleIdx: index('idx_items_collection_title').on(
      table.collectionId,
      sql`${table.title} COLLATE NOCASE`,
      table.id
    ),
  })
)

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
// RAG Chunks — canonical retrieval units derived from asset text sources
// ---------------------------------------------------------------------------
export const ragChunks = sqliteTable(
  'rag_chunks',
  {
    id: text('id').primaryKey(),
    assetId: text('asset_id')
      .notNull()
      .references(() => assets.id, { onDelete: 'cascade' }),
    itemId: text('item_id')
      .notNull()
      .references(() => items.id, { onDelete: 'cascade' }),
    sourceKind: text('source_kind', { enum: ['extraction', 'transcription'] }).notNull(),
    sourceId: text('source_id').notNull(),
    chunkOrdinal: integer('chunk_ordinal').notNull(),
    textContent: text('text_content').notNull(),
    startChar: integer('start_char').notNull(),
    endChar: integer('end_char').notNull(),
    sourceTextHash: text('source_text_hash').notNull(),
    chunkingContract: text('chunking_contract').notNull(),
    embedding: blob('embedding').notNull(),
    embeddingModel: text('embedding_model').notNull(),
    embeddingContract: text('embedding_contract').notNull(),
    dimensions: integer('dimensions').notNull(),
  },
  (table) => ({
    identityUnique: uniqueIndex('idx_rag_chunks_identity_unique').on(
      table.assetId,
      table.sourceKind,
      table.sourceId,
      table.chunkOrdinal
    ),
    assetIdx: index('idx_rag_chunks_asset_id').on(table.assetId),
    itemIdx: index('idx_rag_chunks_item_id').on(table.itemId),
    embeddingContractIdx: index('idx_rag_chunks_embedding_contract').on(
      table.embeddingModel,
      table.embeddingContract,
      table.dimensions
    ),
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
// ---------------------------------------------------------------------------
// Batch processing — durable background queue for OCR + embeddings
// (migration 0032_batch_processing). Historical asset/collection ids are
// snapshots: plain TEXT without CASCADE, so deleting content never destroys
// attempts or history. FKs below only link processing tables to each other.
// ---------------------------------------------------------------------------
export const processingBatches = sqliteTable('processing_batches', {
  id: text('id').primaryKey(),
  requestId: text('request_id').notNull().unique(),
  origin: text('origin').notNull(),
  state: text('state').notNull(),
  desiredState: text('desired_state').notNull(),
  operations: text('operations').notNull(),
  configSnapshotJson: text('config_snapshot_json').notNull().default('{}'),
  planningCursor: integer('planning_cursor').notNull().default(0),
  planningDone: integer('planning_done').notNull().default(0),
  revision: integer('revision').notNull().default(0),
  createdAt: integer('created_at').notNull(),
  updatedAt: integer('updated_at').notNull(),
  startedAt: integer('started_at'),
  finishedAt: integer('finished_at'),
  lastError: text('last_error'),
})
export const processingBatchCollections = sqliteTable(
  'processing_batch_collections',
  {
    batchId: text('batch_id')
      .notNull()
      .references(() => processingBatches.id, { onDelete: 'cascade' }),
    collectionIdSnapshot: text('collection_id_snapshot').notNull(),
    nameSnapshot: text('name_snapshot').notNull(),
  },
  (table) => ({
    pk: primaryKey({ columns: [table.batchId, table.collectionIdSnapshot] }),
  })
)

export const processingBatchMembers = sqliteTable(
  'processing_batch_members',
  {
    batchId: text('batch_id')
      .notNull()
      .references(() => processingBatches.id, { onDelete: 'cascade' }),
    ordinal: integer('ordinal').notNull(),
    assetIdSnapshot: text('asset_id_snapshot').notNull(),
    itemIdSnapshot: text('item_id_snapshot').notNull(),
    collectionIdSnapshot: text('collection_id_snapshot').notNull(),
    titleSnapshot: text('title_snapshot').notNull().default(''),
    classification: text('classification').notNull().default('unclassified'),
    reason: text('reason'),
  },
  (table) => ({
    pk: primaryKey({ columns: [table.batchId, table.ordinal] }),
    batchAssetUnique: uniqueIndex('idx_processing_members_batch_asset_unique').on(
      table.batchId,
      table.assetIdSnapshot
    ),
    batchAssetIdx: index('idx_processing_members_batch_asset').on(
      table.batchId,
      table.assetIdSnapshot
    ),
  })
)
export const processingTasks = sqliteTable(
  'processing_tasks',
  {
    id: text('id').primaryKey(),
    kind: text('kind').notNull(),
    assetIdSnapshot: text('asset_id_snapshot').notNull(),
    inputRevision: integer('input_revision').notNull().default(0),
    inputFingerprint: text('input_fingerprint').notNull().default(''),
    contractHash: text('contract_hash').notNull().default(''),
    state: text('state').notNull(),
    stage: text('stage').notNull().default(''),
    progressDone: integer('progress_done').notNull().default(0),
    progressTotal: integer('progress_total').notNull().default(0),
    outcome: text('outcome').notNull().default(''),
    attemptCount: integer('attempt_count').notNull().default(0),
    retryCycle: integer('retry_cycle').notNull().default(0),
    retryCount: integer('retry_count').notNull().default(0),
    nextRetryAt: integer('next_retry_at'),
    ownerSession: text('owner_session'),
    leaseEpoch: integer('lease_epoch').notNull().default(0),
    heartbeatAt: integer('heartbeat_at'),
    leaseExpiresAt: integer('lease_expires_at'),
    lastErrorCode: text('last_error_code'),
    lastErrorMessage: text('last_error_message'),
    resultReceiptJson: text('result_receipt_json'),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => ({
    claimableIdx: index('idx_processing_tasks_claimable').on(
      table.state,
      table.nextRetryAt,
      table.id
    ),
  })
)

export const processingBatchTasks = sqliteTable(
  'processing_batch_tasks',
  {
    batchId: text('batch_id')
      .notNull()
      .references(() => processingBatches.id, { onDelete: 'cascade' }),
    taskId: text('task_id')
      .notNull()
      .references(() => processingTasks.id),
    kind: text('kind').notNull(),
    assetIdSnapshot: text('asset_id_snapshot').notNull(),
    requestState: text('request_state').notNull().default('active'),
    dependencyTaskId: text('dependency_task_id').references(() => processingTasks.id),
  },
  (table) => ({
    pk: primaryKey({ columns: [table.batchId, table.taskId] }),
    taskIdx: index('idx_processing_batch_tasks_task').on(table.taskId, table.requestState),
    batchIdx: index('idx_processing_batch_tasks_batch').on(table.batchId, table.taskId),
  })
)

export const processingRequests = sqliteTable('processing_requests', {
  requestId: text('request_id').primaryKey(),
  action: text('action').notNull(),
  batchId: text('batch_id').references(() => processingBatches.id, {
    onDelete: 'cascade',
  }),
  payloadHash: text('payload_hash').notNull(),
  state: text('state').notNull().default('open'),
  selectionCursor: integer('selection_cursor').notNull().default(0),
  responseJson: text('response_json'),
  createdAt: integer('created_at').notNull(),
})

export const processingAttempts = sqliteTable(
  'processing_attempts',
  {
    taskId: text('task_id')
      .notNull()
      .references(() => processingTasks.id, { onDelete: 'cascade' }),
    attemptNumber: integer('attempt_number').notNull(),
    leaseEpoch: integer('lease_epoch').notNull().default(0),
    startedAt: integer('started_at').notNull(),
    finishedAt: integer('finished_at'),
    outcome: text('outcome').notNull().default('open'),
    retryable: integer('retryable').notNull().default(0),
    errorCode: text('error_code'),
    errorMessage: text('error_message'),
    providerRequestId: text('provider_request_id'),
  },
  (table) => ({
    pk: primaryKey({ columns: [table.taskId, table.attemptNumber] }),
    taskIdx: index('idx_processing_attempts_task').on(table.taskId, table.attemptNumber),
  })
)
export const processingCheckpoints = sqliteTable(
  'processing_checkpoints',
  {
    taskId: text('task_id')
      .notNull()
      .references(() => processingTasks.id, { onDelete: 'cascade' }),
    unitKey: text('unit_key').notNull(),
    inputFingerprint: text('input_fingerprint').notNull(),
    contractHash: text('contract_hash').notNull(),
    payload: text('payload').notNull().default('{}'),
    payloadChecksum: text('payload_checksum').notNull().default(''),
    createdAt: integer('created_at').notNull(),
  },
  (table) => ({
    pk: primaryKey({ columns: [table.taskId, table.unitKey] }),
  })
)

export const processingAssetRevisions = sqliteTable('processing_asset_revisions', {
  assetId: text('asset_id')
    .primaryKey()
    .references(() => assets.id, { onDelete: 'cascade' }),
  sourceRevision: integer('source_revision').notNull().default(0),
  embeddingCompletedRevision: integer('embedding_completed_revision'),
  invalidatedAt: integer('invalidated_at'),
  invalidationReason: text('invalidation_reason'),
  autoSuppressedRevision: integer('auto_suppressed_revision'),
})
export const processingMeta = sqliteTable('processing_meta', {
  key: text('key').primaryKey(),
  value: text('value').notNull(),
})
