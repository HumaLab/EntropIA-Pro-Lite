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
  origin: text('origin', { enum: ['user', 'manual', 'repair', 'bibliography'] }).notNull(),
  state: text('state').notNull(),
  desiredState: text('desired_state').notNull(),
  operations: text('operations').notNull(),
  configSnapshotJson: text('config_snapshot_json').notNull().default('{}'),
  planningCursor: integer('planning_cursor').notNull().default(0),
  planningDone: integer('planning_done').notNull().default(0),
  revision: integer('revision').notNull().default(0),
  priority: integer('priority').notNull().default(0),
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
    kind: text('kind', { enum: ['ocr', 'embedding', 'bibliography_sync', 'bibliography_profile', 'bibliography_extract'] }).notNull(),
    assetIdSnapshot: text('asset_id_snapshot').notNull(),
    // E2a-1 task-subject identity (migration 0041). Dual-written alongside
    // the snapshot for corpus rows; lookups stay on (kind, assetIdSnapshot)
    // until the E2a-2 cutover. SQL (CHECKs, partial uniques) is authoritative.
    domain: text('domain').notNull().default('corpus'),
    subjectKind: text('subject_kind').notNull().default('asset'),
    subjectId: text('subject_id').notNull().default(''),
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
    sourceInvalidationCount: integer('source_invalidation_count').notNull().default(0),
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
    kind: text('kind', { enum: ['ocr', 'embedding', 'bibliography_sync', 'bibliography_profile', 'bibliography_extract'] }).notNull(),
    assetIdSnapshot: text('asset_id_snapshot').notNull(),
    // E2a-1 task-subject identity mirror (migration 0041); see processingTasks.
    domain: text('domain').notNull().default('corpus'),
    subjectKind: text('subject_kind').notNull().default('asset'),
    subjectId: text('subject_id').notNull().default(''),
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

// ---------------------------------------------------------------------------
// Zotero bibliography catalog foundation (migration 0038). A connection is
// the source namespace; the library and native item key qualify an item. Later
// migrations add collections, tags, attachments and reconciliation state.
// ---------------------------------------------------------------------------
export const zoteroConnections = sqliteTable(
  'zotero_connections',
  {
    id: text('id').primaryKey(),
    sourceOrigin: text('source_origin', { enum: ['local', 'web'] }).notNull(),
    sourceInstanceId: text('source_instance_id'),
    endpoint: text('endpoint'),
    capabilitiesJson: text('capabilities_json').notNull().default('{}'),
    credentialRef: text('credential_ref'),
    state: text('state').notNull().default('unknown'),
    revision: integer('revision').notNull().default(0),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => ({
    sourceIdx: index('idx_zotero_connections_source').on(
      table.sourceOrigin,
      table.sourceInstanceId
    ),
  })
)

export const zoteroLibraries = sqliteTable(
  'zotero_libraries',
  {
    id: text('id').primaryKey(),
    connectionId: text('connection_id')
      .notNull()
      .references(() => zoteroConnections.id, { onDelete: 'cascade' }),
    libraryType: text('library_type', { enum: ['user', 'group'] }).notNull(),
    libraryId: text('library_id').notNull(),
    name: text('name').notNull(),
    lastModifiedVersion: integer('last_modified_version'),
    revision: integer('revision').notNull().default(0),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => ({
    identityUnique: uniqueIndex('idx_zotero_libraries_identity').on(
      table.connectionId,
      table.libraryType,
      table.libraryId
    ),
    connectionIdx: index('idx_zotero_libraries_connection').on(table.connectionId),
  })
)

export const bibliographicItems = sqliteTable(
  'bibliographic_items',
  {
    id: text('id').primaryKey(),
    libraryId: text('library_id')
      .notNull()
      .references(() => zoteroLibraries.id, { onDelete: 'cascade' }),
    itemKey: text('item_key').notNull(),
    itemVersion: integer('item_version'),
    nativeJsonSnapshot: text('native_json_snapshot').notNull(),
    cslJsonSnapshot: text('csl_json_snapshot').notNull(),
    itemType: text('item_type'),
    title: text('title'),
    creatorsJson: text('creators_json'),
    publicationTitle: text('publication_title'),
    publisher: text('publisher'),
    date: text('date'),
    doi: text('doi'),
    isbn: text('isbn'),
    abstract: text('abstract'),
    language: text('language'),
    url: text('url'),
    revision: integer('revision').notNull().default(0),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
    verifiedAt: integer('verified_at').notNull(),
  },
  (table) => ({
    libraryKeyUnique: uniqueIndex('idx_bibliographic_items_library_key').on(
      table.libraryId,
      table.itemKey
    ),
    entityLibraryUnique: uniqueIndex('idx_bibliographic_items_id_library').on(
      table.id,
      table.libraryId
    ),
    keyIdx: index('idx_bibliographic_items_key').on(table.itemKey),
    titleIdx: index('idx_bibliographic_items_title').on(sql`${table.title} COLLATE NOCASE`),
  })
)

// ---------------------------------------------------------------------------
// Zotero catalog relations (migration 0039). Native keys remain raw and
// library-qualified; tombstones are separate one-to-one records so the 0038
// item snapshots and all membership edges remain intact.
// ---------------------------------------------------------------------------
export const zoteroCollections = sqliteTable(
  'zotero_collections',
  {
    id: text('id').primaryKey(),
    libraryId: text('library_id')
      .notNull()
      .references(() => zoteroLibraries.id, { onDelete: 'cascade' }),
    collectionKey: text('collection_key').notNull(),
    name: text('name').notNull(),
    parentCollectionKey: text('parent_collection_key'),
    nativeJsonSnapshot: text('native_json_snapshot').notNull(),
    nativeVersion: integer('native_version'),
    revision: integer('revision').notNull().default(0),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
    verifiedAt: integer('verified_at').notNull(),
  },
  (table) => ({
    libraryKeyUnique: uniqueIndex('idx_zotero_collections_library_key').on(
      table.libraryId,
      table.collectionKey
    ),
    entityLibraryUnique: uniqueIndex('idx_zotero_collections_id_library').on(
      table.id,
      table.libraryId
    ),
    libraryIdx: index('idx_zotero_collections_library').on(table.libraryId),
  })
)

export const zoteroTags = sqliteTable(
  'zotero_tags',
  {
    id: text('id').primaryKey(),
    libraryId: text('library_id')
      .notNull()
      .references(() => zoteroLibraries.id, { onDelete: 'cascade' }),
    tagText: text('tag_text').notNull(),
    tagType: text('tag_type'),
    nativeJsonSnapshot: text('native_json_snapshot').notNull(),
    nativeVersion: integer('native_version'),
    revision: integer('revision').notNull().default(0),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
    verifiedAt: integer('verified_at').notNull(),
  },
  (table) => ({
    libraryTextUnique: uniqueIndex('idx_zotero_tags_library_text').on(
      table.libraryId,
      table.tagText
    ),
    entityLibraryUnique: uniqueIndex('idx_zotero_tags_id_library').on(table.id, table.libraryId),
    libraryIdx: index('idx_zotero_tags_library').on(table.libraryId),
  })
)

export const zoteroAttachments = sqliteTable(
  'zotero_attachments',
  {
    id: text('id').primaryKey(),
    itemId: text('item_id')
      .notNull()
      .references(() => bibliographicItems.id, { onDelete: 'cascade' }),
    attachmentKey: text('attachment_key').notNull(),
    contentType: text('content_type'),
    linkMode: text('link_mode'),
    filename: text('filename'),
    nativePath: text('native_path'),
    url: text('url'),
    md5: text('md5'),
    mtime: integer('mtime'),
    nativeJsonSnapshot: text('native_json_snapshot').notNull(),
    nativeVersion: integer('native_version'),
    revision: integer('revision').notNull().default(0),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
    verifiedAt: integer('verified_at').notNull(),
  },
  (table) => ({
    itemKeyUnique: uniqueIndex('idx_zotero_attachments_item_key').on(
      table.itemId,
      table.attachmentKey
    ),
    itemIdx: index('idx_zotero_attachments_item').on(table.itemId),
  })
)

export const zoteroItemCollections = sqliteTable(
  'zotero_item_collections',
  {
    libraryId: text('library_id')
      .notNull()
      .references(() => zoteroLibraries.id, { onDelete: 'cascade' }),
    itemId: text('item_id').notNull(),
    collectionId: text('collection_id').notNull(),
  },
  (table) => ({
    pk: primaryKey({ columns: [table.libraryId, table.itemId, table.collectionId] }),
    itemLibraryFk: foreignKey({
      columns: [table.itemId, table.libraryId],
      foreignColumns: [bibliographicItems.id, bibliographicItems.libraryId],
      name: 'zotero_item_collections_item_library_fkey',
    }).onDelete('cascade'),
    collectionLibraryFk: foreignKey({
      columns: [table.collectionId, table.libraryId],
      foreignColumns: [zoteroCollections.id, zoteroCollections.libraryId],
      name: 'zotero_item_collections_collection_library_fkey',
    }).onDelete('cascade'),
    itemIdx: index('idx_zotero_item_collections_item').on(table.libraryId, table.itemId),
    collectionIdx: index('idx_zotero_item_collections_collection').on(
      table.libraryId,
      table.collectionId
    ),
  })
)

export const zoteroItemTags = sqliteTable(
  'zotero_item_tags',
  {
    libraryId: text('library_id')
      .notNull()
      .references(() => zoteroLibraries.id, { onDelete: 'cascade' }),
    itemId: text('item_id').notNull(),
    tagId: text('tag_id').notNull(),
  },
  (table) => ({
    pk: primaryKey({ columns: [table.libraryId, table.itemId, table.tagId] }),
    itemLibraryFk: foreignKey({
      columns: [table.itemId, table.libraryId],
      foreignColumns: [bibliographicItems.id, bibliographicItems.libraryId],
      name: 'zotero_item_tags_item_library_fkey',
    }).onDelete('cascade'),
    tagLibraryFk: foreignKey({
      columns: [table.tagId, table.libraryId],
      foreignColumns: [zoteroTags.id, zoteroTags.libraryId],
      name: 'zotero_item_tags_tag_library_fkey',
    }).onDelete('cascade'),
    itemIdx: index('idx_zotero_item_tags_item').on(table.libraryId, table.itemId),
    tagIdx: index('idx_zotero_item_tags_tag').on(table.libraryId, table.tagId),
  })
)

export const zoteroItemTombstones = sqliteTable('zotero_item_tombstones', {
  itemId: text('item_id')
    .primaryKey()
    .references(() => bibliographicItems.id, { onDelete: 'cascade' }),
  observedAt: integer('observed_at').notNull(),
  remoteVersion: integer('remote_version'),
  reason: text('reason').notNull(),
})

export const zoteroCollectionTombstones = sqliteTable('zotero_collection_tombstones', {
  collectionId: text('collection_id')
    .primaryKey()
    .references(() => zoteroCollections.id, { onDelete: 'cascade' }),
  observedAt: integer('observed_at').notNull(),
  remoteVersion: integer('remote_version'),
  reason: text('reason').notNull(),
})

export const zoteroTagTombstones = sqliteTable('zotero_tag_tombstones', {
  tagId: text('tag_id')
    .primaryKey()
    .references(() => zoteroTags.id, { onDelete: 'cascade' }),
  observedAt: integer('observed_at').notNull(),
  remoteVersion: integer('remote_version'),
  reason: text('reason').notNull(),
})

export const zoteroAttachmentTombstones = sqliteTable('zotero_attachment_tombstones', {
  attachmentId: text('attachment_id')
    .primaryKey()
    .references(() => zoteroAttachments.id, { onDelete: 'cascade' }),
  observedAt: integer('observed_at').notNull(),
  remoteVersion: integer('remote_version'),
  reason: text('reason').notNull(),
})

// ---------------------------------------------------------------------------
// Zotero reconciliation durability (migration 0040). One current run row is
// retained per internal library; the normalized seen-set is scoped by both the
// library FK and generated run id so native keys never cross either boundary.
// ---------------------------------------------------------------------------
export const zoteroReconciliationRuns = sqliteTable(
  'zotero_reconciliation_runs',
  {
    libraryId: text('library_id')
      .primaryKey()
      .notNull()
      .references(() => zoteroLibraries.id, { onDelete: 'cascade' }),
    runId: text('run_id').notNull(),
    connectionRevision: integer('connection_revision').notNull(),
    state: text('state', {
      enum: ['running', 'retry_wait', 'interrupted', 'blocked', 'failed', 'completed'],
    }).notNull(),
    phase: text('phase', { enum: ['versions', 'catalog', 'finalize'] }).notNull(),
    cursorStart: integer('cursor_start').notNull().default(0),
    cursorLimit: integer('cursor_limit').notNull(),
    remoteTotal: integer('remote_total'),
    targetVersion: integer('target_version'),
    checkpointVersion: integer('checkpoint_version'),
    retryCount: integer('retry_count').notNull().default(0),
    attemptCount: integer('attempt_count').notNull().default(0),
    nextRetryAt: integer('next_retry_at'),
    lastAttemptAt: integer('last_attempt_at'),
    latestErrorPhase: text('latest_error_phase', {
      enum: ['versions', 'catalog', 'finalize'],
    }),
    latestErrorCode: text('latest_error_code'),
    latestErrorMessage: text('latest_error_message'),
    latestErrorRetryable: integer('latest_error_retryable'),
    latestErrorAt: integer('latest_error_at'),
    revision: integer('revision').notNull().default(0),
    checkpointedAt: integer('checkpointed_at'),
    completedAt: integer('completed_at'),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => ({
    runUnique: uniqueIndex('idx_zotero_reconciliation_runs_library_run').on(
      table.libraryId,
      table.runId
    ),
    runIdUnique: uniqueIndex('idx_zotero_reconciliation_runs_run_id_unique').on(table.runId),
    stateIdx: index('idx_zotero_reconciliation_runs_state').on(
      table.state,
      table.nextRetryAt,
      table.libraryId
    ),
  })
)

export const zoteroReconciliationSeen = sqliteTable(
  'zotero_reconciliation_seen',
  {
    libraryId: text('library_id').notNull(),
    runId: text('run_id').notNull(),
    entityKind: text('entity_kind', {
      enum: ['item', 'collection', 'tag', 'attachment'],
    }).notNull(),
    entityKey: text('entity_key').notNull(),
    parentKey: text('parent_key').notNull().default(''),
    remoteVersion: integer('remote_version'),
    observedAt: integer('observed_at').notNull(),
  },
  (table) => ({
    pk: primaryKey({
      columns: [table.libraryId, table.runId, table.entityKind, table.entityKey, table.parentKey],
    }),
    runFk: foreignKey({
      columns: [table.libraryId, table.runId],
      foreignColumns: [zoteroReconciliationRuns.libraryId, zoteroReconciliationRuns.runId],
      name: 'zotero_reconciliation_seen_run_fkey',
    }).onDelete('cascade'),
    kindIdx: index('idx_zotero_reconciliation_seen_kind').on(
      table.libraryId,
      table.runId,
      table.entityKind,
      table.entityKey
    ),
  })
)

// ---------------------------------------------------------------------------
// Writing workspace (plan-editor.md §9). The canonical manuscript is the
// ProseMirror JSON in currentContentJson; the citation tables below are
// projections of the current revision (§8.4), not a second editable truth.
//
// Corpus references on the citation projection (collectionId, itemId, assetId)
// are deliberately plain columns with no foreign key, matching
// entities.assetId, triples.assetId and the processing_* family: §10.3 needs a
// citation to outlive its source, carrying a metadata snapshot and an
// integrity status instead of vanishing with it.
// ---------------------------------------------------------------------------
export const writingDocuments = sqliteTable(
  'writing_documents',
  {
    id: text('id').primaryKey(),
    title: text('title').notNull(),
    documentType: text('document_type').notNull(),
    status: text('status').notNull().default('active'),
    schemaVersion: integer('schema_version').notNull(),
    currentContentJson: text('current_content_json').notNull(),
    revision: integer('revision').notNull().default(0),
    plainTextCache: text('plain_text_cache'),
    citationStyleId: text('citation_style_id'),
    citationLocale: text('citation_locale'),
    bibliographyEnabled: integer('bibliography_enabled').notNull().default(1),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
    lastOpenedAt: integer('last_opened_at'),
  },
  (table) => ({
    statusUpdatedIdx: index('idx_writing_documents_status_updated').on(
      table.status,
      table.updatedAt
    ),
  })
)

// The one real corpus foreign key, cascading on purpose: the delete path in
// collection.repo.ts is hand-rolled and knows nothing about these tables, so a
// RESTRICT here would block deleting a collection.
export const writingDocumentCollections = sqliteTable(
  'writing_document_collections',
  {
    documentId: text('document_id')
      .notNull()
      .references(() => writingDocuments.id, { onDelete: 'cascade' }),
    collectionId: text('collection_id')
      .notNull()
      .references(() => collections.id, { onDelete: 'cascade' }),
    isPrimary: integer('is_primary').notNull().default(0),
    createdAt: integer('created_at').notNull(),
  },
  (table) => ({
    pk: primaryKey({ columns: [table.documentId, table.collectionId] }),
    collectionIdx: index('idx_writing_document_collections_collection').on(table.collectionId),
  })
)

export const writingDocumentVersions = sqliteTable(
  'writing_document_versions',
  {
    id: text('id').primaryKey(),
    documentId: text('document_id')
      .notNull()
      .references(() => writingDocuments.id, { onDelete: 'cascade' }),
    versionNumber: integer('version_number').notNull(),
    contentJson: text('content_json').notNull(),
    schemaVersion: integer('schema_version').notNull(),
    documentSettingsJson: text('document_settings_json').notNull().default('{}'),
    reason: text('reason').notNull(),
    contentHash: text('content_hash').notNull(),
    createdAt: integer('created_at').notNull(),
  },
  (table) => ({
    documentVersionUnique: uniqueIndex('idx_writing_document_versions_unique').on(
      table.documentId,
      table.versionNumber
    ),
  })
)

export const writingDocumentCitations = sqliteTable(
  'writing_document_citations',
  {
    id: text('id').primaryKey(),
    documentId: text('document_id')
      .notNull()
      .references(() => writingDocuments.id, { onDelete: 'cascade' }),
    citationNodeId: text('citation_node_id').notNull(),
    collectionId: text('collection_id'),
    itemId: text('item_id'),
    assetId: text('asset_id'),
    pageNumber: integer('page_number'),
    startChar: integer('start_char'),
    endChar: integer('end_char'),
    sourceRegionJson: text('source_region_json'),
    quotedText: text('quoted_text'),
    sourceTextHash: text('source_text_hash'),
    locatorJson: text('locator_json'),
    metadataSnapshotJson: text('metadata_snapshot_json').notNull().default('{}'),
    integrityStatus: text('integrity_status').notNull().default('valid'),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => ({
    nodeUnique: uniqueIndex('idx_writing_document_citations_node').on(
      table.documentId,
      table.citationNodeId
    ),
    assetIdx: index('idx_writing_document_citations_asset').on(table.assetId),
  })
)

// One visible citation can be a cluster of several works, so identity is
// (cluster, position) — never one row per parenthesis (§9.5).
export const writingZoteroCitations = sqliteTable(
  'writing_zotero_citations',
  {
    id: text('id').primaryKey(),
    documentId: text('document_id')
      .notNull()
      .references(() => writingDocuments.id, { onDelete: 'cascade' }),
    citationNodeId: text('citation_node_id').notNull(),
    citationClusterId: text('citation_cluster_id').notNull(),
    itemPosition: integer('item_position').notNull(),
    sourceOrigin: text('source_origin').notNull().default('local'),
    sourceInstanceId: text('source_instance_id'),
    libraryType: text('library_type').notNull(),
    libraryId: text('library_id').notNull(),
    itemKey: text('item_key').notNull(),
    itemVersion: integer('item_version'),
    locatorType: text('locator_type'),
    locator: text('locator'),
    prefix: text('prefix'),
    suffix: text('suffix'),
    suppressAuthor: integer('suppress_author').notNull().default(0),
    authorOnly: integer('author_only').notNull().default(0),
    itemCslJsonSnapshot: text('item_csl_json_snapshot').notNull().default('{}'),
    integrityStatus: text('integrity_status').notNull().default('valid'),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => ({
    clusterPositionUnique: uniqueIndex('idx_writing_zotero_citations_cluster').on(
      table.documentId,
      table.citationClusterId,
      table.itemPosition
    ),
    itemIdx: index('idx_writing_zotero_citations_item').on(
      table.libraryType,
      table.libraryId,
      table.itemKey
    ),
  })
)

// Append-only record of operations (§8.4): not a projection, and not cleared
// when the text it describes is edited away.
export const writingProvenanceEvents = sqliteTable(
  'writing_provenance_events',
  {
    id: text('id').primaryKey(),
    documentId: text('document_id')
      .notNull()
      .references(() => writingDocuments.id, { onDelete: 'cascade' }),
    versionId: text('version_id').references(() => writingDocumentVersions.id, {
      onDelete: 'set null',
    }),
    rangeAnchorJson: text('range_anchor_json'),
    originType: text('origin_type').notNull(),
    operationType: text('operation_type').notNull(),
    sourceReferenceJson: text('source_reference_json'),
    modelProvider: text('model_provider'),
    modelName: text('model_name'),
    promptTemplateId: text('prompt_template_id'),
    createdAt: integer('created_at').notNull(),
  },
  (table) => ({
    documentIdx: index('idx_writing_provenance_events_document').on(
      table.documentId,
      table.createdAt
    ),
  })
)

// Pending suggestions live outside the canonical content (§8.2). The target is
// pinned by an anchor plus a hash of the SELECTED content, never a hash of the
// whole manuscript (§9.7).
export const writingAgentSuggestions = sqliteTable(
  'writing_agent_suggestions',
  {
    id: text('id').primaryKey(),
    documentId: text('document_id')
      .notNull()
      .references(() => writingDocuments.id, { onDelete: 'cascade' }),
    selectionAnchorJson: text('selection_anchor_json'),
    sourceRevision: integer('source_revision').notNull(),
    selectedContentHash: text('selected_content_hash').notNull(),
    actionType: text('action_type').notNull(),
    originalText: text('original_text'),
    suggestedText: text('suggested_text'),
    rationale: text('rationale'),
    evidenceJson: text('evidence_json').notNull().default('{}'),
    status: text('status').notNull().default('pending'),
    provider: text('provider'),
    model: text('model'),
    createdAt: integer('created_at').notNull(),
    resolvedAt: integer('resolved_at'),
  },
  (table) => ({
    documentStatusIdx: index('idx_writing_agent_suggestions_document').on(
      table.documentId,
      table.status
    ),
  })
)

// Durable recovery journal (plan-editor.md §16.1). Stores deltas, never whole
// documents: spike S6 measured a 1 KB delta at a p95 of 1.17 ms against 97.5 ms
// for the full manuscript. Persisting an entry here is not a canonical save —
// the UI shows "Guardado" only once writingDocuments.revision advances.
export const writingJournal = sqliteTable(
  'writing_journal',
  {
    documentId: text('document_id')
      .notNull()
      .references(() => writingDocuments.id, { onDelete: 'cascade' }),
    seq: integer('seq').notNull(),
    baseRevision: integer('base_revision').notNull(),
    schemaVersion: integer('schema_version').notNull(),
    deltaJson: text('delta_json').notNull(),
    checksum: text('checksum').notNull(),
    createdAt: integer('created_at').notNull(),
  },
  (table) => ({
    pk: primaryKey({ columns: [table.documentId, table.seq] }),
    replayIdx: index('idx_writing_journal_replay').on(
      table.documentId,
      table.baseRevision,
      table.seq
    ),
  })
)

// ---------------------------------------------------------------------------
// Bibliographic semantic profiles — one canonical-text row per verified work
// (migration 0045_bibliographic_semantic_profiles, E3b-WU1). Profiles are
// reconstructible from the verified catalog; embeddings keep their own
// contract/generation tables, so model changes never rewrite profile history.
// ---------------------------------------------------------------------------
export const bibliographicSemanticProfiles = sqliteTable(
  'bibliographic_semantic_profiles',
  {
    itemId: text('item_id')
      .primaryKey()
      .notNull()
      .references(() => bibliographicItems.id, { onDelete: 'cascade' }),
    profileRevision: integer('profile_revision').notNull(),
    templateVersion: text('template_version').notNull(),
    canonicalText: text('canonical_text').notNull(),
    inputHash: text('input_hash').notNull(),
    fieldProvenanceJson: text('field_provenance_json').notNull(),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => ({
    hashIdx: index('idx_bibliographic_semantic_profiles_hash').on(table.inputHash),
  })
)

// Bibliographic work embeddings — one vector per (work, contract) under the
// effective embedding contract (migration 0046_bibliography_profile_tasks,
// E3b-WU2). Generations arrive in E3c without rewriting this identity.
export const bibliographicItemEmbeddings = sqliteTable(
  'bibliographic_item_embeddings',
  {
    itemId: text('item_id')
      .notNull()
      .references(() => bibliographicItems.id, { onDelete: 'cascade' }),
    generationId: text('generation_id')
      .notNull()
      .references(() => bibliographicIndexGenerations.id),
    embeddingContract: text('embedding_contract').notNull(),
    embeddingModel: text('embedding_model').notNull(),
    dimensions: integer('dimensions').notNull(),
    embedding: text('embedding').notNull(),
    inputHash: text('input_hash').notNull(),
    profileRevision: integer('profile_revision').notNull(),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => ({
    pk: primaryKey({ columns: [table.itemId, table.generationId] }),
    hashIdx: index('idx_bibliographic_item_embeddings_hash').on(table.inputHash),
    generationIdx: index('idx_bibliographic_item_embeddings_generation').on(
      table.generationId,
      table.itemId
    ),
  })
)

// Bibliographic embedding contracts (immutable vector spaces) and index
// generations with a per-contract active pointer (migration
// 0047_bibliographic_index_generations, E3c-WU1). Execution in staging
// generations (E3c-WU2) and hybrid retrieval (E3c-WU3) build on these rows.
export const bibliographicEmbeddingContracts = sqliteTable('bibliographic_embedding_contracts', {
  contractHash: text('contract_hash').primaryKey(),
  provider: text('provider').notNull(),
  model: text('model').notNull(),
  dimensions: integer('dimensions').notNull(),
  chunkingContract: text('chunking_contract').notNull(),
  createdAt: integer('created_at').notNull(),
})

export const bibliographicIndexGenerations = sqliteTable(
  'bibliographic_index_generations',
  {
    id: text('id').primaryKey(),
    contractHash: text('contract_hash')
      .notNull()
      .references(() => bibliographicEmbeddingContracts.contractHash),
    status: text('status', { enum: ['staging', 'active', 'retired'] }).notNull(),
    expectedInputs: integer('expected_inputs').notNull().default(0),
    completedInputs: integer('completed_inputs').notNull().default(0),
    createdAt: integer('created_at').notNull(),
    activatedAt: integer('activated_at'),
    retiredAt: integer('retired_at'),
  },
  (table) => ({
    singleActive: uniqueIndex('idx_bibliographic_generations_single_active')
      .on(table.contractHash)
      .where(sql`status = 'active'`),
    contractIdx: index('idx_bibliographic_generations_contract').on(
      table.contractHash,
      table.status
    ),
  })
)

// Bibliographic native extractions — one whole-document row per attachment
// (migration 0050_bibliographic_extraction_tasks, E4a-WU2). Managed
// derivatives: a catalog row delete cascades. Per-page rows arrive with
// selective OCR (E4b) under their own migration.
export const bibliographicExtractions = sqliteTable(
  'bibliographic_extractions',
  {
    attachmentId: text('attachment_id')
      .primaryKey()
      .notNull()
      .references(() => zoteroAttachments.id, { onDelete: 'cascade' }),
    itemId: text('item_id').notNull(),
    pageCount: integer('page_count').notNull(),
    method: text('method', { enum: ['native'] }).notNull(),
    textContent: text('text_content').notNull(),
    textHash: text('text_hash').notNull(),
    textChars: integer('text_chars').notNull(),
    quality: text('quality', { enum: ['rich', 'sparse', 'empty'] }).notNull(),
    sourceMtime: integer('source_mtime'),
    sourceBytes: integer('source_bytes').notNull(),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => ({
    itemIdx: index('idx_bibliographic_extractions_item').on(table.itemId),
  })
)

// Bibliographic per-page native texts — one row per attachment page
// (migration 0051_bibliographic_page_texts, E4b-WU2). The selective OCR pass
// (E4b-WU3) adds 'ocr' rows beside these; managed derivatives with catalog
// cascade like the whole-document row.
export const bibliographicPageTexts = sqliteTable(
  'bibliographic_page_texts',
  {
    attachmentId: text('attachment_id')
      .notNull()
      .references(() => zoteroAttachments.id, { onDelete: 'cascade' }),
    pageNumber: integer('page_number').notNull(),
    method: text('method', { enum: ['native', 'ocr'] }).notNull(),
    textContent: text('text_content').notNull(),
    textHash: text('text_hash').notNull(),
    textChars: integer('text_chars').notNull(),
    quality: text('quality', { enum: ['rich', 'sparse', 'empty', 'unreadable'] }).notNull(),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => ({
    pk: primaryKey({ columns: [table.attachmentId, table.pageNumber] }),
    attachmentIdx: index('idx_bibliographic_page_texts_attachment').on(
      table.attachmentId,
      table.pageNumber
    ),
  })
)

// Bibliographic structural chunks and page spans — one chunk per (work,
// ordinal) with exact page offsets, including multi-page chunks (migration
// 0052_bibliographic_chunks, E4c-WU1). Chunk vectors keyed by chunk id and
// generation arrive in E4c-WU2.
export const bibliographicChunks = sqliteTable(
  'bibliographic_chunks',
  {
    id: text('id').primaryKey(),
    itemId: text('item_id')
      .notNull()
      .references(() => bibliographicItems.id, { onDelete: 'cascade' }),
    attachmentId: text('attachment_id')
      .notNull()
      .references(() => zoteroAttachments.id, { onDelete: 'cascade' }),
    ordinal: integer('ordinal').notNull(),
    textContent: text('text_content').notNull(),
    textHash: text('text_hash').notNull(),
    chunkingContract: text('chunking_contract').notNull(),
    createdAt: integer('created_at').notNull(),
    updatedAt: integer('updated_at').notNull(),
  },
  (table) => ({
    itemIdx: index('idx_bibliographic_chunks_item').on(table.itemId, table.ordinal),
  })
)

export const bibliographicChunkSpans = sqliteTable(
  'bibliographic_chunk_spans',
  {
    chunkId: text('chunk_id')
      .notNull()
      .references(() => bibliographicChunks.id, { onDelete: 'cascade' }),
    pageNumber: integer('page_number').notNull(),
    startChar: integer('start_char').notNull(),
    endChar: integer('end_char').notNull(),
  },
  (table) => ({
    pk: primaryKey({ columns: [table.chunkId, table.pageNumber, table.startChar] }),
  })
)
