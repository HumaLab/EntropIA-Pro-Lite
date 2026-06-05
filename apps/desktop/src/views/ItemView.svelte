<script lang="ts">
  import { getStore } from '$lib/db'
  import { getAssetUrl } from '$lib/file-import'
  import {
    buildLayoutBlockViews,
    countLayoutBlocksByFilter,
    filterBlocksByPage,
    filterRegionsByPage,
    filterLayoutBlocksByType,
    getBlockCountByPage,
    getLayoutByAsset,
    getPagesFromLayout,
    LAYOUT_BLOCK_FILTERS,
    type LayoutBlockFilterId,
  } from '$lib/layouts'
  import {
    formatLayoutBbox,
    getLayoutOverlaySourceMeta,
    serializeLayoutBlock,
  } from '$lib/layout-inspector'
  import { OcrStore, extractText, type OcrMode } from '$lib/ocr'
  import { TranscriptionStore, transcribeAudio, transcribeDictation } from '$lib/transcription'
  import {
    NlpStore,
    indexFts,
    embedAsset,
    extractEntities,
    extractEntitiesForAsset,
    similarAssets as fetchSimilarAssets,
    type SimilarAsset,
  } from '$lib/nlp'
  import {
    LlmStore,
    llmSummarize,
    llmCorrectOcr,
    llmExtractTriples,
    llmSummarizeAsset,
    llmCorrectOcrAsset,
    llmExtractTriplesAsset,
    llmIsAvailable,
    llmGetResult,
  } from '$lib/llm'
  import { GeoStore } from '$lib/geo'
  import {
    DocumentViewer,
    MetadataEditor,
    NoteEditor,
    ActionIcon,
    EntityViewer,
    MapViewer,
    TopicEditor,
    isNoteHtmlEffectivelyEmpty,
    normalizeNoteLinkHref,
    normalizeNoteContentForRender,
  } from '@entropia/ui'
  import type { MapMarker } from '@entropia/ui'
  import { onMount, onDestroy } from 'svelte'
  import { listen, emit } from '@tauri-apps/api/event'
  import { invoke } from '@tauri-apps/api/core'
  import { navigation } from '$lib/navigation'
  import { getFocusableElements, getNextFocusTrapTarget } from '$lib/modal-focus'
  import {
    DOCUMENT_EXPLORER_ASSET_SELECTED_EVENT,
    DOCUMENT_EXPLORER_ASSET_SELECT_REQUEST_EVENT,
    type DocumentExplorerAssetDetail,
  } from '$lib/document-explorer'
  import { locale, t, type I18nKey, type I18nParams } from '$lib/i18n'
  import type {
    Item,
    Asset,
    Collection,
    Note,
    Annotation as StoreAnnotation,
    AnnotationKind as StoreAnnotationKind,
  } from '@entropia/store'
  import type {
    Entity,
    ViewerAnnotation,
    ViewerLayoutRegion,
    AnnotationKind as ViewerAnnotationKind,
    EditTool,
    ImageEditResult,
  } from '@entropia/ui'
  import { TranscriptionRepo } from '@entropia/store'

  const isDev = import.meta.env.DEV
  const IMPORTED_FILE_METADATA_KEY = '__entropia_file_metadata'

  type ImportedFileMetadata = {
    originalName?: string
    originalPath?: string
    importedAt?: string
    sizeBytes?: number
    readonly?: boolean
    isFile?: boolean
    isDirectory?: boolean
    createdAt?: number | null
    modifiedAt?: number | null
    accessedAt?: number | null
  }

  // ── Sidebar resize ──
  const MIN_SIDEBAR_PCT = 20
  const MAX_SIDEBAR_PCT = 50
  const DEFAULT_SIDEBAR_PCT = 33

  let sidebarWidth = $state(
    (() => {
      try {
        const stored = localStorage.getItem('entropia-sidebar-width')
        if (stored !== null) {
          const parsed = Number(stored)
          if (!isNaN(parsed)) {
            return Math.max(MIN_SIDEBAR_PCT, Math.min(MAX_SIDEBAR_PCT, parsed))
          }
        }
      } catch {}
      return DEFAULT_SIDEBAR_PCT
    })()
  )

  let isDragging = $state(false)
  let itemViewEl: HTMLElement | undefined = $state()
  let dragCleanup: (() => void) | null = null

  function handleExplorerAssetSelectRequest(event: Event) {
    const detail = (event as CustomEvent<DocumentExplorerAssetDetail>).detail
    if (detail.itemId !== itemId || !detail.assetId) return

    const nextIndex = assets.findIndex((asset) => asset.id === detail.assetId)
    if (nextIndex >= 0) {
      selectedAssetIndex = nextIndex
    }
  }

  function onResizeHandlePointerDown(e: PointerEvent) {
    e.preventDefault()
    isDragging = true

    const startX = e.clientX
    const startWidthPct = sidebarWidth
    const containerEl = itemViewEl ?? document.querySelector('.item-view') ?? document.body
    const containerWidth = (containerEl as HTMLElement).clientWidth

    let rafId: number | null = null
    let lastClientX = startX

    function onPointerMove(e: PointerEvent) {
      lastClientX = e.clientX
      if (rafId !== null) return
      rafId = requestAnimationFrame(() => {
        const deltaX = lastClientX - startX
        const deltaPct = (deltaX / containerWidth) * 100
        sidebarWidth = Math.max(
          MIN_SIDEBAR_PCT,
          Math.min(MAX_SIDEBAR_PCT, startWidthPct - deltaPct)
        )
        rafId = null
      })
    }

    function onPointerUp() {
      isDragging = false
      try {
        localStorage.setItem('entropia-sidebar-width', String(Math.round(sidebarWidth)))
      } catch {}
      window.removeEventListener('pointermove', onPointerMove)
      window.removeEventListener('pointerup', onPointerUp)
      document.body.classList.remove('no-select')
      dragCleanup = null
    }

    document.body.classList.add('no-select')
    window.addEventListener('pointermove', onPointerMove)
    window.addEventListener('pointerup', onPointerUp)
    dragCleanup = onPointerUp
  }

  let { itemId, collectionId }: { itemId: string; collectionId: string } = $props()

  let item = $state<Item | null>(null)
  let assets = $state<Asset[]>([])
  let collection = $state<Collection | null>(null)
  let notes = $state<Note[]>([])
  let loading = $state(true)
  let error = $state<string | null>(null)
  const currentLocale = locale
  const translate = $derived.by(() => {
    $currentLocale
    return (key: I18nKey, params?: I18nParams) => t(key, params)
  })
  let selectedAssetIndex = $state(0)
  let savingMetadata = $state(false)
  let annotations = $state<ViewerAnnotation[]>([])
  let selectedAnnotationId = $state<string | null>(null)
  let annotationTool = $state<'select' | 'rectangle' | 'underline'>('select')
  let annotationColor = $state('var(--color-accent)')
  let annotationSaveError = $state<string | null>(null)
  let annotationSaveTimer: ReturnType<typeof setTimeout> | null = null
  let pendingAnnotationSave: {
    assetId: string
    annotations: ViewerAnnotation[]
  } | null = null

  let assetLayout = $state<Awaited<ReturnType<typeof getLayoutByAsset>>>(null)
  let layoutLoading = $state(false)
  let layoutError = $state<string | null>(null)
  let showLayout = $state(false)
  let layoutTypeFilter = $state<LayoutBlockFilterId>('all')
  let layoutHoveredBlockId = $state<string | null>(null)
  let layoutSelectedBlockId = $state<string | null>(null)
  let layoutHoveredRegionId = $state<string | null>(null)
  let layoutSelectedRegionId = $state<string | null>(null)
  let layoutBlockListEl = $state<HTMLDivElement | null>(null)
  let lastAutoScrolledLayoutBlockId = $state<string | null>(null)
  let layoutInspectorCopyMessage = $state<{ tone: 'success' | 'error'; text: string } | null>(null)
  let layoutInspectorCopyTimer = $state<ReturnType<typeof setTimeout> | null>(null)
  let layoutLoadToken = 0
  let viewerPage = $state(1)
  let viewerTotalPages = $state(1)
  let leftPanelTab = $state<'document' | 'text'>('document')

  // Image edit state
  let editTool = $state<EditTool>('none')
  let imageVersion = $state(0)

  // Undo history: stack of { path, width, height, annotations } snapshots
  // Each entry represents the state BEFORE an edit operation. Popping restores
  // the asset to that state (the file is still on disk because edits create
  // versioned files rather than overwriting in-place).
  interface UndoEntry {
    path: string
    width: number
    height: number
    annotations: ViewerAnnotation[]
  }
  let undoStack = $state<UndoEntry[]>([])
  let canUndo = $derived(undoStack.length > 0)
  let lastSelectedAssetId = $state<string | null>(null)

  // OCR state — plain TS class, updated via Tauri events
  const ocrStore = new OcrStore({
    onComplete: (assetId) => {
      // After OCR extraction completes on a specific asset, auto-trigger
      // asset-level refreshes and entity extraction.
      if (selectedAsset && selectedAsset.id === assetId) {
        void reloadSelectedAssetPersistedState({ layout: true })
        void extractEntitiesForAsset(itemId, assetId).catch(() => {})
      }
    },
  })
  // Reactive tick counter: incremented on every OCR event to force Svelte re-evaluation
  let ocrTick = $state(0)
  // Edited text per asset — tracks user corrections to OCR output
  let ocrEditedText = $state(new Map<string, string>())
  // Debounce timers per asset for persisting edits to DB
  let ocrPersistTimers = $state(new Map<string, ReturnType<typeof setTimeout>>())
  // Debounce timers per asset for downstream NLP reprocessing after user inactivity
  let assetReanalysisTimers = $state(new Map<string, ReturnType<typeof setTimeout>>())

  // Transcription state — mirrors OcrStore pattern for audio assets
  const transcriptionStore = new TranscriptionStore({
    onComplete: (assetId) => {
      // After transcription completes, auto-trigger entity extraction only.
      if (selectedAsset && selectedAsset.id === assetId) {
        void extractEntitiesForAsset(itemId, assetId).catch(() => {})
      }
    },
  })
  let transcriptionTick = $state(0)

  let transEditedText = $state(new Map<string, string>())
  let transPersistTimers = $state(new Map<string, ReturnType<typeof setTimeout>>())

  const PERSIST_IDLE_MS = 500
  const REANALYSIS_IDLE_MS = 1500

  function scheduleAssetReanalysis(assetId: string) {
    const existing = assetReanalysisTimers.get(assetId)
    if (existing) clearTimeout(existing)

    const timer = setTimeout(async () => {
      const jobs: Array<[string, () => Promise<unknown>]> = [
        ['ner', () => extractEntitiesForAsset(itemId, assetId)],
        ['fts', () => indexFts(itemId)],
        ['embed', () => embedAsset(itemId, assetId)],
      ]

      try {
        console.info('[ItemView] Re-running post-edit analysis', { itemId, assetId })

        const results = await Promise.allSettled(jobs.map(([, run]) => run()))
        results.forEach((result, index) => {
          const jobName = jobs[index]?.[0] ?? 'unknown'
          if (result.status === 'rejected') {
            console.error(`[ItemView] Post-edit ${jobName} failed`, result.reason)
          }
        })
      } finally {
        assetReanalysisTimers.delete(assetId)
      }
    }, REANALYSIS_IDLE_MS)

    assetReanalysisTimers.set(assetId, timer)
  }

  /** Save quickly, but only re-run expensive analysis after longer inactivity. */
  function schedulePersist(assetId: string, text: string) {
    // Cancel any pending timer for this asset
    const existing = ocrPersistTimers.get(assetId)
    if (existing) clearTimeout(existing)

    // Schedule new persist
    const timer = setTimeout(async () => {
      try {
        await invoke('update_extraction_text_cmd', { assetId, textContent: text })
        scheduleAssetReanalysis(assetId)
      } catch (e) {
        console.error('[ItemView] Failed to persist OCR correction:', e)
      }
      ocrPersistTimers.delete(assetId)
    }, PERSIST_IDLE_MS)

    ocrPersistTimers.set(assetId, timer)
  }

  /** Schedule a debounced persist of edited transcription text to the DB. */
  function scheduleTranscriptionPersist(assetId: string, text: string) {
    const existing = transPersistTimers.get(assetId)
    if (existing) clearTimeout(existing)

    const timer = setTimeout(async () => {
      try {
        await invoke('update_transcription_text_cmd', { assetId, textContent: text })
        scheduleAssetReanalysis(assetId)
      } catch (e) {
        console.error('[ItemView] Failed to persist transcription correction:', e)
      }
      transPersistTimers.delete(assetId)
    }, PERSIST_IDLE_MS)

    transPersistTimers.set(assetId, timer)
  }

  // NLP state — mirrors OcrStore pattern
  const nlpStore = new NlpStore()
  let nlpTick = $state(0)
  let entities = $state<Entity[]>([])
  type EditableEntityType = 'person' | 'organization' | 'place' | 'misc' | 'date'
  const EDITABLE_ENTITY_TYPES: EditableEntityType[] = [
    'person',
    'organization',
    'place',
    'misc',
    'date',
  ]
  let newEntityValue = $state('')
  let newEntityType = $state<EditableEntityType>('organization')
  let editingEntityId = $state<string | null>(null)
  let editingEntityValue = $state('')
  let entityActionError = $state<string | null>(null)
  let similarAssets = $state<SimilarAsset[]>([])
  let ftsQuery = $state('')
  let ftsResults = $state<
    Array<{ itemId: string; title: string; rank: number; collectionId: string }>
  >([])
  let ftsSearching = $state(false)
  let ftsSearchError = $state<string | null>(null)
  let ftsSearchTimer: ReturnType<typeof setTimeout> | null = null
  let ftsIndexedRows = $state<number | null>(null)
  let ftsDebug = $state<{
    rawQuery: string
    sanitizedQuery: string
    strategy: 'empty' | 'strict' | 'relaxed'
    matchCount: number
    hydratedCount: number
    resultIds: string[]
  } | null>(null)
  let triples = $state<Array<{ subject: string; predicate: string; object: string }>>([])
  let rightPanelTab = $state<'notes' | 'text' | 'analysis' | 'search' | 'layout' | 'metadata'>(
    'notes'
  )
  let rightPanelOpen = $state(true)

  const metadataEditorLabels = {
    keyPlaceholder: 'Campo',
    valuePlaceholder: 'Valor',
    removeFieldAria: 'Eliminar campo',
    addField: '+ Agregar campo',
    fieldLabel: 'Campo',
    valueLabel: 'Valor',
    emptyText: 'No hay metadatos cargados para este documento.',
  }

  const documentViewerLabels = $derived.by(() => {
    $currentLocale
    return {
      imageAlt: translate('item.viewerImageAlt'),
      imageOverlayAriaLabel: translate('item.viewerImageOverlay'),
      audioSkipBack: translate('item.audioSkipBack'),
      audioPlay: translate('item.audioPlay'),
      audioPause: translate('item.audioPause'),
      audioSkipForward: translate('item.audioSkipForward'),
      audioSeek: translate('item.audioSeek'),
      audioVolume: translate('item.audioVolume'),
      pdfLoading: translate('item.viewerPdfLoading'),
      pdfLoadError: translate('item.viewerPdfLoadError'),
      pdfRenderError: translate('item.viewerPdfRenderError'),
      pdfPreviousPage: translate('item.previousPage'),
      pdfNextPage: translate('item.nextPage'),
      pdfZoomOut: translate('item.toolbar.zoomOut'),
      pdfZoomIn: translate('item.toolbar.zoomIn'),
      layoutOverlayAriaLabel: translate('item.viewerLayoutOverlay'),
      layoutRegionAriaLabel: (label: string) => translate('item.viewerLayoutRegion', { label }),
      annotationAriaLabel: (id: string) => translate('item.viewerAnnotation', { id }),
      cropRegionAriaLabel: translate('item.viewerCropRegion'),
      eraseRegionAriaLabel: translate('item.viewerEraseRegion'),
    }
  })

  const annotationToolbarLabels = $derived.by(() => {
    $currentLocale
    return {
      expandToolbar: translate('item.toolbar.expand'),
      expandToolbarTitle: translate('item.toolbar.expandTitle'),
      collapseToolbar: translate('item.toolbar.collapse'),
      collapseToolbarTitle: translate('item.toolbar.collapseTitle'),
      toolbarAriaLabel: translate('item.toolbar.imageTools'),
      undo: translate('item.toolbar.undo'),
      undoTitle: translate('item.toolbar.undoTitle'),
      rectangleTool: translate('item.toolbar.rectangle'),
      underlineTool: translate('item.toolbar.underline'),
      cropTool: translate('item.toolbar.crop'),
      eraseTool: translate('item.toolbar.erase'),
      rotateLeft: translate('item.toolbar.rotateLeft'),
      rotateRight: translate('item.toolbar.rotateRight'),
      zoomOut: translate('item.toolbar.zoomOut'),
      zoomIn: translate('item.toolbar.zoomIn'),
      deleteSelected: translate('item.toolbar.deleteAnnotation'),
      colorAriaLabel: (label: string) => translate('item.toolbar.colorAria', { label }),
    }
  })

  const noteEditorLabels = $derived.by(() => {
    $currentLocale
    return {
      toolbarAriaLabel: translate('item.noteEditor.toolbar'),
      textStyleGroup: translate('item.noteEditor.group.textStyle'),
      structureGroup: translate('item.noteEditor.group.structure'),
      insertGroup: translate('item.noteEditor.group.insert'),
      dictationGroup: translate('item.noteEditor.group.dictation'),
      bold: translate('item.noteEditor.bold'),
      italic: translate('item.noteEditor.italic'),
      underline: translate('item.noteEditor.underline'),
      inlineCode: translate('item.noteEditor.inlineCode'),
      heading1: translate('item.noteEditor.heading1'),
      heading2: translate('item.noteEditor.heading2'),
      heading3: translate('item.noteEditor.heading3'),
      bulletList: translate('item.noteEditor.bulletList'),
      bulletListShort: translate('item.noteEditor.bulletListShort'),
      orderedList: translate('item.noteEditor.orderedList'),
      orderedListShort: translate('item.noteEditor.orderedListShort'),
      quote: translate('item.noteEditor.quote'),
      quoteShort: translate('item.noteEditor.quoteShort'),
      addLink: translate('item.noteEditor.addLink'),
      addLinkShort: translate('item.noteEditor.addLinkShort'),
      removeLink: translate('item.noteEditor.removeLink'),
      removeLinkShort: translate('item.noteEditor.removeLinkShort'),
      dictationStart: translate('item.noteEditor.dictationStart'),
      dictationStop: translate('item.noteEditor.dictationStop'),
      dictationProcessing: translate('item.noteEditor.dictationProcessing'),
      dictationIdle: translate('item.noteEditor.dictationIdle'),
      helperText: translate('item.noteEditor.helper'),
      dictationNoMicrophone: translate('item.noteEditor.noMicrophone'),
      dictationNoAudio: translate('item.noteEditor.noAudio'),
      dictationAutoStopProcessing: translate('item.noteEditor.autoStopProcessing', {
        duration: '{duration}',
      }),
      dictationTranscribing: translate('item.noteEditor.transcribing'),
      dictationAutoStopInserted: translate('item.noteEditor.autoStopInserted', {
        duration: '{duration}',
      }),
      dictationInserted: translate('item.noteEditor.inserted'),
      dictationNoText: translate('item.noteEditor.noText'),
      dictationTranscriptionFailed: translate('item.noteEditor.transcriptionFailed'),
      linkInvalidUrl: translate('item.noteEditor.linkInvalidUrl'),
      linkInvalidHttp: translate('item.noteEditor.linkInvalidHttp'),
      linkInvalidExample: translate('item.noteEditor.linkInvalidExample'),
      linkModalTitle: translate('item.noteEditor.linkTitle'),
      linkModalDescription: translate('item.noteEditor.linkDescription'),
      linkUrlLabel: translate('item.noteEditor.linkUrlLabel'),
      linkPlaceholder: translate('item.noteEditor.linkPlaceholder'),
      linkCancel: translate('item.noteEditor.linkCancel'),
      linkSubmit: translate('item.noteEditor.linkSubmit'),
    }
  })

  const layoutFilterLabels = $derived.by(() => {
    $currentLocale
    return Object.fromEntries(
      LAYOUT_BLOCK_FILTERS.map((filter) => [filter.id, translate(`item.layoutFilter.${filter.id}`)])
    ) as Record<LayoutBlockFilterId, string>
  })

  // LLM state (Gemma 4)
  const llmStore = new LlmStore({
    onComplete: (id, job, result) => {
      llmTick++
      // Track summary results in the dedicated map
      if (job === 'summarize') {
        summaryTexts.set(id, result)
        summaryTick++
      }
      // When LLM triples complete, reload triples from DB (they're now in the triples table)
      if (job === 'extract_triples') {
        loadTriples()
        nlpStore._setJobStatus(itemId, 'triples', 'done')
        nlpTick++
      }
      if (job === 'correct_ocr') {
        ocrCorrectedAssets.add(id)
        ocrTick++ // Force Svelte reactivity for the textarea
        const assetId = selectedAsset?.id === id ? id : null
        if (assetId) {
          ocrEditedText.set(assetId, result)
          ocrStore.setTextContent(assetId, result)
          schedulePersist(assetId, result)
        } else {
          // Item-level (legacy): update the first asset's text or whichever asset has OCR text
          const asset = assets.find((a: Asset) => ocrStore.getTextContent(a.id))
          if (asset) {
            ocrEditedText.set(asset.id, result)
            ocrStore.setTextContent(asset.id, result)
            schedulePersist(asset.id, result)
          }
        }
      }
    },
    onCorrectOcr: (id, _result) => {
      // Track that OCRC already ran for this asset (from persisted results or live)
      ocrCorrectedAssets.add(id)
    },
    onError: (id, job, error) => {
      // When LLM triples extraction fails, set NLP triples status to error
      if (job === 'extract_triples') {
        nlpStore._setJobStatus(itemId, 'triples', 'error', error)
        nlpTick++
      }
    },
  })
  let llmTick = $state(0)

  // OCRC tracking: once OCRC is done for an asset, hide the button and show
  // only Embedding + Triple buttons in the LLM section.
  let ocrCorrectedAssets = $state(new Set<string>()) // asset IDs that have been OCRC'd

  let llmAvailable = $state(false)
  let summaryTexts = $state(new Map<string, string>()) // assetId → summary text
  let summaryTick = $state(0) // reactivity trigger for summary display

  /**
   * Get the LLM state for the currently active context.
   * When a specific asset/page is selected (multipage), use the asset ID
   * so LLM state is scoped per-page. Otherwise fall back to item ID.
   */
  function getLlmState() {
    void llmTick
    const targetId = selectedAsset ? selectedAsset.id : itemId
    return llmStore.getState(targetId)
  }

  async function handleLlmSummarize() {
    try {
      if (selectedAsset) {
        await llmSummarizeAsset(selectedAsset.id)
      } else {
        await llmSummarize(itemId)
      }
    } catch (e) {
      console.error('[LLM] summarize failed:', e)
    }
  }

  async function handleLlmCorrectOcr() {
    try {
      if (selectedAsset) {
        await llmCorrectOcrAsset(selectedAsset.id)
      } else {
        await llmCorrectOcr(itemId)
      }
    } catch (e) {
      console.error('[LLM] correct OCR failed:', e)
    }
  }

  async function handleLlmExtractTriples() {
    nlpStore._setJobStatus(itemId, 'triples', 'pending')
    nlpTick++
    try {
      if (selectedAsset) {
        await llmExtractTriplesAsset(selectedAsset.id)
      } else {
        await llmExtractTriples(itemId)
      }
    } catch (e) {
      console.error('[LLM] extract triples failed:', e)
      nlpStore._setJobStatus(itemId, 'triples', 'error', e instanceof Error ? e.message : 'Failed')
      nlpTick++
    }
  }

  // Geo state (OpenStreetMap)
  const geoStore = new GeoStore({
    onEntityComplete: () => {
      loadGeoMarkers()
    },
    onItemComplete: () => {
      loadGeoMarkers()
    },
  })
  let geoMarkers = $state<MapMarker[]>([])

  async function loadGeoMarkers() {
    try {
      const rows = await invoke<
        Array<{ id: string; value: string; latitude: number; longitude: number }>
      >('db_select', {
        sql: `SELECT id, value, latitude, longitude FROM entities
              WHERE item_id = ? AND entity_type = 'place' AND geo_status = 'resolved'
              AND latitude IS NOT NULL AND longitude IS NOT NULL`,
        params: [itemId],
      })
      geoMarkers = rows.map((r) => ({
        entityId: r.id,
        label: r.value,
        latitude: r.latitude,
        longitude: r.longitude,
      }))
    } catch (e) {
      console.error('[geo] Failed to load markers:', e)
    }
  }

  let metadataValue = $derived<Record<string, string>>(
    item?.metadata ? parseMetadataRecord(item.metadata) : {}
  )
  let originalFileMetadata = $derived<ImportedFileMetadata | null>(
    item?.metadata ? parseImportedFileMetadata(item.metadata) : null
  )
  let customMetadataNormalizedKeys = $derived(
    new Set(Object.keys(metadataValue).map((key) => normalizeMetadataKey(key)))
  )

  // Topic state
  let itemTopics = $state<string[]>([])
  let topicSuggestions = $state<string[]>([])

  async function loadTopics() {
    try {
      const topics = await getStore().topics.findByItemId(itemId)
      itemTopics = topics.map((t) => t.name)
    } catch (e) {
      console.error('[topics] Failed to load topics:', e)
    }
  }

  async function loadTopicSuggestions() {
    try {
      topicSuggestions = await getStore().topics.allNames()
    } catch (e) {
      console.error('[topics] Failed to load suggestions:', e)
    }
  }

  async function handleTopicsChange(newTopics: string[]) {
    try {
      const store = getStore()
      // Find topics to add (in new but not in current)
      const currentSet = new Set(itemTopics)
      const newSet = new Set(newTopics)
      // Add new topics
      for (const name of newTopics) {
        if (!currentSet.has(name)) {
          await store.topics.addTopicToItem(itemId, name)
        }
      }
      // Remove topics no longer present
      for (const name of itemTopics) {
        if (!newSet.has(name)) {
          const topic = await store.topics.findByName(name)
          if (topic) {
            await store.topics.removeTopicFromItem(itemId, topic.id)
          }
        }
      }
      itemTopics = newTopics.map((t) => t.toUpperCase())
      // Refresh suggestions to include any newly created topics
      void loadTopicSuggestions()
    } catch (e) {
      console.error('[topics] Failed to save topics:', e)
    }
  }

  let selectedAsset = $derived(assets[selectedAssetIndex] ?? null)
  let fileMetadataEntries = $derived(
    buildTechnicalMetadata({
      item,
      selectedAsset,
      collection,
      originalFileMetadata,
      customMetadataKeys: customMetadataNormalizedKeys,
    })
  )

  let viewerSrc = $derived(
    selectedAsset
      ? getAssetUrl(selectedAsset.path) + (imageVersion > 0 ? `?_t=${imageVersion}` : '')
      : ''
  )

  let viewerType = $derived<'image' | 'pdf' | 'audio'>(
    selectedAsset?.type === 'pdf' ? 'pdf' : selectedAsset?.type === 'audio' ? 'audio' : 'image'
  )

  let layoutBlocks = $derived(assetLayout ? buildLayoutBlockViews(assetLayout) : [])
  let layoutPages = $derived(getPagesFromLayout(assetLayout))
  let layoutPageOptions = $derived(
    viewerType === 'pdf' && assetLayout
      ? Array.from(
          { length: Math.max(viewerTotalPages, layoutPages[layoutPages.length - 1] ?? 0) },
          (_, index) => index + 1
        )
      : []
  )
  let layoutActivePage = $derived(viewerType === 'pdf' ? viewerPage : (layoutPages[0] ?? 1))
  let layoutBlockCountsByPage = $derived(getBlockCountByPage(layoutBlocks))
  let layoutPageRegions = $derived(
    assetLayout
      ? viewerType === 'pdf'
        ? filterRegionsByPage(assetLayout.regions, layoutActivePage)
        : assetLayout.regions
      : []
  )
  let layoutPageBlocks = $derived(
    viewerType === 'pdf' ? filterBlocksByPage(layoutBlocks, layoutActivePage) : layoutBlocks
  )
  let layoutFilterCounts = $derived(countLayoutBlocksByFilter(layoutPageBlocks))
  let visibleLayoutBlocks = $derived(filterLayoutBlocksByType(layoutPageBlocks, layoutTypeFilter))
  let selectedLayoutBlock = $derived(findVisibleLayoutBlockById(layoutSelectedBlockId))
  let layoutRegions = $derived<ViewerLayoutRegion[]>(
    visibleLayoutBlocks.map((block) => ({
      id: block.regionId,
      blockId: block.id,
      label: block.label,
      x: block.overlayBbox.x,
      y: block.overlayBbox.y,
      width: block.overlayBbox.width,
      height: block.overlayBbox.height,
      matchSource: block.overlaySource,
    }))
  )
  let layoutReferenceWidth = $derived(
    layoutPageBlocks[0]?.imageWidth ??
      layoutPageRegions[0]?.imageWidth ??
      assetLayout?.imageWidth ??
      0
  )
  let layoutReferenceHeight = $derived(
    layoutPageBlocks[0]?.imageHeight ??
      layoutPageRegions[0]?.imageHeight ??
      assetLayout?.imageHeight ??
      0
  )
  let hasLayoutData = $derived(Boolean(assetLayout && layoutBlocks.length > 0))

  function findVisibleLayoutBlockById(blockId: string | null) {
    if (!blockId) return null
    return visibleLayoutBlocks.find((block) => block.id === blockId) ?? null
  }

  function findVisibleLayoutBlockByRegionId(regionId: string | null) {
    if (!regionId) return null
    return visibleLayoutBlocks.find((block) => block.regionId === regionId) ?? null
  }

  function syncLayoutHoverFromBlock(blockId: string | null) {
    const block = findVisibleLayoutBlockById(blockId)
    layoutHoveredBlockId = block?.id ?? null
    layoutHoveredRegionId = block?.regionId ?? null
  }

  function syncLayoutHoverFromRegion(regionId: string | null) {
    const block = findVisibleLayoutBlockByRegionId(regionId)
    layoutHoveredBlockId = block?.id ?? null
    layoutHoveredRegionId = block?.regionId ?? null
  }

  function setSelectedLayoutBlock(blockId: string | null) {
    const block = findVisibleLayoutBlockById(blockId)
    layoutSelectedBlockId = block?.id ?? null
    layoutSelectedRegionId = block?.regionId ?? null
    if (block) {
      showLayout = true
    }
  }

  function setSelectedLayoutRegion(regionId: string | null) {
    const block = findVisibleLayoutBlockByRegionId(regionId)
    layoutSelectedBlockId = block?.id ?? null
    layoutSelectedRegionId = block?.regionId ?? null
    if (block) {
      showLayout = true
    }
  }

  function scrollSelectedLayoutBlockIntoView(blockId: string | null) {
    if (!layoutBlockListEl || !blockId) return
    const selector = `[data-layout-block-id="${blockId}"]`
    const blockEl = layoutBlockListEl.querySelector<HTMLElement>(selector)
    blockEl?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
  }

  function clearLayoutInspectorCopyMessage() {
    if (layoutInspectorCopyTimer) {
      clearTimeout(layoutInspectorCopyTimer)
      layoutInspectorCopyTimer = null
    }
  }

  function showLayoutInspectorCopyMessage(tone: 'success' | 'error', text: string) {
    clearLayoutInspectorCopyMessage()
    layoutInspectorCopyMessage = { tone, text }
    layoutInspectorCopyTimer = setTimeout(() => {
      layoutInspectorCopyMessage = null
      layoutInspectorCopyTimer = null
    }, 2200)
  }

  async function copyLayoutInspectorValue(value: string, successText: string) {
    try {
      if (!navigator?.clipboard?.writeText) {
        throw new Error('Clipboard API unavailable')
      }

      await navigator.clipboard.writeText(value)
      showLayoutInspectorCopyMessage('success', successText)
    } catch {
      showLayoutInspectorCopyMessage('error', 'No se pudo copiar desde el inspector.')
    }
  }

  function clampNormalized(value: number) {
    return Math.max(0, Math.min(1, value))
  }

  function normalizeAnnotationsForAsset(
    asset: Asset,
    nextAnnotations: ViewerAnnotation[]
  ): ViewerAnnotation[] {
    return nextAnnotations.map((annotation) => {
      const now = Date.now()
      return {
        ...annotation,
        id: annotation.id || crypto.randomUUID(),
        assetId: asset.id,
        page: 1,
        color: annotation.color,
        x: clampNormalized(annotation.x),
        y: clampNormalized(annotation.y),
        width: clampNormalized(annotation.width),
        height: clampNormalized(annotation.height),
        createdAt: annotation.createdAt || now,
        updatedAt: now,
      }
    })
  }

  async function persistAnnotations(assetId: string, nextAnnotations: ViewerAnnotation[]) {
    try {
      const inputs = nextAnnotations.map((a) => ({
        kind: a.kind as StoreAnnotationKind,
        color: a.color,
        x: a.x,
        y: a.y,
        width: a.width,
        height: a.height,
      }))
      await getStore().annotations.replaceForAssetPage(assetId, 1, inputs)
      annotationSaveError = null
    } catch {
      annotationSaveError = 'Failed to save annotations. Changes remain local until retry.'
    }
  }

  function clearAnnotationSaveTimer() {
    if (annotationSaveTimer) {
      clearTimeout(annotationSaveTimer)
      annotationSaveTimer = null
    }
  }

  async function flushPendingAnnotationSave() {
    clearAnnotationSaveTimer()

    if (!pendingAnnotationSave) {
      return
    }

    const saveJob = pendingAnnotationSave
    pendingAnnotationSave = null
    await persistAnnotations(saveJob.assetId, saveJob.annotations)
  }

  function scheduleAnnotationPersist(assetId: string, nextAnnotations: ViewerAnnotation[]) {
    clearAnnotationSaveTimer()
    pendingAnnotationSave = {
      assetId,
      annotations: nextAnnotations,
    }

    annotationSaveTimer = setTimeout(async () => {
      const saveJob = pendingAnnotationSave
      pendingAnnotationSave = null
      annotationSaveTimer = null

      if (!saveJob) {
        return
      }

      await persistAnnotations(saveJob.assetId, saveJob.annotations)
    }, 500)
  }

  function handleAnnotationsChange(nextAnnotations: ViewerAnnotation[]) {
    if (!selectedAsset || selectedAsset.type !== 'image') {
      return
    }

    annotations = normalizeAnnotationsForAsset(selectedAsset, nextAnnotations)
    annotationSaveError = null
    scheduleAnnotationPersist(selectedAsset.id, annotations)
  }

  function handleSelectedAnnotationIdChange(annotationId: string | null) {
    selectedAnnotationId = annotationId
  }

  function handleAnnotationToolChange(tool: 'select' | 'rectangle' | 'underline') {
    annotationTool = tool
  }

  function handleAnnotationColorChange(color: string) {
    annotationColor = color
  }

  // ── Image editing handlers ────────────────────────────────────────────

  /** Convert normalized (0-1) region to pixel coordinates based on image dimensions */
  function normalizedToPixels(
    region: { x: number; y: number; width: number; height: number },
    naturalW: number,
    naturalH: number
  ) {
    return {
      x: Math.round(region.x * naturalW),
      y: Math.round(region.y * naturalH),
      width: Math.round(region.width * naturalW),
      height: Math.round(region.height * naturalH),
    }
  }

  /** Adjust annotations after a rotation. Converted = new image dimensions. */
  function adjustAnnotationsAfterRotation(rotation: 'left' | 'right') {
    annotations = annotations.map((a) => {
      if (rotation === 'right') {
        // 90° CW: new_x = 1 - old_y - old_height, new_y = old_x
        const nx = 1 - a.y - a.height
        const ny = a.x
        return { ...a, x: nx, y: ny, width: a.height, height: a.width }
      } else {
        // 90° CCW: new_x = old_y, new_y = 1 - old_x - old_width
        const nx = a.y
        const ny = 1 - a.x - a.width
        return { ...a, x: nx, y: ny, width: a.height, height: a.width }
      }
    })
  }

  /** Adjust annotations after a crop. Region is the crop area in normalized coords. */
  function adjustAnnotationsAfterCrop(region: {
    x: number
    y: number
    width: number
    height: number
  }) {
    const { x: cx, y: cy, width: cw, height: ch } = region
    annotations = annotations
      .filter((a) => {
        // Keep annotations that overlap with the crop region
        const overlapsX = a.x < cx + cw && a.x + a.width > cx
        const overlapsY = a.y < cy + ch && a.y + a.height > cy
        return overlapsX && overlapsY
      })
      .map((a) => {
        // Clamp to crop region
        const clampedX = Math.max(a.x, cx)
        const clampedY = Math.max(a.y, cy)
        const clampedRight = Math.min(a.x + a.width, cx + cw)
        const clampedBottom = Math.min(a.y + a.height, cy + ch)
        const newWidth = clampedRight - clampedX
        const newHeight = clampedBottom - clampedY
        return {
          ...a,
          x: (clampedX - cx) / cw,
          y: (clampedY - cy) / ch,
          width: newWidth / cw,
          height: newHeight / ch,
        }
      })
  }

  async function handleEditSelect(region: { x: number; y: number; width: number; height: number }) {
    if (!selectedAsset || selectedAsset.type !== 'image') return
    if (imageNaturalW === 0 || imageNaturalH === 0) return

    await flushPendingAnnotationSave()

    const asset = selectedAsset
    const pixelRegion = normalizedToPixels(region, imageNaturalW, imageNaturalH)

    // Push current state onto undo stack before performing the edit
    undoStack = [
      ...undoStack,
      {
        path: asset.path,
        width: imageNaturalW,
        height: imageNaturalH,
        annotations: JSON.parse(JSON.stringify(annotations)),
      },
    ]

    try {
      if (editTool === 'crop') {
        const result: ImageEditResult = await invoke('crop_image', {
          path: asset.path,
          x: pixelRegion.x,
          y: pixelRegion.y,
          width: pixelRegion.width,
          height: pixelRegion.height,
        })
        adjustAnnotationsAfterCrop(region)
        await handleImageEditResult(result, asset.id)
      } else if (editTool === 'erase') {
        const result: ImageEditResult = await invoke('erase_region', {
          path: asset.path,
          x: pixelRegion.x,
          y: pixelRegion.y,
          width: pixelRegion.width,
          height: pixelRegion.height,
          fill: 'white',
        })
        await handleImageEditResult(result, asset.id)
      }
    } catch (e) {
      // On failure, pop the undo entry since the edit didn't succeed
      undoStack = undoStack.slice(0, -1)
      console.error('[ItemView] Image edit failed:', e)
    } finally {
      // Reset edit tool after operation
      editTool = 'none'
    }
  }

  async function handleRotateLeft() {
    if (!selectedAsset || selectedAsset.type !== 'image') return
    await flushPendingAnnotationSave()
    const asset = selectedAsset

    // Push current state onto undo stack before rotating
    undoStack = [
      ...undoStack,
      {
        path: asset.path,
        width: imageNaturalW,
        height: imageNaturalH,
        annotations: JSON.parse(JSON.stringify(annotations)),
      },
    ]

    try {
      const result: ImageEditResult = await invoke('rotate_image', {
        path: asset.path,
        direction: 'left',
      })
      adjustAnnotationsAfterRotation('left')
      await handleImageEditResult(result, asset.id)
    } catch (e) {
      undoStack = undoStack.slice(0, -1)
      console.error('[ItemView] Rotate left failed:', e)
    }
  }

  async function handleRotateRight() {
    if (!selectedAsset || selectedAsset.type !== 'image') return
    await flushPendingAnnotationSave()
    const asset = selectedAsset

    // Push current state onto undo stack before rotating
    undoStack = [
      ...undoStack,
      {
        path: asset.path,
        width: imageNaturalW,
        height: imageNaturalH,
        annotations: JSON.parse(JSON.stringify(annotations)),
      },
    ]

    try {
      const result: ImageEditResult = await invoke('rotate_image', {
        path: asset.path,
        direction: 'right',
      })
      adjustAnnotationsAfterRotation('right')
      await handleImageEditResult(result, asset.id)
    } catch (e) {
      undoStack = undoStack.slice(0, -1)
      console.error('[ItemView] Rotate right failed:', e)
    }
  }

  /** Undo the last image edit: restore the asset path, dimensions,
   *  and annotations to the previous state. */
  async function handleUndo() {
    if (!selectedAsset || selectedAsset.type !== 'image') return
    if (undoStack.length === 0) return

    await flushPendingAnnotationSave()

    const entry = undoStack[undoStack.length - 1]!
    const assetId = selectedAsset.id

    // Restore state from undo entry
    const store = getStore()
    await store.assets.updatePath(assetId, entry.path)
    assets = assets.map((a) => (a.id === assetId ? { ...a, path: entry.path } : a))
    annotations = entry.annotations
    selectedAnnotationId = null
    // Force image refresh
    imageVersion++

    // Persist the restored annotations
    await persistAnnotations(assetId, annotations)

    // Pop the undo stack
    undoStack = undoStack.slice(0, -1)

    // Notify other views
    try {
      await emit('asset:image-updated', {
        itemId,
        assetId,
        path: entry.path,
      })
    } catch (e) {
      console.warn('[ItemView] Failed to emit asset:image-updated event on undo:', e)
    }
  }

  /** Post-edit: always update asset path in DB (even if format didn't change),
   *  refresh image, persist annotations, push undo entry, and notify other views. */
  async function handleImageEditResult(result: ImageEditResult, assetId: string) {
    // Always update the asset path in DB — versioned paths change on every edit,
    // and the DB must reflect the current file on disk.
    const store = getStore()
    await store.assets.updatePath(assetId, result.path)
    // Update the local assets array with the new path
    assets = assets.map((a) => (a.id === assetId ? { ...a, path: result.path } : a))

    // Force image refresh: bump version counter so the browser fetches the
    // new file (versioned paths already make the URL unique, but this helps
    // if something caches at the protocol level).
    imageVersion++

    // Persist adjusted annotations
    if (selectedAsset && selectedAsset.id === assetId) {
      await persistAnnotations(assetId, annotations)
    }

    // Notify CollectionView (and any other listeners) that the asset path
    // has changed, so they can invalidate their cached thumbnail URLs.
    try {
      await emit('asset:image-updated', {
        itemId,
        assetId,
        path: result.path,
      })
    } catch (e) {
      console.warn('[ItemView] Failed to emit asset:image-updated event:', e)
    }
  }

  // Track natural image dimensions for pixel coordinate conversion
  let imageNaturalW = $state(0)
  let imageNaturalH = $state(0)

  function parseMetadataRecord(json: string): Record<string, string> {
    try {
      const obj = JSON.parse(json)
      const record: Record<string, string> = {}
      for (const [key, value] of Object.entries(obj)) {
        if (key === IMPORTED_FILE_METADATA_KEY) continue
        record[key] = String(value)
      }
      return record
    } catch {
      return {}
    }
  }

  function parseImportedFileMetadata(json: string): ImportedFileMetadata | null {
    try {
      const obj = JSON.parse(json) as Record<string, unknown>
      const metadata = obj[IMPORTED_FILE_METADATA_KEY]
      return metadata && typeof metadata === 'object' ? (metadata as ImportedFileMetadata) : null
    } catch {
      return null
    }
  }

  function mergeReservedMetadata(metadata: Record<string, string>): Record<string, unknown> {
    const reserved = item?.metadata ? parseImportedFileMetadata(item.metadata) : null
    return reserved ? { ...metadata, [IMPORTED_FILE_METADATA_KEY]: reserved } : metadata
  }

  let metadataSaveTimer: ReturnType<typeof setTimeout> | null = null

  async function handleExtractText(asset: Asset, mode: OcrMode = 'light') {
    ocrStore._updateState(asset.id, { status: 'pending', progress: 0 })
    ocrTick++
    try {
      await extractText(asset.id, asset.path, asset.type, mode)
    } catch (e) {
      ocrStore._updateState(asset.id, {
        status: 'error',
        error: e instanceof Error ? e.message : 'Extraction failed',
      })
      ocrTick++
    }
  }

  async function handleTranscribeAudio(asset: Asset) {
    transcriptionStore._updateState(asset.id, { status: 'pending', progress: 0 })
    transcriptionTick++
    try {
      await transcribeAudio(asset.id, asset.path)
    } catch (e) {
      transcriptionStore._updateState(asset.id, {
        status: 'error',
        error: e instanceof Error ? e.message : 'Transcription failed',
      })
      transcriptionTick++
    }
  }

  async function handleTranscribeDictation(audio: Blob): Promise<string> {
    return transcribeDictation(audio)
  }

  function getExtractionPrimaryActionLabel(assetType: Asset['type']) {
    if (assetType === 'pdf') {
      return translate('item.pdfTextAction')
    }

    return translate('item.ocrFastAction')
  }

  function getCorrectionActionLabel(assetType: Asset['type']) {
    return assetType === 'pdf'
      ? translate('item.pdfCorrectAction')
      : translate('item.ocrCorrectAction')
  }

  function getSummaryActionLabel(assetType: Asset['type']) {
    if (assetType === 'pdf') {
      return translate('item.summaryPdfAction')
    }

    if (assetType === 'audio') {
      return translate('item.summaryAudioAction')
    }

    return translate('item.summaryAction')
  }

  function getTranscriptionActionLabel(busy: boolean) {
    return busy ? translate('item.transcribeBusyAction') : translate('item.transcribeShortAction')
  }

  function getTranscriptionStageLabel(stage?: string) {
    if (!stage) return ''

    switch (stage) {
      case 'uploading':
        return translate('item.transcriptionStage.uploading')
      case 'submitting_remote':
        return translate('item.transcriptionStage.submitting_remote')
      case 'polling_remote':
        return translate('item.transcriptionStage.polling_remote')
      default:
        return ''
    }
  }

  function getOcrStageLabel(stage?: string) {
    if (!stage || stage === 'done' || stage === 'error') return ''

    switch (stage) {
      case 'reading':
        return translate('item.ocrStage.reading')
      case 'extracting_native':
        return translate('item.ocrStage.extracting_native')
      case 'ocr_inference':
        return translate('item.ocrStage.ocr_inference')
      case 'paddlevl_detection':
        return translate('item.ocrStage.paddlevl_detection')
      case 'submitting_glm_ocr':
        return translate('item.ocrStage.submitting_glm_ocr')
      case 'waiting_glm_ocr':
        return translate('item.ocrStage.waiting_glm_ocr')
      case 'parsing_glm_ocr':
        return translate('item.ocrStage.parsing_glm_ocr')
      default:
        return ''
    }
  }

  function getOcrState(assetId: string) {
    // Depend on ocrTick to trigger Svelte reactivity when events arrive
    void ocrTick
    return ocrStore.getState(assetId)
  }

  function getTranscriptionState(assetId: string) {
    void transcriptionTick
    return transcriptionStore.getState(assetId)
  }

  function getNlpState() {
    void nlpTick
    return nlpStore.getState(itemId)
  }

  async function handleIndexFts() {
    nlpStore._setJobStatus(itemId, 'fts', 'pending')
    nlpTick++
    try {
      await indexFts(itemId)
    } catch (e) {
      nlpStore._setJobStatus(itemId, 'fts', 'error', e instanceof Error ? e.message : 'Failed')
      nlpTick++
    }
  }

  function getAssetPathLabel(path: string) {
    return path.split(/[/\\]/).pop() ?? path
  }

  function getAssetTypeLabel(assetType: string) {
    return assetType ? assetType.toUpperCase() : 'ASSET'
  }

  function normalizeMetadataKey(key: string) {
    return key
      .trim()
      .toLowerCase()
      .normalize('NFD')
      .replace(/[\u0300-\u036f]/g, '')
      .replace(/[^a-z0-9]+/g, ' ')
      .trim()
  }

  function getFileExtension(fileName: string): string | null {
    const index = fileName.lastIndexOf('.')
    if (index <= 0 || index === fileName.length - 1) return null
    return fileName.slice(index).toLowerCase()
  }

  function formatBytes(size: number | null | undefined): string | null {
    if (size === null || size === undefined || !Number.isFinite(size) || size < 0) return null
    if (size < 1024) return `${size} B`

    const units = ['KB', 'MB', 'GB', 'TB']
    let value = size / 1024
    let unitIndex = 0

    while (value >= 1024 && unitIndex < units.length - 1) {
      value /= 1024
      unitIndex++
    }

    const digits = value >= 10 ? 0 : 1
    return `${value.toFixed(digits)} ${units[unitIndex]}`
  }

  function formatTimestamp(timestamp: number | string | null | undefined): string | null {
    if (timestamp === null || timestamp === undefined) return null
    const millis = typeof timestamp === 'string' ? Date.parse(timestamp) : timestamp
    if (!Number.isFinite(millis)) return null
    return new Date(millis).toLocaleString()
  }

  function formatBoolean(value: boolean | null | undefined): string | null {
    if (value === null || value === undefined) return null
    return value ? 'Sí' : 'No'
  }

  function pushTechnicalMetadataEntry(
    entries: Array<{ label: string; value: string }>,
    customMetadataKeys: Set<string>,
    label: string,
    value: string | null | undefined,
    aliases: string[] = []
  ) {
    if (!value) return

    const normalizedCandidates = [label, ...aliases].map((candidate) =>
      normalizeMetadataKey(candidate)
    )
    if (normalizedCandidates.some((candidate) => customMetadataKeys.has(candidate))) {
      return
    }

    entries.push({ label, value })
  }

  function buildTechnicalMetadata({
    item,
    selectedAsset,
    collection,
    originalFileMetadata,
    customMetadataKeys,
  }: {
    item: Item | null
    selectedAsset: Asset | null
    collection: Collection | null
    originalFileMetadata: ImportedFileMetadata | null
    customMetadataKeys: Set<string>
  }) {
    const entries: Array<{ label: string; value: string }> = []
    const fileName = selectedAsset ? getAssetPathLabel(selectedAsset.path) : null
    const extension = fileName ? getFileExtension(fileName) : null

    pushTechnicalMetadataEntry(entries, customMetadataKeys, 'Nombre del archivo', fileName, [
      'archivo',
      'nombre archivo',
      'file name',
    ])
    pushTechnicalMetadataEntry(
      entries,
      customMetadataKeys,
      'Tipo de archivo',
      selectedAsset?.type ? getAssetTypeLabel(selectedAsset.type) : null,
      ['tipo', 'tipo archivo', 'file type', 'mime', 'mime type']
    )
    pushTechnicalMetadataEntry(entries, customMetadataKeys, 'Extensión', extension, [
      'extension',
      'ext',
    ])
    pushTechnicalMetadataEntry(
      entries,
      customMetadataKeys,
      'Tamaño',
      formatBytes(selectedAsset?.size),
      ['tamano', 'tamaño archivo', 'file size', 'size']
    )
    pushTechnicalMetadataEntry(entries, customMetadataKeys, 'Documento ID', item?.id ?? null, [
      'documento id',
      'document id',
      'item id',
      'id',
    ])
    pushTechnicalMetadataEntry(entries, customMetadataKeys, 'Asset ID', selectedAsset?.id ?? null, [
      'asset id',
      'archivo id',
    ])
    pushTechnicalMetadataEntry(
      entries,
      customMetadataKeys,
      'Ruta interna',
      selectedAsset?.path ?? null,
      ['ruta interna', 'internal path', 'path']
    )
    pushTechnicalMetadataEntry(entries, customMetadataKeys, 'Colección', collection?.name ?? null, [
      'coleccion',
      'collection',
      'project',
      'proyecto',
    ])

    pushTechnicalMetadataEntry(
      entries,
      customMetadataKeys,
      'Nombre original',
      originalFileMetadata?.originalName,
      ['original name', 'nombre fuente']
    )
    pushTechnicalMetadataEntry(
      entries,
      customMetadataKeys,
      'Ruta original',
      originalFileMetadata?.originalPath,
      ['source path', 'ruta fuente']
    )
    pushTechnicalMetadataEntry(
      entries,
      customMetadataKeys,
      'Tamaño original',
      formatBytes(originalFileMetadata?.sizeBytes),
      ['original size', 'source size']
    )
    pushTechnicalMetadataEntry(
      entries,
      customMetadataKeys,
      'Importado el',
      formatTimestamp(originalFileMetadata?.importedAt),
      ['imported at', 'fecha importacion']
    )
    pushTechnicalMetadataEntry(
      entries,
      customMetadataKeys,
      'Creado en origen',
      formatTimestamp(originalFileMetadata?.createdAt),
      ['created at', 'fecha creacion origen']
    )
    pushTechnicalMetadataEntry(
      entries,
      customMetadataKeys,
      'Modificado en origen',
      formatTimestamp(originalFileMetadata?.modifiedAt),
      ['modified at', 'fecha modificacion origen']
    )
    pushTechnicalMetadataEntry(
      entries,
      customMetadataKeys,
      'Solo lectura',
      formatBoolean(originalFileMetadata?.readonly),
      ['readonly', 'read only']
    )

    return entries
  }

  let activeAssetSummary = $derived(
    selectedAsset
      ? `${getAssetTypeLabel(selectedAsset.type)} · ${getAssetPathLabel(selectedAsset.path)}`
      : 'Sin asset seleccionado'
  )

  async function handleEmbedAsset() {
    if (!selectedAsset) {
      nlpStore._setJobStatus(
        itemId,
        'embed',
        'error',
        'Select an asset before generating embeddings.'
      )
      nlpTick++
      return
    }

    nlpStore._setJobStatus(itemId, 'embed', 'pending')
    nlpTick++
    try {
      await embedAsset(itemId, selectedAsset.id)
    } catch (e) {
      nlpStore._setJobStatus(itemId, 'embed', 'error', e instanceof Error ? e.message : 'Failed')
      nlpTick++
    }
  }

  async function handleExtractEntities() {
    nlpStore._setJobStatus(itemId, 'ner', 'pending')
    nlpTick++
    try {
      if (selectedAsset) {
        await extractEntitiesForAsset(itemId, selectedAsset.id)
      } else {
        await extractEntities(itemId)
      }
    } catch (e) {
      nlpStore._setJobStatus(itemId, 'ner', 'error', e instanceof Error ? e.message : 'Failed')
      nlpTick++
    }
  }

  async function loadEntities() {
    try {
      const store = getStore()
      if (selectedAsset) {
        entities = (
          (await store.entities.findByAssetId(itemId, selectedAsset.id)) as Entity[]
        ).filter((entity) => entity.confidence == null || entity.confidence > 0.89)
      } else {
        entities = ((await store.entities.findByItemId(itemId)) as Entity[]).filter(
          (entity) => entity.confidence == null || entity.confidence > 0.89
        )
      }
    } catch {
      // Non-fatal: entities panel shows empty state
    }
  }

  function normalizeManualEntityValue(value: string) {
    return value
      .trim()
      .replace(/^["'“”‘’«»\-–—\s]+|["'“”‘’«»\-–—\s]+$/g, '')
      .trim()
  }

  function toEditableEntityType(entityType: Entity['entityType']): EditableEntityType {
    if (
      entityType === 'person' ||
      entityType === 'organization' ||
      entityType === 'place' ||
      entityType === 'misc' ||
      entityType === 'date'
    ) {
      return entityType
    }
    return 'organization'
  }

  async function handleCreateEntity() {
    const value = normalizeManualEntityValue(newEntityValue)
    if (!value) return
    try {
      await getStore().entities.create({
        itemId,
        assetId: selectedAsset?.id ?? null,
        entityType: newEntityType,
        value,
        startOffset: 0,
        endOffset: 0,
        confidence: 1.0,
        source: 'manual',
        modelName: null,
        createdAt: Date.now(),
      })
      newEntityValue = ''
      newEntityType = 'organization'
      entityActionError = null
      await loadEntities()
    } catch (e) {
      entityActionError = e instanceof Error ? e.message : 'Failed to add entity'
    }
  }

  function startEditingEntity(entity: Entity) {
    editingEntityId = entity.id
    editingEntityValue = entity.value
    entityActionError = null
  }

  function cancelEditingEntity() {
    editingEntityId = null
    editingEntityValue = ''
  }

  function handleEditingEntityValueChange(value: string) {
    editingEntityValue = value
  }

  async function handleSaveEntity(entityId: string, nextValue = editingEntityValue) {
    const value = normalizeManualEntityValue(nextValue)
    if (!value) return
    const entity = entities.find((candidate) => candidate.id === entityId)
    if (!entity) return
    try {
      await getStore().entities.update(entityId, {
        entityType: toEditableEntityType(entity.entityType),
        value,
        confidence: 1.0,
        source: 'manual',
      })
      cancelEditingEntity()
      entityActionError = null
      await loadEntities()
    } catch (e) {
      entityActionError = e instanceof Error ? e.message : 'Failed to save entity'
    }
  }

  async function handleDeleteEntity(entityId: string) {
    try {
      await getStore().entities.delete(entityId)
      if (editingEntityId === entityId) {
        cancelEditingEntity()
      }
      entityActionError = null
      await loadEntities()
    } catch (e) {
      entityActionError = e instanceof Error ? e.message : 'Failed to delete entity'
    }
  }

  async function loadSimilarAssets() {
    if (!selectedAsset) {
      similarAssets = []
      return
    }

    try {
      similarAssets = await fetchSimilarAssets(selectedAsset.id, 5)
    } catch {
      similarAssets = []
    }
  }

  function navigateToSimilarItem(item: { itemId: string; title: string; collectionId: string }) {
    navigation.replace({
      name: 'item',
      itemId: item.itemId,
      collectionId: item.collectionId,
      collectionName: '',
      itemTitle: item.title || item.itemId,
    })
  }

  function clearFtsSearchTimer() {
    if (ftsSearchTimer) {
      clearTimeout(ftsSearchTimer)
      ftsSearchTimer = null
    }
  }

  function escapeRegExp(value: string): string {
    return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  }

  function getFtsTerms(rawQuery: string): string[] {
    if (!rawQuery.trim()) return []

    const noOperators = rawQuery.replace(/\b(AND|OR|NOT|NEAR)\b/gi, ' ')
    const terms = noOperators
      .split(/\s+/)
      .map((token) => token.replace(/[()"\-*^:,./\\]/g, '').trim())
      .filter((token) => token.length > 0)

    return Array.from(new Set(terms.map((token) => token.toLocaleLowerCase())))
  }

  function splitHighlightedSegments(text: string, rawQuery: string) {
    const terms = getFtsTerms(rawQuery)
    if (terms.length === 0 || !text) return [{ text, isMatch: false }]

    const pattern = terms
      .slice()
      .sort((a, b) => b.length - a.length)
      .map((term) => escapeRegExp(term))
      .join('|')

    if (!pattern) return [{ text, isMatch: false }]

    const regex = new RegExp(pattern, 'gi')
    const segments: Array<{ text: string; isMatch: boolean }> = []
    let lastIndex = 0

    for (const match of text.matchAll(regex)) {
      const index = match.index ?? 0
      const value = match[0] ?? ''
      if (index > lastIndex) {
        segments.push({ text: text.slice(lastIndex, index), isMatch: false })
      }
      if (value) {
        segments.push({ text: value, isMatch: true })
      }
      lastIndex = index + value.length
    }

    if (lastIndex < text.length) {
      segments.push({ text: text.slice(lastIndex), isMatch: false })
    }

    return segments.length > 0 ? segments : [{ text, isMatch: false }]
  }

  async function runFtsSearch(rawQuery: string) {
    const query = rawQuery.trim()
    if (!query) {
      ftsResults = []
      ftsSearchError = null
      ftsSearching = false
      ftsDebug = null
      return
    }

    ftsSearching = true
    ftsSearchError = null

    try {
      const store = getStore()
      if (isDev) {
        const stats = await store.fts.stats()
        ftsIndexedRows = stats.totalRows
      }

      const response = await store.fts.searchWithDebug(query, 10)
      const rows = response.results

      const hydrated = await Promise.all(
        rows.map(async (row) => {
          const found = await store.items.findById(row.itemId)
          if (!found) return null

          return {
            itemId: found.id,
            title: found.title,
            rank: row.rank,
            collectionId: found.collectionId,
          }
        })
      )

      ftsResults = hydrated.filter(
        (row): row is { itemId: string; title: string; rank: number; collectionId: string } => !!row
      )

      if (isDev) {
        ftsDebug = {
          ...response.debug,
          hydratedCount: ftsResults.length,
        }
      }
    } catch {
      ftsResults = []
      ftsSearchError = 'No se pudo ejecutar la búsqueda full-text.'
      if (isDev) {
        ftsDebug = null
      }
    } finally {
      ftsSearching = false
    }
  }

  async function loadFtsStats() {
    if (!isDev) return

    try {
      const store = getStore()
      const stats = await store.fts.stats()
      ftsIndexedRows = stats.totalRows
    } catch {
      ftsIndexedRows = null
    }
  }

  function handleFtsInput(event: Event) {
    const value = (event.currentTarget as HTMLInputElement).value
    ftsQuery = value

    clearFtsSearchTimer()
    if (!value.trim()) {
      ftsResults = []
      ftsSearchError = null
      ftsSearching = false
      ftsDebug = null
      return
    }

    ftsSearchTimer = setTimeout(() => {
      void runFtsSearch(value)
    }, 250)
  }

  function handleFtsKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter') {
      event.preventDefault()
      clearFtsSearchTimer()
      void runFtsSearch(ftsQuery)
      return
    }

    if (event.key === 'Escape') {
      event.preventDefault()
      clearFtsSearchTimer()
      ftsQuery = ''
      ftsResults = []
      ftsSearchError = null
      ftsSearching = false
      ftsDebug = null
    }
  }

  async function loadTriples() {
    try {
      const store = getStore()
      if (selectedAsset) {
        triples = await store.triples.findByAssetId(itemId, selectedAsset.id)
      } else {
        triples = await store.triples.findByItemId(itemId)
      }
    } catch {
      triples = []
    }
  }

  async function reloadSelectedAssetPersistedState(options: {
    layout?: boolean
    entities?: boolean
    triples?: boolean
    similarAssets?: boolean
  }) {
    const asset = selectedAsset
    if (!asset) return

    const reloads: Promise<unknown>[] = []

    if (options.layout && asset.type !== 'audio') {
      reloads.push(reloadLayoutForAsset(asset))
    }
    if (options.entities) {
      reloads.push(loadEntities())
    }
    if (options.triples) {
      reloads.push(loadTriples())
    }
    if (options.similarAssets) {
      reloads.push(loadSimilarAssets())
    }

    await Promise.allSettled(reloads)
  }

  function handleMetadataChange(metadata: Record<string, string>) {
    if (metadataSaveTimer) clearTimeout(metadataSaveTimer)
    metadataSaveTimer = setTimeout(async () => {
      if (!item) return
      try {
        savingMetadata = true
        const store = getStore()
        await store.items.update(item.id, { metadata: JSON.stringify(mergeReservedMetadata(metadata)) })
      } catch (e) {
        error = e instanceof Error ? e.message : 'Failed to save metadata'
      } finally {
        savingMetadata = false
      }
    }, 1000)
  }

  async function handleSaveNote(content: string) {
    try {
      error = null
      const store = getStore()
      await store.notes.create({ itemId, assetId: selectedAsset?.id ?? null, content })
      notes = await loadNotesForAsset()
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to save note'
    }
  }

  let pendingDeleteNoteId = $state<string | null>(null)
  let deletingNote = $state(false)
  let deleteNoteDialogEl: HTMLElement | undefined = $state()
  let deleteNoteTriggerEl: HTMLElement | null = null

  async function handleDeleteNote(noteId: string) {
    try {
      error = null
      deletingNote = true
      const store = getStore()
      await store.notes.delete(noteId)
      notes = await loadNotesForAsset()
      if (expandedNoteId === noteId) {
        expandedNoteId = null
      }
      if (editingNoteId === noteId) {
        editingNoteId = null
      }
      pendingDeleteNoteId = null
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to delete note'
    } finally {
      deletingNote = false
    }
  }

  // Note editing state
  let editingNoteId = $state<string | null>(null)
  let expandedNoteId = $state<string | null>(null)

  function openDeleteNoteConfirm(noteId: string) {
    deleteNoteTriggerEl = document.activeElement instanceof HTMLElement ? document.activeElement : null
    pendingDeleteNoteId = noteId
  }

  function handleDeleteNoteCancel() {
    if (deletingNote) return
    pendingDeleteNoteId = null
    deleteNoteTriggerEl?.focus()
    deleteNoteTriggerEl = null
  }

  async function handleDeleteNoteConfirm() {
    if (!pendingDeleteNoteId || deletingNote) return
    await handleDeleteNote(pendingDeleteNoteId)
    deleteNoteTriggerEl = null
  }

  function handleDeleteNoteDialogKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault()
      handleDeleteNoteCancel()
      return
    }

    if (event.key !== 'Tab') return

    const target = getNextFocusTrapTarget(
      getFocusableElements(deleteNoteDialogEl ?? null),
      event.target instanceof HTMLElement ? event.target : null,
      event.shiftKey,
      deleteNoteDialogEl ?? null
    )

    if (target) {
      event.preventDefault()
      target.focus()
    }
  }

  $effect(() => {
    if (!pendingDeleteNoteId || !deleteNoteDialogEl) return

    setTimeout(() => {
      getFocusableElements(deleteNoteDialogEl ?? null)[0]?.focus()
    }, 0)
  })

  function handleEditNote(note: Note) {
    editingNoteId = note.id
  }

  function toggleNoteExpanded(noteId: string) {
    expandedNoteId = expandedNoteId === noteId ? null : noteId
  }

  function handleNoteRowClick(noteId: string, event: MouseEvent) {
    const target = event.target
    if (target instanceof Element && target.closest('a, button')) {
      return
    }
    toggleNoteExpanded(noteId)
  }

  function handleNoteRowKeydown(noteId: string, event: KeyboardEvent) {
    if (event.key !== 'Enter' && event.key !== ' ') return
    event.preventDefault()
    toggleNoteExpanded(noteId)
  }

  async function handleSaveEdit(noteId: string, content: string) {
    if (isNoteHtmlEffectivelyEmpty(content)) return
    try {
      error = null
      const store = getStore()
      await store.notes.update(noteId, content)
      notes = await loadNotesForAsset()
      editingNoteId = null
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to update note'
    }
  }

  function handleCancelEdit() {
    editingNoteId = null
  }

  function getRenderedNoteContent(content: string): string {
    return normalizeNoteContentForRender(content)
  }

  async function handleExpandedNoteContentClick(event: MouseEvent) {
    const target = event.target
    if (!(target instanceof Element)) return

    const link = target.closest('a')
    if (!(link instanceof HTMLAnchorElement)) return

    const url = normalizeNoteLinkHref(link.getAttribute('href') ?? link.href)
    if (!url) return

    event.preventDefault()
    event.stopPropagation()

    try {
      await invoke('open_external_url', { url })
    } catch (error) {
      console.error(`[ItemView] ${translate('item.noteOpenLinkError')}`, error)
    }
  }

  function expandedNoteContentLinkHandler(node: HTMLElement) {
    const handleClick = (event: MouseEvent) => {
      void handleExpandedNoteContentClick(event)
    }

    node.addEventListener('click', handleClick)

    return {
      destroy() {
        node.removeEventListener('click', handleClick)
      },
    }
  }

  function getPlainTextFromNote(content: string): string {
    const rendered = getRenderedNoteContent(content)
    if (!rendered) return ''

    const withSeparators = rendered
      .replace(/<br\s*\/?>/gi, '\n')
      .replace(/<li\b[^>]*>/gi, '• ')
      .replace(/<\/(?:p|h1|h2|h3|li|blockquote|pre|ul|ol)>/gi, '\n')

    if (typeof document === 'undefined') {
      return withSeparators
        .replace(/<[^>]+>/g, ' ')
        .replace(/\s+/g, ' ')
        .trim()
    }

    const container = document.createElement('div')
    container.innerHTML = withSeparators
    return (container.textContent ?? '').replace(/\s+/g, ' ').trim()
  }

  function getNotePreview(content: string): string {
    return getPlainTextFromNote(content)
  }

  function formatNoteDate(timestamp: number): string {
    return new Date(timestamp).toLocaleDateString()
  }

  /** Load notes scoped to the current asset (plus item-level notes). */
  async function loadNotesForAsset(): Promise<Note[]> {
    if (!selectedAsset) {
      const store = getStore()
      return store.notes.findByItem(itemId)
    }
    const store = getStore()
    return store.notes.findByAsset(itemId, selectedAsset.id)
  }

  async function loadData() {
    try {
      loading = true
      error = null
      selectedAssetIndex = 0 // Reset page selection on item change
      const store = getStore()
      const [loadedItem, loadedAssets, loadedCollection] = await Promise.all([
        store.items.findById(itemId),
        store.assets.findByItem(itemId),
        store.collections.findById(collectionId),
      ])
      item = loadedItem
      assets = loadedAssets
      collection = loadedCollection
      // Asset-scoped data (notes, entities, triples, similar assets) will be loaded by the selectedAsset effect
      void loadTopics()
      void loadTopicSuggestions()
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to load item'
    } finally {
      loading = false
    }
  }

  async function reloadLayoutForAsset(asset: Asset | null) {
    const requestToken = ++layoutLoadToken

    if (!asset || asset.type === 'audio') {
      assetLayout = null
      layoutLoading = false
      layoutError = null
      return
    }

    layoutLoading = true
    layoutError = null

    try {
      const layout = await getLayoutByAsset(asset.id)
      if (layoutLoadToken !== requestToken || selectedAsset?.id !== asset.id) {
        return
      }

      assetLayout = layout
      if (!layout || layout.blocks.length === 0) {
        showLayout = false
      }
    } catch (e) {
      if (layoutLoadToken !== requestToken || selectedAsset?.id !== asset.id) {
        return
      }

      assetLayout = null
      layoutError = e instanceof Error ? e.message : 'Failed to load layout'
      showLayout = false
    } finally {
      if (layoutLoadToken === requestToken && selectedAsset?.id === asset.id) {
        layoutLoading = false
      }
    }
  }

  $effect(() => {
    const asset = selectedAsset
    const currentAssetId = asset?.id ?? null
    const switchedAsset = currentAssetId !== lastSelectedAssetId

    lastSelectedAssetId = currentAssetId

    if (switchedAsset) {
      selectedAnnotationId = null
      annotationTool = 'select'
      editTool = 'none'
      viewerPage = 1
      viewerTotalPages = 1
      showLayout = false
      layoutTypeFilter = 'all'
      layoutHoveredBlockId = null
      layoutSelectedBlockId = null
      layoutHoveredRegionId = null
      layoutSelectedRegionId = null
      lastAutoScrolledLayoutBlockId = null
      // Reset undo stack only when switching to a DIFFERENT asset by id.
      // Editing the same asset creates a new versioned path, which should NOT
      // clear undo history.
      undoStack = []
    }

    if (pendingAnnotationSave && pendingAnnotationSave.assetId !== currentAssetId) {
      void flushPendingAnnotationSave()
    }

    if (!asset || asset.type !== 'image') {
      annotations = []
      annotationSaveError = null
      return
    }

    let cancelled = false

    void (async () => {
      try {
        annotationSaveError = null
        const loadedAnnotations = await getStore().annotations.findByAsset(asset.id, 1)
        if (!cancelled && selectedAsset?.id === asset.id) {
          annotations = loadedAnnotations.map((a) => ({
            ...a,
            kind: a.kind as ViewerAnnotationKind,
          }))
        }
      } catch {
        if (!cancelled) {
          annotations = []
          annotationSaveError = 'Failed to load annotations for this asset.'
        }
      }
    })()

    return () => {
      cancelled = true
    }
  })

  $effect(() => {
    window.dispatchEvent(
      new CustomEvent<DocumentExplorerAssetDetail>(DOCUMENT_EXPLORER_ASSET_SELECTED_EVENT, {
        detail: {
          itemId,
          assetId: selectedAsset?.id ?? null,
        },
      })
    )
  })

  $effect(() => {
    void reloadLayoutForAsset(selectedAsset)
  })

  $effect(() => {
    if (!visibleLayoutBlocks.some((block) => block.id === layoutSelectedBlockId)) {
      layoutSelectedBlockId = null
      layoutSelectedRegionId = null
    }

    if (!visibleLayoutBlocks.some((block) => block.id === layoutHoveredBlockId)) {
      layoutHoveredBlockId = null
      layoutHoveredRegionId = null
    }
  })

  $effect(() => {
    const selectedBlockId = layoutSelectedBlockId

    if (!selectedBlockId) {
      lastAutoScrolledLayoutBlockId = null
      return
    }

    if (!visibleLayoutBlocks.some((block) => block.id === selectedBlockId)) {
      lastAutoScrolledLayoutBlockId = null
      return
    }

    if (lastAutoScrolledLayoutBlockId === selectedBlockId) {
      return
    }

    lastAutoScrolledLayoutBlockId = selectedBlockId
    queueMicrotask(() => {
      scrollSelectedLayoutBlockIntoView(selectedBlockId)
    })
  })

  // Reload asset-scoped data when the selected asset changes
  $effect(() => {
    const asset = selectedAsset
    if (!asset) return

    leftPanelTab = 'document'
    rightPanelTab = 'notes'

    // Reload notes for this asset (plus item-level notes)
    void loadNotesForAsset().then((loadedNotes) => {
      notes = loadedNotes
    })

    // Load existing extraction text for this asset
    const store = getStore()
    void store.extractions.findByAsset(asset.id).then((extraction) => {
      if (extraction) {
        ocrStore._updateState(asset.id, {
          status: 'done',
          progress: 100,
          textLength: extraction.textContent.length,
          method: extraction.method,
          textContent: extraction.textContent,
        })
        ocrTick++
      }
    })

    // Load existing transcription for audio assets
    if (asset.type === 'audio') {
      void store.transcriptions.findByAsset(asset.id).then((transcription) => {
        if (transcription) {
          transcriptionStore._updateState(asset.id, {
            status: 'done',
            progress: 100,
            text: transcription.textContent,
            language: transcription.language ?? undefined,
            durationMs: transcription.durationMs ?? undefined,
            segmentsCount: transcription.segments
              ? TranscriptionRepo.parseSegments(transcription.segments).length
              : 0,
          })
          transcriptionTick++
        }
      })
    }
  })

  // Reload analysis data when the selected asset changes
  $effect(() => {
    const asset = selectedAsset
    if (!asset) return
    void loadEntities()
    void loadTriples()
    void loadSimilarAssets()
    // Load persisted LLM results for this asset so previous
    // asset-level results (summarize, correct_ocr, etc.) are visible.
    llmStore.loadPersistedResults(asset.id, 'asset')
    llmGetResult(asset.id, 'summarize', 'asset')
      .then((result) => {
        if (result) {
          summaryTexts.set(asset.id, result.result)
          summaryTick++
        }
      })
      .catch(() => {
        // Silently degrade — persisted summaries are optional
      })
  })

  $effect(() => {
    // Reload all data when navigating to a different item.
    // Reading itemId here ensures the effect re-runs when the prop changes.
    const _id = itemId
    void loadData()
  })

  onMount(() => {
    window.addEventListener(
      DOCUMENT_EXPLORER_ASSET_SELECT_REQUEST_EVENT,
      handleExplorerAssetSelectRequest
    )

    ocrStore
      .startListening((eventName, callback) =>
        listen(eventName, callback).then((unlisten) => {
          // Wrap unlisten to also trigger reactivity tick
          return () => {
            unlisten()
          }
        })
      )
      .then(() => {
        // Patch each event to also bump ocrTick for Svelte reactivity
        const origUpdate = ocrStore._updateState.bind(ocrStore)
        ocrStore._updateState = (assetId, partial) => {
          origUpdate(assetId, partial)
          ocrTick++
        }
      })

    nlpStore
      .startListening((eventName, callback) =>
        listen(eventName, callback).then((unlisten) => () => unlisten())
      )
      .then(() => {
        const origSet = nlpStore._setJobStatus.bind(nlpStore)
        nlpStore._setJobStatus = (id, job, status, err) => {
          origSet(id, job, status, err)
          nlpTick++
          // After NER completes, reload entities for the current context
          if (job === 'ner' && status === 'done' && id === itemId) {
            void reloadSelectedAssetPersistedState({ entities: true })
          }
          if (job === 'embed' && status === 'done' && id === itemId) {
            void reloadSelectedAssetPersistedState({ similarAssets: true })
          }
          if (job === 'triples' && status === 'done' && id === itemId) {
            void reloadSelectedAssetPersistedState({ triples: true })
          }
        }
      })

    transcriptionStore
      .startListening((eventName, callback) =>
        listen(eventName, callback).then((unlisten) => () => unlisten())
      )
      .then(() => {
        const origUpdate = transcriptionStore._updateState.bind(transcriptionStore)
        transcriptionStore._updateState = (assetId, partial) => {
          origUpdate(assetId, partial)
          transcriptionTick++
        }
      })

    llmStore.startListening().then(() => {
      llmStore.onChange(() => {
        llmTick++
      })
      // Load persisted LLM results for the item (legacy item-level results).
      // Asset-level results are loaded in the selectedAsset effect below.
      llmStore.loadPersistedResults(itemId, 'item')
    })

    llmIsAvailable()
      .then((available) => {
        llmAvailable = available
      })
      .catch(() => {
        llmAvailable = false
      })

    geoStore.startListening()
    return () => {
      if (metadataSaveTimer) clearTimeout(metadataSaveTimer)
    }
  })

  onDestroy(() => {
    layoutLoadToken++
    window.removeEventListener(
      DOCUMENT_EXPLORER_ASSET_SELECT_REQUEST_EVENT,
      handleExplorerAssetSelectRequest
    )
    clearLayoutInspectorCopyMessage()
    ocrStore.stopListening()
    nlpStore.stopListening()
    transcriptionStore.stopListening()
    llmStore.stopListening()
    geoStore.stopListening()
    // Clear any pending debounce timers to avoid stale persist after unmount
    for (const timer of ocrPersistTimers.values()) {
      clearTimeout(timer)
    }
    ocrPersistTimers.clear()
    for (const timer of transPersistTimers.values()) {
      clearTimeout(timer)
    }
    transPersistTimers.clear()
    for (const timer of assetReanalysisTimers.values()) {
      clearTimeout(timer)
    }
    assetReanalysisTimers.clear()
    clearAnnotationSaveTimer()
    clearFtsSearchTimer()
    if (dragCleanup) dragCleanup()
  })
</script>

{#if loading}
  <p class="status">{translate('item.loading')}</p>
{:else if error && !item}
  <p class="error">{error}</p>
{:else if item}
  <div
    class="item-view"
    bind:this={itemViewEl}
    style="grid-template-columns: 1fr auto {rightPanelOpen ? `6px ${sidebarWidth}%` : ''}"
  >
    <div class="left-panel">
      {#if selectedAsset}
        <div class="left-panel-tabs" role="tablist" aria-label={translate('item.assetPanel')}>
          <button
            id="left-panel-tab-document"
            type="button"
            role="tab"
            class:active={leftPanelTab === 'document'}
            class="left-panel-tab"
            aria-selected={leftPanelTab === 'document'}
            aria-controls="left-panel-document"
            onclick={() => {
              leftPanelTab = 'document'
            }}
          >
            {translate('item.documentTab')}
          </button>
          <button
            id="left-panel-tab-text"
            type="button"
            role="tab"
            class:active={leftPanelTab === 'text'}
            class="left-panel-tab"
            aria-selected={leftPanelTab === 'text'}
            aria-controls="left-panel-text"
            onclick={() => {
              leftPanelTab = 'text'
            }}
          >
            {translate('item.extractedTextTab')}
          </button>
        </div>

        <div class="left-panel-content">
          <div
            id="left-panel-document"
            role="tabpanel"
            aria-labelledby="left-panel-tab-document"
            class="left-panel-pane left-panel-pane--document"
            class:is-hidden={leftPanelTab !== 'document'}
          >
            <DocumentViewer
              path={selectedAsset.path}
              assetUrl={viewerSrc}
              type={viewerType}
              {annotations}
              {layoutRegions}
              showLayoutOverlay={showLayout && layoutRegions.length > 0}
              hoveredLayoutRegionId={layoutHoveredRegionId}
              selectedLayoutRegionId={layoutSelectedRegionId}
              {layoutReferenceWidth}
              {layoutReferenceHeight}
              {selectedAnnotationId}
              {annotationTool}
              {annotationColor}
              {editTool}
              {canUndo}
              currentPage={viewerPage}
              onAnnotationsChange={handleAnnotationsChange}
              onSelectedAnnotationIdChange={handleSelectedAnnotationIdChange}
              onAnnotationToolChange={handleAnnotationToolChange}
              onAnnotationColorChange={handleAnnotationColorChange}
              onLayoutRegionHoverChange={(regionId: string | null) => {
                syncLayoutHoverFromRegion(regionId)
              }}
              onLayoutRegionSelect={(regionId: string) => {
                setSelectedLayoutRegion(regionId)
              }}
              onEditSelect={handleEditSelect}
              onEditToolChange={(tool: EditTool) => {
                editTool = tool
                if (tool !== 'none') annotationTool = 'select'
              }}
              onRotateLeft={handleRotateLeft}
              onRotateRight={handleRotateRight}
              onUndo={handleUndo}
              onPageChange={(page: number, totalPages: number) => {
                viewerPage = page
                viewerTotalPages = totalPages
              }}
              onDimensionsChange={(dims: { width: number; height: number }) => {
                imageNaturalW = dims.width
                imageNaturalH = dims.height
              }}
              labels={documentViewerLabels}
              {annotationToolbarLabels}
            />

            {#if annotationSaveError}
              <p class="error">{annotationSaveError}</p>
            {/if}
          </div>

          <div
            id="left-panel-text"
            role="tabpanel"
            aria-labelledby="left-panel-tab-text"
            class="left-panel-pane left-panel-pane--text"
            class:is-hidden={leftPanelTab !== 'text'}
          >
            {#if selectedAsset.type !== 'audio'}
              {@const ocr = getOcrState(selectedAsset.id)}
              <section class="left-text-panel-section">
                <div class="left-text-panel-card">
                  {#if (ocrEditedText.get(selectedAsset.id) ?? ocr.textContent ?? '').trim()}
                    <div class="left-text-panel-meta">
                      <span>{translate('item.extractedText')}</span>
                      <span class="ocr-meta"
                        >via {ocr.method ?? translate('item.ocrMethodUnknown')} · {translate(
                          'item.characters',
                          {
                            count: (ocrEditedText.get(selectedAsset.id) ?? ocr.textContent ?? '')
                              .length,
                          }
                        )}</span
                      >
                    </div>
                    <div class="left-text-panel-body">
                      {ocrEditedText.get(selectedAsset.id) ?? ocr.textContent ?? ''}
                    </div>
                  {:else}
                    <p class="empty-text">{translate('item.noExtractedText')}</p>
                  {/if}
                </div>
              </section>
            {:else}
              {@const ts = getTranscriptionState(selectedAsset.id)}
              <section class="left-text-panel-section">
                <div class="left-text-panel-card">
                  {#if (transEditedText.get(selectedAsset.id) ?? ts.text ?? '').trim()}
                    <div class="left-text-panel-meta">
                      <span>{translate('item.transcription')}</span>
                      <span class="ocr-meta">
                        {#if ts.language}{ts.language} &middot;
                        {/if}{translate('item.characters', {
                          count: (transEditedText.get(selectedAsset.id) ?? ts.text ?? '').length,
                        })}
                        {#if ts.durationMs}
                          &middot; {translate('item.audioDurationSeconds', {
                            count: Math.round(ts.durationMs / 1000),
                          })}{/if}
                      </span>
                    </div>
                    <div class="left-text-panel-body">
                      {transEditedText.get(selectedAsset.id) ?? ts.text ?? ''}
                    </div>
                  {:else}
                    <p class="empty-text">{translate('item.noExtractedText')}</p>
                  {/if}
                </div>
              </section>
            {/if}
          </div>
        </div>
      {:else}
        <div class="empty-viewer">
          <p>{translate('item.noAssets')}</p>
        </div>
      {/if}

      {#if assets.length > 1}
        <div class="asset-pagination">
          <button
            class="pagination-btn"
            disabled={selectedAssetIndex <= 0}
            onclick={() => (selectedAssetIndex = Math.max(0, selectedAssetIndex - 1))}
            aria-label={translate('item.previousPage')}
          >
            ‹
          </button>
          <span class="pagination-info">
            {selectedAssetIndex + 1} / {assets.length}
          </span>
          <button
            class="pagination-btn"
            disabled={selectedAssetIndex >= assets.length - 1}
            onclick={() =>
              (selectedAssetIndex = Math.min(assets.length - 1, selectedAssetIndex + 1))}
            aria-label={translate('item.nextPage')}
          >
            ›
          </button>
        </div>
      {/if}
    </div>

    <!-- Right panel toggle -->
    <button
      class="right-panel-toggle"
      type="button"
      onclick={() => { rightPanelOpen = !rightPanelOpen }}
      title={rightPanelOpen ? 'Ocultar panel' : 'Mostrar panel'}
      aria-label={rightPanelOpen ? 'Ocultar panel derecho' : 'Mostrar panel derecho'}
    >
      <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
        {#if rightPanelOpen}
          <path d="m6 3.5 4.5 4.5L6 12.5" />
        {:else}
          <path d="m10 3.5-4.5 4.5 4.5 4.5" />
        {/if}
      </svg>
    </button>

    {#if rightPanelOpen}
    <div
      class="resize-handle"
      role="separator"
      aria-orientation="vertical"
      onpointerdown={onResizeHandlePointerDown}
    ></div>

    <div class="right-panel">
      <header class="item-header">
        <span class="item-header__eyebrow">{translate('item.activeDocument')}</span>
        <h2 class="item-title">{item.title}</h2>
        <p class="item-header__meta">{activeAssetSummary}</p>
      </header>

      {#if error}
        <p class="error">{error}</p>
      {/if}

      <div class="right-panel-tabs" role="tablist" aria-label={translate('item.rightPanel')}>
        <button
          type="button"
          role="tab"
          class:active={rightPanelTab === 'notes'}
          class="right-panel-tab"
          aria-selected={rightPanelTab === 'notes'}
          onclick={() => {
            rightPanelTab = 'notes'
          }}
        >
          {translate('item.notesTab')}
        </button>
        <button
          type="button"
          role="tab"
          class:active={rightPanelTab === 'text'}
          class="right-panel-tab"
          aria-selected={rightPanelTab === 'text'}
          onclick={() => {
            rightPanelTab = 'text'
          }}
        >
          {translate('item.textTab')}
        </button>
        <button
          type="button"
          role="tab"
          class:active={rightPanelTab === 'analysis'}
          class="right-panel-tab"
          aria-selected={rightPanelTab === 'analysis'}
          onclick={() => {
            rightPanelTab = 'analysis'
            loadEntities()
            loadTriples()
            loadGeoMarkers()
          }}
        >
          {translate('item.analysisTab')}
        </button>
        <button
          type="button"
          role="tab"
          class:active={rightPanelTab === 'search'}
          class="right-panel-tab"
          aria-selected={rightPanelTab === 'search'}
          onclick={() => {
            rightPanelTab = 'search'
            loadSimilarAssets()
            loadFtsStats()
          }}
        >
          {translate('item.searchTab')}
        </button>
        <button
          type="button"
          role="tab"
          class:active={rightPanelTab === 'layout'}
          class="right-panel-tab"
          aria-selected={rightPanelTab === 'layout'}
          onclick={() => {
            rightPanelTab = 'layout'
          }}
        >
          {translate('item.layoutTab')}
        </button>
        <button
          type="button"
          role="tab"
          class:active={rightPanelTab === 'metadata'}
          class="right-panel-tab"
          aria-selected={rightPanelTab === 'metadata'}
          onclick={() => {
            rightPanelTab = 'metadata'
          }}
        >
          {translate('item.metadataTab')}
        </button>
      </div>

      <div class="right-panel-content">
        <div class="right-panel-pane" class:is-hidden={rightPanelTab !== 'notes'}>
          <section class="section">
            <h3>{translate('item.topics')}</h3>
            <TopicEditor
              topics={itemTopics}
              suggestions={topicSuggestions}
              onchange={handleTopicsChange}
            />
          </section>

          <section class="section">
            <h3>
              {translate('item.addNote')}{#if assets.length > 1}
                {translate('item.pageInline', { page: selectedAssetIndex + 1 })}{/if}
            </h3>
            <NoteEditor
              onsave={handleSaveNote}
              ondictate={handleTranscribeDictation}
              clearOnSave={true}
              placeholder={translate('item.writeNote')}
              saveLabel={translate('item.saveNote')}
              labels={noteEditorLabels}
            />
          </section>

          <section class="section">
            <h3>
              {translate('item.notes')} ({notes.length}){#if assets.length > 1}
                {translate('item.pageInline', { page: selectedAssetIndex + 1 })}{/if}
            </h3>
            {#if notes.length === 0}
              <p class="empty-text">{translate('item.noNotes')}</p>
            {:else}
              <div class="notes-list">
                {#each notes as note (note.id)}
                  <div class="note-card">
                    {#if editingNoteId === note.id}
                      <div class="note-edit">
                        <NoteEditor
                          content={note.content}
                          onsave={(content: string) => handleSaveEdit(note.id, content)}
                          oncancel={handleCancelEdit}
                          ondictate={handleTranscribeDictation}
                          clearOnSave={false}
                          saveLabel={translate('item.saveNote')}
                          cancelLabel={translate('item.cancelEdit')}
                          labels={noteEditorLabels}
                        />
                      </div>
                    {:else}
                      <div
                        class="note-row"
                        role="button"
                        tabindex="0"
                        aria-label={getNotePreview(note.content)}
                        aria-expanded={expandedNoteId === note.id}
                        onclick={(event) => handleNoteRowClick(note.id, event)}
                        onkeydown={(event) => handleNoteRowKeydown(note.id, event)}
                      >
                        <span class="note-preview" title={getNotePreview(note.content)}>
                          {getNotePreview(note.content)}
                        </span>
                        <p class="note-date note-date--inline">{formatNoteDate(note.createdAt)}</p>
                        <div class="note-actions">
                          <button
                            type="button"
                            class="note-action-button note-action-button--edit"
                            aria-label={translate('item.editNote')}
                            onclick={(event) => {
                              event.stopPropagation()
                              handleEditNote(note)
                            }}
                          >
                            <ActionIcon name="edit" />
                          </button>
                          <button
                            type="button"
                            class="note-action-button note-action-button--delete"
                            aria-label={translate('item.deleteNote')}
                            onclick={(event) => {
                              event.stopPropagation()
                              openDeleteNoteConfirm(note.id)
                            }}
                          >
                            <ActionIcon name="delete" />
                          </button>
                        </div>
                      </div>
                      {#if expandedNoteId === note.id}
                        <div
                          class="note-expanded note-content note-content--rich"
                          use:expandedNoteContentLinkHandler
                        >
                          {@html getRenderedNoteContent(note.content)}
                        </div>
                      {/if}
                    {/if}
                  </div>
                {/each}
              </div>
            {/if}
          </section>

          {#if pendingDeleteNoteId}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div class="modal-overlay" onclick={handleDeleteNoteCancel} role="presentation">
              <div
                bind:this={deleteNoteDialogEl}
                class="modal"
                tabindex="-1"
                role="alertdialog"
                aria-modal="true"
                aria-labelledby="delete-note-modal-title"
                aria-describedby="delete-note-modal-description"
                onclick={(event) => event.stopPropagation()}
                onkeydown={handleDeleteNoteDialogKeydown}
              >
                <h3 id="delete-note-modal-title" class="modal-title">
                  {translate('item.deleteNoteTitle')}
                </h3>
                <p id="delete-note-modal-description" class="modal-message">
                  {translate('item.deleteNoteMessage')}
                </p>

                <div class="modal-actions">
                  <button
                    type="button"
                    class="modal-secondary-button"
                    onclick={handleDeleteNoteCancel}
                    disabled={deletingNote}
                  >
                    {translate('collections.cancel')}
                  </button>
                  <button
                    type="button"
                    class="modal-delete-button"
                    aria-label={translate('item.confirmDeleteNote')}
                    title={translate('item.confirmDeleteNote')}
                    onclick={handleDeleteNoteConfirm}
                    disabled={deletingNote}
                    aria-busy={deletingNote}
                  >
                    <ActionIcon name="delete" />
                  </button>
                </div>
              </div>
            </div>
          {/if}
        </div>

        <div class="right-panel-pane" class:is-hidden={rightPanelTab !== 'metadata'}>
          <section class="section">
            <h3>
              {translate('item.metadata')}
              {#if savingMetadata}<span class="saving">{translate('item.saving')}</span>{/if}
            </h3>

            <div class="metadata-sections">
              <section class="metadata-subsection" data-testid="item-file-metadata">
                <h4>Metadatos del archivo</h4>

                {#if fileMetadataEntries.length > 0}
                  <dl class="metadata-list">
                    {#each fileMetadataEntries as entry (entry.label)}
                      <div class="metadata-list__row">
                        <dt>{entry.label}</dt>
                        <dd>{entry.value}</dd>
                      </div>
                    {/each}
                  </dl>
                {/if}
              </section>

              <section class="metadata-subsection" data-testid="item-custom-metadata">
                <h4>Metadatos personalizados</h4>
                <MetadataEditor
                  value={metadataValue}
                  onchange={handleMetadataChange}
                  labels={metadataEditorLabels}
                />
              </section>
            </div>
          </section>
        </div>

        <div class="right-panel-pane" class:is-hidden={rightPanelTab !== 'layout'}>
          {#if selectedAsset && selectedAsset.type !== 'audio'}
            <section class="section">
              <div class="layout-section-header">
                <div>
                  <h3>
                    {translate('item.layoutTab')}{#if viewerType === 'pdf'}
                      {translate('item.pageInline', { page: layoutActivePage })}{/if}
                  </h3>
                  {#if assetLayout}
                    <p class="layout-meta">
                      {assetLayout.model} · {viewerType === 'pdf'
                        ? translate('item.layoutBlocksCount', {
                            count: layoutBlockCountsByPage[layoutActivePage] ?? 0,
                          })
                        : translate('item.layoutBlocksCount', { count: layoutBlocks.length })} · {viewerType ===
                      'pdf'
                        ? translate('item.layoutRegionsCount', {
                            count: layoutPageRegions.length,
                          })
                        : translate('item.layoutRegionsCount', {
                            count: assetLayout.regions.length,
                          })}
                    </p>
                  {/if}
                </div>

                <button
                  type="button"
                  class="layout-toggle"
                  disabled={!hasLayoutData}
                  aria-pressed={showLayout}
                  onclick={() => {
                    showLayout = !showLayout
                  }}
                >
                  {showLayout
                    ? translate('item.layoutToggleHide')
                    : translate('item.layoutToggleShow')}
                </button>
              </div>

              {#if layoutLoading}
                <p class="empty-text">{translate('item.layoutLoading')}</p>
              {:else if layoutError}
                <p class="error">{translate('item.layoutLoadError', { error: layoutError })}</p>
              {:else if !assetLayout}
                <p class="empty-text">{translate('item.layoutMissing')}</p>
              {:else if layoutBlocks.length === 0}
                <p class="empty-text">{translate('item.layoutNoBlocks')}</p>
              {:else}
                {#if showLayout}
                  <p class="layout-help">{translate('item.layoutHelp')}</p>
                {/if}

                {#if viewerType === 'pdf' && layoutPageOptions.length > 1}
                  <div class="layout-page-toolbar">
                    <p class="layout-page-summary" data-testid="layout-page-summary">
                      {translate('item.pageOf', {
                        page: layoutActivePage,
                        total: layoutPageOptions.length,
                      })}
                    </p>

                    <div
                      class="layout-page-group"
                      role="group"
                      aria-label={translate('item.layoutPageSelect')}
                    >
                      {#each layoutPageOptions as page (page)}
                        <button
                          type="button"
                          class:active={layoutActivePage === page}
                          class="layout-page-chip"
                          data-testid={`layout-page-chip-${page}`}
                          aria-pressed={layoutActivePage === page}
                          onclick={() => {
                            viewerPage = page
                          }}
                        >
                          <span>{translate('item.pageShort', { page })}</span>
                          <span class="layout-page-chip__count"
                            >{layoutBlockCountsByPage[page] ?? 0}</span
                          >
                        </button>
                      {/each}
                    </div>
                  </div>
                {/if}

                <div class="layout-filter-toolbar">
                  <div
                    class="layout-filter-group"
                    role="group"
                    aria-label={translate('item.layoutFilterGroup')}
                  >
                    {#each LAYOUT_BLOCK_FILTERS as filter (filter.id)}
                      {@const count = layoutFilterCounts[filter.id]}
                      <button
                        type="button"
                        class:active={layoutTypeFilter === filter.id}
                        class="layout-filter-chip"
                        data-testid={`layout-filter-${filter.id}`}
                        aria-pressed={layoutTypeFilter === filter.id}
                        onclick={() => {
                          layoutTypeFilter = filter.id
                        }}
                      >
                        <span>{layoutFilterLabels[filter.id]}</span>
                        <span
                          class="layout-filter-chip__count"
                          data-testid={`layout-filter-count-${filter.id}`}
                        >
                          {count}
                        </span>
                      </button>
                    {/each}
                  </div>

                  <p class="layout-filter-summary">
                    {translate('item.layoutShowing', {
                      visible: visibleLayoutBlocks.length,
                      total: layoutPageBlocks.length,
                    })}
                  </p>
                </div>

                {#if layoutPageBlocks.length === 0}
                  <p class="empty-text">{translate('item.layoutNoPageBlocks')}</p>
                {:else if visibleLayoutBlocks.length === 0}
                  <p class="empty-text">{translate('item.layoutNoFilterBlocks')}</p>
                {:else}
                  <div class="layout-block-list" bind:this={layoutBlockListEl}>
                    {#each visibleLayoutBlocks as block (block.id)}
                      {@const isHovered = layoutHoveredBlockId === block.id}
                      {@const isSelected = layoutSelectedBlockId === block.id}
                      {@const overlayMeta = getLayoutOverlaySourceMeta(block.overlaySource)}
                      <button
                        type="button"
                        data-testid={`layout-block-item-${block.id}`}
                        data-layout-block-id={block.id}
                        class:hovered={isHovered}
                        class:selected={isSelected}
                        class:fallback={block.overlaySource === 'block'}
                        class="layout-block-item"
                        onmouseenter={() => {
                          syncLayoutHoverFromBlock(block.id)
                        }}
                        onmouseleave={() => {
                          syncLayoutHoverFromBlock(null)
                        }}
                        onclick={() => {
                          setSelectedLayoutBlock(block.id)
                        }}
                      >
                        <span class="layout-block-order">#{block.order}</span>
                        <span class="layout-block-content">
                          <span class="layout-block-heading">
                            <span class="layout-block-label">{block.label}</span>
                            <span
                              class:layout-block-source-badge--fallback={block.overlaySource ===
                                'block'}
                              class="layout-block-source-badge"
                            >
                              {overlayMeta.shortLabel}
                            </span>
                            {#if viewerType === 'pdf'}
                              <span class="layout-block-page-chip"
                                >{translate('item.pageShort', { page: block.page })}</span
                              >
                            {/if}
                          </span>
                          <span class="layout-block-preview"
                            >{block.preview || translate('item.layoutNoPreview')}</span
                          >
                        </span>
                      </button>
                    {/each}
                  </div>

                  <div class="layout-inspector" data-testid="layout-block-inspector">
                    {#if selectedLayoutBlock}
                      {@const overlayMeta = getLayoutOverlaySourceMeta(
                        selectedLayoutBlock.overlaySource
                      )}
                      <div class="layout-inspector__header">
                        <div>
                          <p class="layout-inspector__eyebrow">
                            {translate('item.layoutInspector')}
                          </p>
                          <h4>
                            {translate('item.layoutSelectedBlock', {
                              order: selectedLayoutBlock.order,
                            })}
                          </h4>
                        </div>

                        <div class="layout-inspector__actions">
                          <button
                            type="button"
                            class="layout-inspector__action"
                            data-testid="layout-inspector-copy-text"
                            disabled={!selectedLayoutBlock.content.trim()}
                            onclick={() =>
                              copyLayoutInspectorValue(
                                selectedLayoutBlock.content,
                                translate('item.layoutCopiedText')
                              )}
                          >
                            {translate('item.layoutCopyText')}
                          </button>
                          <button
                            type="button"
                            class="layout-inspector__action"
                            data-testid="layout-inspector-copy-bbox"
                            onclick={() =>
                              copyLayoutInspectorValue(
                                formatLayoutBbox(selectedLayoutBlock.overlayBbox),
                                translate('item.layoutCopiedBbox')
                              )}
                          >
                            {translate('item.layoutCopyBbox')}
                          </button>
                          <button
                            type="button"
                            class="layout-inspector__action"
                            data-testid="layout-inspector-copy-json"
                            onclick={() =>
                              copyLayoutInspectorValue(
                                serializeLayoutBlock(selectedLayoutBlock),
                                translate('item.layoutCopiedJson')
                              )}
                          >
                            {translate('item.layoutCopyJson')}
                          </button>
                        </div>
                      </div>

                      <div class="layout-inspector__grid">
                        <div>
                          <span class="layout-inspector__label"
                            >{translate('item.layoutLabel')}</span
                          >
                          <strong data-testid="layout-inspector-label"
                            >{selectedLayoutBlock.label}</strong
                          >
                        </div>
                        <div>
                          <span class="layout-inspector__label"
                            >{translate('item.layoutOrder')}</span
                          >
                          <strong>#{selectedLayoutBlock.order}</strong>
                        </div>
                        <div>
                          <span class="layout-inspector__label">{translate('item.layoutPage')}</span
                          >
                          <strong>{selectedLayoutBlock.page}</strong>
                        </div>
                        <div>
                          <span class="layout-inspector__label"
                            >{translate('item.layoutGroup')}</span
                          >
                          <strong>{selectedLayoutBlock.groupId || '—'}</strong>
                        </div>
                        <div>
                          <span class="layout-inspector__label"
                            >{translate('item.layoutBlockBbox')}</span
                          >
                          <code>{formatLayoutBbox(selectedLayoutBlock.bbox)}</code>
                        </div>
                        <div>
                          <span class="layout-inspector__label"
                            >{translate('item.layoutOverlayBbox')}</span
                          >
                          <code data-testid="layout-inspector-bbox"
                            >{formatLayoutBbox(selectedLayoutBlock.overlayBbox)}</code
                          >
                        </div>
                        <div class="layout-inspector__field layout-inspector__field--wide">
                          <span class="layout-inspector__label"
                            >{translate('item.layoutOverlaySource')}</span
                          >
                          <strong
                            class:layout-inspector__source--fallback={selectedLayoutBlock.overlaySource ===
                              'block'}
                            class="layout-inspector__source"
                            data-testid="layout-inspector-overlay-source"
                          >
                            {overlayMeta.label}
                          </strong>
                          <p>{overlayMeta.description}</p>
                        </div>
                      </div>

                      <div class="layout-inspector__content">
                        <span class="layout-inspector__label"
                          >{translate('item.layoutPreview')}</span
                        >
                        <pre data-testid="layout-inspector-content">{selectedLayoutBlock.content ||
                            translate('item.layoutNoFullText')}</pre>
                      </div>

                      {#if layoutInspectorCopyMessage}
                        <p
                          class:layout-inspector__message--error={layoutInspectorCopyMessage.tone ===
                            'error'}
                          class="layout-inspector__message"
                          data-testid="layout-inspector-copy-message"
                        >
                          {layoutInspectorCopyMessage.text}
                        </p>
                      {/if}
                    {:else}
                      <div class="layout-inspector__empty" data-testid="layout-inspector-empty">
                        {translate('item.layoutEmptyInspector')}
                      </div>
                    {/if}
                  </div>
                {/if}
              {/if}
            </section>
          {:else}
            <section class="section">
              <p class="empty-text">{translate('item.layoutUnavailableForAudio')}</p>
            </section>
          {/if}
        </div>

        <div class="right-panel-pane" class:is-hidden={rightPanelTab !== 'text'}>
          {#if selectedAsset && selectedAsset.type !== 'audio'}
            {@const ocr = getOcrState(selectedAsset.id)}
            {@const busy = ocr.status === 'pending' || ocr.status === 'running'}
            {@const isPdfAsset = selectedAsset.type === 'pdf'}
            <section class="section">
              <h3>
                {translate('item.textExtraction')}{#if assets.length > 1}
                  {translate('item.pageInline', { page: selectedAssetIndex + 1 })}{/if}
              </h3>
              <div class="ocr-item">
                <div class="ocr-item-header">
                  <span class="ocr-filename">
                    {assets.length > 1 && assets.every((a) => a.type === 'image')
                      ? translate('item.assetPageLabel', { page: selectedAssetIndex + 1 })
                      : (selectedAsset.path.split(/[/\\]/).pop() ??
                        translate('item.assetNoSelection'))}
                  </span>
                  <div class="ocr-btn-group">
                    <button
                      class="ocr-btn ocr-btn--light"
                      disabled={busy}
                      onclick={() => handleExtractText(selectedAsset, 'light')}
                      title={busy
                        ? isPdfAsset
                          ? translate('item.pdfTextBusyTitle')
                          : translate('item.ocrFastBusyTitle')
                        : isPdfAsset
                          ? translate('item.pdfTextTitle')
                          : translate('item.ocrFastTitle')}
                    >
                      {getExtractionPrimaryActionLabel(selectedAsset.type)}
                    </button>
                    {#if !isPdfAsset}
                      <button
                        class="ocr-btn ocr-btn--high"
                        disabled={busy}
                        onclick={() => handleExtractText(selectedAsset, 'high')}
                        title={busy
                          ? translate('item.ocrHighBusyTitle')
                          : translate('item.ocrHighTitle')}
                      >
                        {translate('item.ocrHighAction')}
                      </button>
                    {/if}
                    {#if llmAvailable && !ocrCorrectedAssets.has(selectedAsset.id)}
                      <button
                        class="ocr-btn ocr-btn--correct"
                        disabled={getLlmState().status === 'running' || ocr.status !== 'done'}
                        onclick={handleLlmCorrectOcr}
                        title={!llmAvailable
                          ? translate('item.ocrCorrectUnavailable')
                          : ocr.status !== 'done'
                            ? translate('item.ocrCorrectNeedsText')
                            : isPdfAsset
                              ? translate('item.pdfCorrectTitle')
                              : translate('item.ocrCorrectTitle')}
                      >
                        {getCorrectionActionLabel(selectedAsset.type)}
                      </button>
                    {/if}
                    {#if llmAvailable}
                      <button
                        class="ocr-btn ocr-btn--summarize"
                        disabled={getLlmState().status === 'running' || ocr.status !== 'done'}
                        onclick={handleLlmSummarize}
                        title={!llmAvailable
                          ? translate('item.summaryUnavailable')
                          : ocr.status !== 'done'
                            ? translate('item.summaryNeedsText')
                            : translate('item.summaryTitle')}
                      >
                        {getSummaryActionLabel(selectedAsset.type)}
                      </button>
                    {/if}
                  </div>
                  {#if !llmAvailable}
                    <p class="ocr-llm-hint">{translate('item.llmUnavailableHint')}</p>
                  {/if}
                </div>

                {#if ocr.status === 'running'}
                  {@const ocrStageLabel = getOcrStageLabel(ocr.stage)}
                  <progress class="ocr-progress" value={ocr.progress} max="100">
                    {ocr.progress}%
                  </progress>
                  <p class="ocr-status-text">
                    {ocrStageLabel
                      ? translate('item.extractionRunningStage', {
                          progress: ocr.progress,
                          stage: ocrStageLabel,
                        })
                      : translate('item.extractionRunning', { progress: ocr.progress })}
                  </p>
                {:else if ocr.status === 'pending'}
                  <p class="ocr-status-text">{translate('item.extractionStarting')}</p>
                {:else if ocr.status === 'error'}
                  <p class="ocr-error">
                    {translate('item.extractionFailed', { error: ocr.error ?? '' })}
                  </p>
                {:else if ocr.status === 'done'}
                  {@const editedText = (() => {
                    void ocrTick
                    return ocrEditedText.get(selectedAsset.id) ?? ocr.textContent ?? ''
                  })()}
                  {@const displayLength = editedText.length}
                  <details class="ocr-result">
                    <summary>
                      {translate('item.extractedText')}
                      <span class="ocr-meta">
                        via {ocr.method ?? translate('item.ocrMethodUnknown')} · {translate(
                          'item.characters',
                          { count: displayLength }
                        )}
                      </span>
                    </summary>
                    <textarea
                      class="ocr-result-body ocr-textarea"
                      rows="8"
                      oninput={(e) => {
                        const val = e.currentTarget.value
                        ocrEditedText.set(selectedAsset.id, val)
                        ocrStore.setTextContent(selectedAsset.id, val)
                        schedulePersist(selectedAsset.id, val)
                        ocrTick++
                      }}>{editedText}</textarea
                    >
                  </details>
                {/if}
              </div>
            </section>
          {/if}

          {#if selectedAsset && selectedAsset.type === 'audio'}
            {@const ts = getTranscriptionState(selectedAsset.id)}
            {@const busy = ts.status === 'pending' || ts.status === 'running'}
            <section class="section">
              <h3>
                {translate('item.audioTranscription')}{#if assets.length > 1}
                  {translate('item.pageInline', { page: selectedAssetIndex + 1 })}{/if}
              </h3>
              <div class="ocr-item">
                <div class="ocr-item-header">
                  <span class="ocr-filename"
                    >&#x1f50a; {selectedAsset.path.split(/[/\\]/).pop() ??
                      translate('item.audioLabel')}</span
                  >
                  <div class="ocr-btn-group">
                    <button
                      class="ocr-btn"
                      disabled={busy}
                      onclick={() => handleTranscribeAudio(selectedAsset)}
                      title={busy
                        ? translate('item.transcribeBusyTitle')
                        : translate('item.transcribeTitle')}
                    >
                      {getTranscriptionActionLabel(busy)}
                    </button>
                    {#if llmAvailable}
                      <button
                        class="ocr-btn ocr-btn--summarize"
                        disabled={getLlmState().status === 'running' || ts.status !== 'done'}
                        onclick={handleLlmSummarize}
                        title={!llmAvailable
                          ? translate('item.summaryUnavailable')
                          : ts.status !== 'done'
                            ? translate('item.summaryNeedsText')
                            : translate('item.summaryTitle')}
                      >
                        {getSummaryActionLabel(selectedAsset.type)}
                      </button>
                    {/if}
                  </div>
                </div>

                {#if ts.status === 'running'}
                  <progress class="ocr-progress" value={ts.progress} max="100">
                    {ts.progress}%
                  </progress>
                  <p class="ocr-status-text">
                    {translate('item.transcriptionRunning', { progress: ts.progress })}
                    {#if getTranscriptionStageLabel(ts.stage)}
                      · {getTranscriptionStageLabel(ts.stage)}
                    {/if}
                  </p>
                {:else if ts.status === 'pending'}
                  <p class="ocr-status-text">{translate('item.transcriptionStarting')}</p>
                {:else if ts.status === 'error'}
                  <p class="ocr-error">
                    {translate('item.transcriptionFailed', { error: ts.error ?? '' })}
                  </p>
                {:else if ts.status === 'done'}
                  {@const editedText = transEditedText.get(selectedAsset.id) ?? ts.text ?? ''}
                  {@const displayLength = editedText.length}
                  <details class="ocr-result">
                    <summary>
                      {translate('item.transcription')}
                      <span class="ocr-meta">
                        {#if ts.language}{ts.language} &middot;
                        {/if}{translate('item.characters', { count: displayLength })}
                        {#if ts.durationMs}
                          &middot; {translate('item.audioDurationSeconds', {
                            count: Math.round(ts.durationMs / 1000),
                          })}{/if}
                      </span>
                    </summary>
                    <textarea
                      class="ocr-result-body ocr-textarea"
                      rows="8"
                      oninput={(e) => {
                        const val = e.currentTarget.value
                        transEditedText.set(selectedAsset.id, val)
                        transcriptionStore.setTextContent(selectedAsset.id, val)
                        scheduleTranscriptionPersist(selectedAsset.id, val)
                        transcriptionTick++
                      }}>{editedText}</textarea
                    >
                  </details>
                {/if}
              </div>
            </section>
          {/if}

          {#if selectedAsset}
            {@const currentSummary = (() => {
              void summaryTick
              return summaryTexts.get(selectedAsset.id) ?? null
            })()}
            {@const isSummarizing =
              getLlmState().status === 'running' && getLlmState().activeJob === 'summarize'}
            {#if currentSummary || isSummarizing}
              <section class="section">
                <h3>
                  {translate('item.summary')}{#if assets.length > 1}
                    {translate('item.pageInline', { page: selectedAssetIndex + 1 })}{/if}
                </h3>
                {#if isSummarizing}
                  <p class="summary-status">{translate('item.generatingSummary')}</p>
                {:else if currentSummary}
                  <div class="summary-result">
                    <pre class="summary-text">{currentSummary}</pre>
                  </div>
                {/if}
              </section>
            {/if}
          {/if}
        </div>

        <div class="right-panel-pane" class:is-hidden={rightPanelTab !== 'analysis'}>
          {#if assets.length > 0}
            {@const nlp = getNlpState()}
            <section class="section">
              <div class="analysis-panel analysis-panel--tabbed">
                <div class="nlp-actions">
                  <button
                    class="nlp-btn"
                    disabled={nlp.fts === 'pending' || nlp.fts === 'running'}
                    onclick={handleIndexFts}
                  >
                    {translate('item.indexAction')}
                    <span class="nlp-badge nlp-badge--{nlp.fts}">{nlp.fts}</span>
                  </button>

                  <button
                    class="nlp-btn"
                    disabled={!selectedAsset || nlp.embed === 'pending' || nlp.embed === 'running'}
                    onclick={handleEmbedAsset}
                  >
                    {translate('item.embedAction')}
                    <span class="nlp-badge nlp-badge--{nlp.embed}">{nlp.embed}</span>
                  </button>

                  <button
                    class="nlp-btn"
                    disabled={nlp.ner === 'pending' || nlp.ner === 'running'}
                    onclick={handleExtractEntities}
                  >
                    {translate('item.nerAction')}
                    <span class="nlp-badge nlp-badge--{nlp.ner}">{nlp.ner}</span>
                  </button>

                  <button
                    class="nlp-btn"
                    disabled={!llmAvailable ||
                      nlp.triples === 'pending' ||
                      nlp.triples === 'running'}
                    onclick={handleLlmExtractTriples}
                  >
                    {translate('item.triplesAction')}
                    <span class="nlp-badge nlp-badge--{nlp.triples}">{nlp.triples}</span>
                  </button>
                </div>

                {#if nlp.errors?.embed}
                  <p class="ocr-error">
                    {translate('item.embeddingError', { error: nlp.errors.embed })}
                  </p>
                {/if}

                {#if !selectedAsset}
                  <p class="empty-text">
                    {translate('item.analysisNeedAsset')}
                  </p>
                {/if}

                <div class="geo-section">
                  <MapViewer
                    markers={geoMarkers}
                    height="280px"
                    visible={rightPanelTab === 'analysis'}
                  />
                </div>

                <div class="entities-section">
                  <h4>{translate('item.entities')}</h4>
                  <EntityViewer
                    {entities}
                    {editingEntityId}
                    editingValue={editingEntityValue}
                    labels={{
                      editValueAria: translate('item.entityEditValueAria'),
                      deleteEntityAria: (value: string) =>
                        translate('item.entityDeleteAria', { value }),
                    }}
                    onentityclick={startEditingEntity}
                    oneditvaluechange={handleEditingEntityValueChange}
                    onsaveentity={handleSaveEntity}
                    oncancelentityedit={cancelEditingEntity}
                    ondeleteentity={handleDeleteEntity}
                  />

                  <div class="entity-editor">
                    <h5>{translate('item.manualEntities')}</h5>
                    <p class="entity-editor__hint">
                      {translate('item.entityHint')}
                    </p>

                    <div class="entity-editor__create">
                      <select
                        value={newEntityType}
                        aria-label={translate('item.newEntityType')}
                        onchange={(event) => {
                          newEntityType = event.currentTarget.value as EditableEntityType
                        }}
                      >
                        {#each EDITABLE_ENTITY_TYPES as type}
                          <option value={type}>{type.toUpperCase()}</option>
                        {/each}
                      </select>
                      <input
                        bind:value={newEntityValue}
                        type="text"
                        placeholder={translate('item.newEntityValue')}
                        aria-label={translate('item.newEntityValue')}
                        onkeydown={(event) => event.key === 'Enter' && void handleCreateEntity()}
                      />
                      <button type="button" class="nlp-btn" onclick={handleCreateEntity}
                        >{translate('item.addEntity')}</button
                      >
                    </div>

                    {#if entityActionError}
                      <p class="error">{entityActionError}</p>
                    {/if}
                  </div>
                </div>

                <div class="triples-section">
                  <h4>
                    {translate('item.semanticTriples')}{#if assets.length > 1}
                      {translate('item.pageInline', { page: selectedAssetIndex + 1 })}{/if}
                  </h4>
                  {#if triples.length === 0}
                    <p class="empty-text">
                      {translate('item.noTriples', {
                        suffix: assets.length > 1 ? translate('item.noTriplesPageSuffix') : '',
                      })}
                    </p>
                  {:else}
                    <ul class="triples-list">
                      {#each triples as triple, i (`${triple.subject}-${triple.predicate}-${triple.object}-${i}`)}
                        <li class="triple-item">
                          <span class="triple-cell">{triple.subject}</span>
                          <span class="triple-cell">{triple.predicate}</span>
                          <span class="triple-cell">{triple.object}</span>
                        </li>
                      {/each}
                    </ul>
                  {/if}
                </div>
              </div>
            </section>
          {/if}
        </div>

        <div class="right-panel-pane" class:is-hidden={rightPanelTab !== 'search'}>
          {#if assets.length > 0}
            <section class="section">
              <div class="analysis-panel analysis-panel--tabbed">
                <div class="fts-search-section">
                  <h4>{translate('item.searchBySimilarText')}</h4>
                  <input
                    class="fts-search-input"
                    type="search"
                    placeholder={translate('item.ftsPlaceholder')}
                    value={ftsQuery}
                    oninput={handleFtsInput}
                    onkeydown={handleFtsKeydown}
                  />

                  {#if ftsSearchError}
                    <p class="ocr-error">{ftsSearchError}</p>
                  {:else if ftsSearching}
                    <p class="empty-text">{translate('item.ftsSearching')}</p>
                  {:else if ftsQuery.trim().length === 0}
                    <p class="empty-text">{translate('item.ftsPrompt')}</p>
                  {:else if ftsResults.length === 0}
                    <p class="empty-text">{translate('item.ftsNoResults')}</p>
                  {:else}
                    <ul class="similar-list">
                      {#each ftsResults as result (result.itemId)}
                        <li class="similar-item">
                          <button
                            class="similar-item-btn"
                            onclick={() => navigateToSimilarItem(result)}
                          >
                            <span class="similar-title">
                              {#each splitHighlightedSegments(result.title || result.itemId, ftsQuery) as segment, i (`${result.itemId}-seg-${i}-${segment.text}`)}
                                {#if segment.isMatch}
                                  <mark class="fts-match">{segment.text}</mark>
                                {:else}
                                  {segment.text}
                                {/if}
                              {/each}
                            </span>
                            <span class="similar-score"
                              >{translate('item.rank', { value: result.rank.toFixed(3) })}</span
                            >
                          </button>
                        </li>
                      {/each}
                    </ul>
                  {/if}

                  {#if isDev}
                    <details class="fts-debug-panel">
                      <summary>{translate('item.ftsDebugTitle')}</summary>

                      <div class="fts-debug-grid">
                        <div class="fts-debug-row">
                          <span class="fts-debug-label"
                            >{translate('item.ftsDebug.indexedRows')}</span
                          >
                          <code>{ftsIndexedRows ?? 'unknown'}</code>
                        </div>
                        <div class="fts-debug-row">
                          <span class="fts-debug-label">{translate('item.ftsDebug.rawQuery')}</span>
                          <code>{ftsDebug?.rawQuery ?? (ftsQuery.trim() || '—')}</code>
                        </div>
                        <div class="fts-debug-row">
                          <span class="fts-debug-label">{translate('item.ftsDebug.sanitized')}</span
                          >
                          <code>{ftsDebug?.sanitizedQuery || '—'}</code>
                        </div>
                        <div class="fts-debug-row">
                          <span class="fts-debug-label">{translate('item.ftsDebug.strategy')}</span>
                          <code>{ftsDebug?.strategy ?? '—'}</code>
                        </div>
                        <div class="fts-debug-row">
                          <span class="fts-debug-label">{translate('item.ftsDebug.dbMatches')}</span
                          >
                          <code>{ftsDebug?.matchCount ?? 0}</code>
                        </div>
                        <div class="fts-debug-row">
                          <span class="fts-debug-label"
                            >{translate('item.ftsDebug.hydratedItems')}</span
                          >
                          <code>{ftsDebug?.hydratedCount ?? 0}</code>
                        </div>
                        <div class="fts-debug-row fts-debug-row--stacked">
                          <span class="fts-debug-label">{translate('item.ftsDebug.resultIds')}</span
                          >
                          <code>{ftsDebug?.resultIds.join(', ') || '—'}</code>
                        </div>
                      </div>
                    </details>
                  {/if}
                </div>

                {#if similarAssets.length > 0}
                  <div class="similar-section">
                    <h4>
                      {assets.length > 1
                        ? translate('item.similarAssetsPage', { page: selectedAssetIndex + 1 })
                        : translate('item.similarAssets')}
                    </h4>
                    <ul class="similar-list">
                      {#each similarAssets.slice(0, 5) as asset (asset.assetId)}
                        <li class="similar-item">
                          <button
                            class="similar-item-btn"
                            onclick={() => navigateToSimilarItem(asset)}
                            data-testid={`similar-asset-${asset.assetId}`}
                          >
                            <span class="similar-item-main">
                              <span class="similar-title">{asset.title || asset.itemId}</span>
                              <span class="similar-meta">
                                {getAssetTypeLabel(asset.assetType)} · {getAssetPathLabel(
                                  asset.assetPath
                                )}
                              </span>
                              <span class="similar-meta">
                                {translate('item.assetMetaLine', {
                                  assetId: asset.assetId,
                                  itemId: asset.itemId,
                                  collectionId: asset.collectionId,
                                })}
                              </span>
                              {#if asset.assetPath && getAssetPathLabel(asset.assetPath) !== asset.assetPath}
                                <span class="similar-meta similar-meta--path"
                                  >{asset.assetPath}</span
                                >
                              {/if}
                            </span>
                            <span class="similar-score">{(asset.similarity * 100).toFixed(1)}%</span
                            >
                          </button>
                        </li>
                      {/each}
                    </ul>
                  </div>
                {:else}
                  <div class="similar-section">
                    <h4>
                      {assets.length > 1
                        ? translate('item.similarAssetsPage', { page: selectedAssetIndex + 1 })
                        : translate('item.similarAssets')}
                    </h4>
                    <p class="empty-text">
                      {#if selectedAsset}
                        {translate('item.similarAssetsEmpty')}
                      {:else}
                        {translate('item.similarAssetsNeedSelection')}
                      {/if}
                    </p>
                  </div>
                {/if}
              </div>
            </section>
          {/if}
        </div>
      </div>
    </div>
    {/if}
  </div>
{/if}

<style>
  .item-view {
    display: grid;
    /* grid-template-columns set via inline style */
    gap: var(--space-3);
    height: 100%;
  }
  .left-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    overflow-y: auto;
    padding: var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-xl);
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.02), rgba(255, 255, 255, 0.004) 52%),
      var(--color-surface-glass);
    box-shadow: var(--shadow-sm);
  }
  .left-panel-tabs {
    display: flex;
    gap: 0;
    border-bottom: 1px solid var(--color-border-subtle);
  }
  .left-panel-tab {
    flex: 1;
    padding: var(--space-2) var(--space-3);
    border: none;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    cursor: pointer;
    transition: color var(--transition-base), border-color var(--transition-base);
  }
  .left-panel-tab:hover {
    color: var(--color-text-primary);
  }
  .left-panel-tab.active {
    color: var(--color-text-primary);
    border-bottom-color: var(--color-accent);
  }
  .left-panel-content {
    position: relative;
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 1;
  }
  .left-panel-pane {
    min-height: 0;
  }
  .left-panel-pane.is-hidden {
    display: none;
  }
  .left-panel-pane--text {
    flex: 1;
    padding: 0 var(--space-2);
    min-height: 0;
  }
  .left-text-panel-section {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  .left-text-panel-card {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    gap: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--color-border-subtle);
    border-radius: 4px;
    background: var(--color-surface);
  }
  .left-text-panel-meta {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
  }
  .left-text-panel-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-3);
    border: 1px solid var(--color-border-subtle);
    border-radius: 4px;
    background: var(--color-surface-sunken);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .right-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    overflow: hidden;
    padding: 0;
    min-height: 0;
  }
  .right-panel-tabs {
    display: flex;
    flex-wrap: wrap;
    gap: 0;
    padding: 0 var(--space-2);
    border-bottom: 1px solid var(--color-border-subtle);
  }
  .right-panel-tab {
    padding: var(--space-2) var(--space-3);
    border: none;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    cursor: pointer;
    transition: color var(--transition-base), border-color var(--transition-base);
  }
  .right-panel-tab:hover {
    color: var(--color-text-primary);
  }
  .right-panel-tab.active {
    color: var(--color-accent);
    border-bottom-color: var(--color-accent);
  }
  .right-panel-tab.active::after {
    display: none;
  }
  .right-panel-content {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    background: var(--color-surface);
    border: 1px solid var(--color-border-subtle);
    border-radius: 4px;
  }
  .right-panel-pane {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    height: 100%;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-2);
  }
  .right-panel-pane.is-hidden {
    display: none;
  }
  .analysis-panel--tabbed {
    border-top: 1px solid var(--color-border);
    border-radius: var(--radius-md);
  }
  .item-header {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--color-border-subtle);
  }
  .item-header__eyebrow {
    font-family: var(--font-mono);
    font-size: 0.6rem;
    font-weight: var(--font-weight-normal);
    letter-spacing: 0.15em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }
  .right-panel-toggle {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    flex-shrink: 0;
    border: none;
    background: var(--color-surface-sunken);
    border-left: 1px solid var(--color-border-subtle);
    border-right: 1px solid var(--color-border-subtle);
    color: var(--color-text-muted);
    cursor: pointer;
    transition: color var(--transition-base), background-color var(--transition-base);
  }
  .right-panel-toggle:hover {
    color: var(--color-accent);
    background: var(--color-accent-soft);
  }
  .resize-handle {
    width: 6px;
    position: relative;
    cursor: col-resize;
    z-index: 1;
  }
  .resize-handle::before {
    content: '';
    position: absolute;
    top: 0;
    bottom: 0;
    left: 50%;
    transform: translateX(-50%);
    width: 1px;
    background-color: var(--color-border);
    transition:
      background-color 0.15s ease,
      width 0.15s ease;
  }
  .resize-handle:hover::before {
    background-color: var(--color-text-muted, var(--color-border));
    width: 2px;
  }
  :global(body.no-select),
  :global(body.no-select *) {
    cursor: col-resize !important;
    user-select: none !important;
    -webkit-user-select: none !important;
  }
  .item-title {
    font-family: var(--font-display);
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-bold);
    color: var(--color-text-primary);
  }
  .item-header__meta {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }
  .section h3 {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
    margin-bottom: var(--space-1);
  }
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-lg);
    background:
      radial-gradient(circle at top left, rgba(255, 255, 255, 0.028), transparent 26%),
      linear-gradient(180deg, rgba(255, 255, 255, 0.018), transparent 75%),
      var(--color-surface);
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.03),
      0 6px 16px rgba(0, 0, 0, 0.08);
  }
  .metadata-sections {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }
  .metadata-subsection {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .metadata-subsection h4 {
    margin: 0;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
    color: var(--color-text-primary);
  }
  .metadata-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
  }
  .metadata-list__row {
    display: grid;
    grid-template-columns: minmax(0, 0.45fr) minmax(0, 0.55fr);
    gap: var(--space-3);
    padding: var(--space-2) 0;
    border-bottom: 1px solid var(--color-border-subtle);
  }
  .metadata-list__row:last-child {
    border-bottom: none;
  }
  .metadata-list dt {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.02em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }
  .metadata-list dd {
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
    overflow-wrap: anywhere;
  }
  .layout-section-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .layout-meta {
    margin: var(--space-1) 0 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }
  .layout-help {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }
  .layout-page-toolbar {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
  .layout-page-summary {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }
  .layout-page-group {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .layout-page-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px 10px;
    border: 1px solid var(--color-hairline);
    border-radius: 999px;
    background: var(--color-surface);
    color: var(--color-text-primary);
    font-size: var(--font-size-xs);
    cursor: pointer;
    transition:
      border-color 0.15s ease,
      background-color 0.15s ease,
      color 0.15s ease;
  }
  .layout-page-chip:hover,
  .layout-page-chip.active {
    border-color: var(--color-accent);
    background: color-mix(in srgb, var(--color-accent) 10%, var(--color-surface));
  }
  .layout-page-chip__count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 22px;
    padding: 2px 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--color-border) 55%, transparent);
    font-variant-numeric: tabular-nums;
  }
  .layout-filter-toolbar {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
  .layout-filter-group {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
  .layout-filter-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px 10px;
    border: 1px solid var(--color-hairline);
    border-radius: 999px;
    background: var(--color-surface);
    color: var(--color-text-primary);
    font-size: var(--font-size-xs);
    cursor: pointer;
    transition:
      border-color 0.15s ease,
      background-color 0.15s ease,
      color 0.15s ease;
  }
  .layout-filter-chip:hover,
  .layout-filter-chip.active {
    border-color: var(--color-accent);
    background: color-mix(in srgb, var(--color-accent) 10%, var(--color-surface));
  }
  .layout-filter-chip__count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 22px;
    padding: 2px 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--color-border) 55%, transparent);
    font-variant-numeric: tabular-nums;
  }
  .layout-filter-summary {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }
  .layout-toggle {
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
    background: var(--color-surface);
    color: var(--color-text-primary);
    font-size: var(--font-size-xs);
    cursor: pointer;
    white-space: nowrap;
  }
  .layout-toggle:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .layout-block-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-height: 320px;
    overflow: auto;
  }
  .layout-block-item {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
    background: var(--color-surface);
    color: inherit;
    text-align: left;
    cursor: pointer;
    transition:
      border-color 0.15s ease,
      background-color 0.15s ease,
      transform 0.15s ease,
      box-shadow 0.15s ease;
  }
  .layout-block-item:hover,
  .layout-block-item.hovered,
  .layout-block-item.selected {
    border-color: var(--color-accent);
    background: color-mix(in srgb, var(--color-accent) 8%, var(--color-surface));
  }
  .layout-block-item.hovered:not(.selected) {
    border-color: var(--color-warning);
    background: color-mix(in srgb, var(--color-warning) 10%, var(--color-surface));
  }
  .layout-block-item:hover,
  .layout-block-item.hovered {
    transform: translateY(-1px);
  }
  .layout-block-item.selected {
    transform: translateX(2px);
    box-shadow: 0 12px 28px color-mix(in srgb, var(--color-accent) 12%, transparent);
  }
  .layout-block-item.fallback {
    border-style: dashed;
  }
  .layout-block-item.fallback.selected {
    border-color: color-mix(in srgb, var(--color-warning) 65%, var(--color-accent));
    background: color-mix(in srgb, var(--color-warning) 14%, var(--color-surface));
  }
  .layout-block-order {
    flex-shrink: 0;
    min-width: 42px;
    color: var(--color-text-muted);
    font-family: var(--font-mono);
    font-size: var(--font-size-xs);
  }
  .layout-block-content {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .layout-block-heading {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .layout-block-label {
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }
  .layout-block-source-badge,
  .layout-block-page-chip {
    display: inline-flex;
    align-items: center;
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 10px;
    line-height: 1.2;
    border: 1px solid color-mix(in srgb, var(--color-accent) 35%, var(--color-border));
    background: color-mix(in srgb, var(--color-accent) 10%, var(--color-surface));
    color: var(--color-text-secondary);
  }
  .layout-block-source-badge--fallback {
    border-color: color-mix(in srgb, var(--color-warning) 45%, var(--color-border));
    background: color-mix(in srgb, var(--color-warning) 12%, var(--color-surface));
  }
  .layout-block-preview {
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    line-height: 1.4;
    word-break: break-word;
  }
  .layout-inspector {
    margin-top: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-lg);
    background: linear-gradient(
      180deg,
      color-mix(in srgb, var(--color-accent) 4%, var(--color-surface)) 0%,
      var(--color-surface) 100%
    );
  }
  .layout-inspector__header {
    display: flex;
    justify-content: space-between;
    gap: var(--space-3);
    align-items: flex-start;
  }
  .layout-inspector__eyebrow {
    margin: 0 0 4px;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--color-text-muted);
  }
  .layout-inspector h4 {
    margin: 0;
    font-size: var(--font-size-md);
    color: var(--color-text-primary);
  }
  .layout-inspector__actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    justify-content: flex-end;
  }
  .layout-inspector__action {
    padding: 6px 10px;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
    background: var(--color-surface);
    color: var(--color-text-primary);
    font-size: var(--font-size-xs);
    cursor: pointer;
  }
  .layout-inspector__action:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .layout-inspector__grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: var(--space-3);
    margin-top: var(--space-3);
  }
  .layout-inspector__field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .layout-inspector__field--wide {
    grid-column: 1 / -1;
  }
  .layout-inspector__label {
    display: block;
    margin-bottom: 4px;
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--color-text-muted);
  }
  .layout-inspector__grid strong,
  .layout-inspector__grid code {
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
  }
  .layout-inspector__grid p {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    line-height: 1.4;
  }
  .layout-inspector__source {
    display: inline-flex;
    align-items: center;
    width: fit-content;
    padding: 4px 10px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--color-accent) 12%, var(--color-surface));
    border: 1px solid color-mix(in srgb, var(--color-accent) 35%, var(--color-border));
  }
  .layout-inspector__source--fallback {
    background: color-mix(in srgb, var(--color-warning) 14%, var(--color-surface));
    border-color: color-mix(in srgb, var(--color-warning) 45%, var(--color-border));
  }
  .layout-inspector__content {
    margin-top: var(--space-3);
  }
  .layout-inspector__content pre {
    margin: 0;
    padding: var(--space-3);
    max-height: 220px;
    overflow: auto;
    white-space: pre-wrap;
    word-break: break-word;
    border-radius: var(--radius-md);
    border: 1px solid var(--color-hairline);
    background: color-mix(in srgb, var(--color-surface) 88%, black 12%);
    color: var(--color-text-primary);
    font-family: var(--font-sans);
    font-size: var(--font-size-sm);
    line-height: 1.5;
  }
  .layout-inspector__message,
  .layout-inspector__empty {
    margin: var(--space-3) 0 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }
  .layout-inspector__message {
    color: var(--color-success);
  }
  .layout-inspector__message--error {
    color: var(--color-danger);
  }
  .asset-pagination {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-2) 0;
  }
  .pagination-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
    background: var(--color-surface-sunken);
    color: var(--color-text-primary);
    font-size: var(--font-size-md);
    cursor: pointer;
    transition:
      background var(--transition-smooth),
      border-color var(--transition-smooth);
  }
  .pagination-btn:hover:not(:disabled) {
    border-color: var(--color-accent);
    background: var(--color-primary-subtle);
  }
  .pagination-btn:disabled {
    opacity: 0.35;
    cursor: not-allowed;
  }
  .pagination-info {
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    min-width: 60px;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  .empty-viewer {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 300px;
    color: var(--color-text-secondary);
    border: 1px dashed var(--color-hairline);
    border-radius: var(--radius-md);
  }

  /* ── Summary (auto-generated by Gemma 4) ── */
  .summary-result {
    margin-top: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
    background: var(--color-surface-sunken);
  }

  .summary-status {
    font-size: var(--font-size-sm);
    color: var(--color-text-muted);
    font-style: italic;
  }

  .summary-text {
    margin: 0;
    font-size: var(--font-size-sm);
    font-family: var(--font-sans);
    white-space: pre-wrap;
    word-wrap: break-word;
    max-height: 300px;
    overflow-y: auto;
    line-height: 1.6;
    color: var(--color-text-secondary);
  }
  .notes-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .note-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.018), transparent 58%),
      var(--color-surface);
  }
  .note-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    cursor: pointer;
  }
  .note-row:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
    border-radius: var(--radius-sm);
  }
  .note-preview {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--color-text-primary);
    line-height: 1.35;
  }
  .note-content {
    color: var(--color-text-primary);
    line-height: 1.6;
    word-break: break-word;
  }
  .note-expanded {
    padding-top: var(--space-1);
  }
  .note-content--rich :global(p:first-child),
  .note-content--rich :global(h1:first-child),
  .note-content--rich :global(h2:first-child),
  .note-content--rich :global(h3:first-child),
  .note-content--rich :global(blockquote:first-child) {
    margin-top: 0;
  }
  .note-content--rich :global(p:last-child),
  .note-content--rich :global(h1:last-child),
  .note-content--rich :global(h2:last-child),
  .note-content--rich :global(h3:last-child),
  .note-content--rich :global(blockquote:last-child),
  .note-content--rich :global(ul:last-child),
  .note-content--rich :global(ol:last-child) {
    margin-bottom: 0;
  }
  .note-content--rich :global(a) {
    color: var(--color-accent-hover);
    text-decoration: underline;
  }
  .note-content--rich :global(blockquote) {
    margin: var(--space-3) 0;
    padding-left: var(--space-3);
    border-left: 3px solid color-mix(in srgb, var(--color-accent) 45%, var(--color-border));
    color: var(--color-text-secondary);
  }
  .note-content--rich :global(code) {
    background: color-mix(in srgb, var(--color-border) 65%, transparent);
    border-radius: var(--radius-sm);
    padding: 0.1rem 0.3rem;
    font-size: 0.95em;
  }
  .note-content--rich :global(pre) {
    background: var(--color-surface-sunken);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    overflow-x: auto;
  }
  .note-content--rich :global(ul),
  .note-content--rich :global(ol) {
    padding-left: 1.25rem;
  }
  .note-date {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    margin: 0;
  }
  .note-date--inline {
    margin-top: 0;
    white-space: nowrap;
  }
  .note-actions {
    display: flex;
    gap: var(--space-1);
    margin-top: 0;
    align-items: center;
    justify-self: end;
  }
  .note-action-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 1.75rem;
    height: 1.75rem;
    padding: 0;
    border: 0;
    border-radius: 0;
    background: transparent;
    box-shadow: none;
    color: var(--color-text-muted);
    cursor: pointer;
    transition:
      color var(--transition-base),
      opacity var(--transition-base);
  }
  .note-action-button:hover {
    background: transparent;
    box-shadow: none;
  }
  .note-action-button:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
    border-radius: var(--radius-sm);
  }
  .note-action-button--edit {
    color: color-mix(in srgb, var(--color-text-primary) 78%, white);
  }
  .note-action-button--edit:hover {
    color: var(--color-text-primary);
    opacity: 1;
  }
  .note-action-button--delete {
    color: var(--color-text-muted);
    opacity: 0.9;
  }
  .note-action-button--delete:hover {
    color: var(--color-danger);
    opacity: 1;
  }
  .note-edit {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .modal-overlay {
    position: fixed;
    inset: 0;
    background-color: var(--color-overlay);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    padding: var(--space-4);
  }
  .modal {
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.02), transparent 58%),
      var(--color-surface-glass);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-lg);
    padding: var(--space-6);
    max-width: 420px;
    width: 100%;
    box-shadow: var(--shadow-lg);
  }
  .modal-title {
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-bold);
    color: var(--color-text-primary);
    margin: 0 0 var(--space-3) 0;
  }
  .modal-message {
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    margin: 0 0 var(--space-4) 0;
    line-height: 1.5;
  }
  .modal-actions {
    display: flex;
    gap: var(--space-3);
    justify-content: flex-end;
  }
  .modal-secondary-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-height: var(--control-height-sm);
    padding: 0 var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
    background: var(--color-surface-elevated);
    color: var(--color-text-primary);
    cursor: pointer;
    transition:
      background-color var(--transition-smooth),
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth);
  }
  .modal-secondary-button:hover:not(:disabled) {
    background: var(--color-surface);
  }
  .modal-secondary-button:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  .modal-secondary-button:disabled {
    opacity: 0.48;
    cursor: not-allowed;
  }
  .modal-delete-button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--control-height-sm);
    height: var(--control-height-sm);
    padding: 0;
    border: 1px solid var(--color-danger);
    border-radius: var(--radius-md);
    background-color: var(--color-danger);
    color: var(--color-bg);
    cursor: pointer;
    transition:
      background-color var(--transition-smooth),
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth),
      transform var(--transition-smooth);
    box-shadow: 0 8px 18px color-mix(in srgb, var(--color-danger) 18%, transparent);
  }
  .modal-delete-button:hover:not(:disabled) {
    background-color: var(--color-danger-hover);
    border-color: var(--color-danger-hover);
    transform: translateY(-1px);
  }
  .modal-delete-button:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  .modal-delete-button:disabled {
    opacity: 0.48;
    cursor: not-allowed;
    transform: none;
  }
  .empty-text {
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }
  .saving {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    font-weight: normal;
  }
  .status {
    color: var(--color-text-secondary);
    text-align: center;
  }
  .error {
    color: var(--color-danger);
  }

  @media (max-width: 720px) {
    .modal-secondary-button,
    .modal-delete-button {
      width: 100%;
    }

    .modal-actions {
      flex-direction: column-reverse;
    }
  }

  /* ── OCR UI ── */
  .ocr-badge {
    display: inline-block;
    margin-left: var(--space-1);
    padding: 1px 5px;
    font-size: 10px;
    border-radius: var(--radius-sm);
    vertical-align: middle;
    text-transform: uppercase;
    font-weight: var(--font-weight-medium);
    background: var(--color-surface);
    color: var(--color-text-secondary);
    border: 1px solid var(--color-hairline);
  }
  .ocr-badge--running {
    background: var(--color-warning-soft);
    color: var(--color-warning);
  }
  .ocr-badge--pending {
    background: var(--color-info-soft);
    color: var(--color-info);
  }
  .ocr-badge--done {
    background: var(--color-success-soft);
    color: var(--color-success);
  }
  .ocr-badge--error {
    background: var(--color-danger-soft);
    color: var(--color-danger);
  }

  .ocr-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .ocr-item {
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
    padding: var(--space-2);
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.02), transparent 70%),
      var(--color-surface);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.025);
  }
  .ocr-item-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .ocr-filename {
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }
  .ocr-btn {
    padding: var(--space-1) var(--space-2);
    font-size: var(--font-size-xs);
    border: 1px solid var(--color-primary);
    border-radius: var(--radius-sm);
    background: var(--color-primary-subtle);
    color: var(--color-primary);
    cursor: pointer;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .ocr-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
    border-color: var(--color-border);
    background: var(--color-surface);
    color: var(--color-text-muted);
  }
  .ocr-btn-group {
    display: flex;
    gap: var(--space-1);
    flex-shrink: 0;
  }
  .ocr-btn--light {
    border-color: var(--color-success);
    background: var(--color-success-soft);
    color: var(--color-success);
  }
  .ocr-btn--light:disabled {
    border-color: var(--color-border);
    background: var(--color-surface);
    color: var(--color-text-muted);
  }
  .ocr-btn--high {
    border-color: var(--color-info);
    background: var(--color-info-soft);
    color: var(--color-info);
  }
  .ocr-btn--high:disabled {
    border-color: var(--color-border);
    background: var(--color-surface);
    color: var(--color-text-muted);
  }
  .ocr-btn--correct {
    border-color: var(--color-accent);
    background: var(--color-accent-faint);
    color: var(--color-accent);
  }
  .ocr-btn--correct:disabled {
    border-color: var(--color-border);
    background: var(--color-surface);
    color: var(--color-text-muted);
  }
  .ocr-btn--summarize {
    border-color: var(--color-warning);
    background: var(--color-warning-soft);
    color: var(--color-warning);
  }
  .ocr-btn--summarize:disabled {
    border-color: var(--color-border);
    background: var(--color-surface);
    color: var(--color-text-muted);
  }
  .ocr-llm-hint {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    margin: var(--space-1) 0 0;
    font-style: italic;
  }
  .ocr-progress {
    width: 100%;
    height: 6px;
    border-radius: var(--radius-sm);
    appearance: none;
  }
  .ocr-status-text {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }
  .ocr-error {
    font-size: var(--font-size-xs);
    color: var(--color-danger);
  }
  .ocr-meta {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }
  .ocr-result {
    font-size: var(--font-size-sm);
  }
  .ocr-result summary {
    cursor: pointer;
    color: var(--color-text-secondary);
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .ocr-result-body {
    margin-top: var(--space-1);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .ocr-textarea {
    width: 100%;
    min-height: 7rem;
    padding: var(--space-1) var(--space-2);
    font-family: var(--font-mono, ui-monospace, SFMono-Regular, monospace);
    font-size: var(--font-size-sm);
    line-height: 1.5;
    color: var(--color-text-secondary);
    background: var(--color-surface-sunken);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
    resize: vertical;
    white-space: pre-wrap;
    word-break: break-word;
    outline: none;
    transition: border-color 0.15s ease;
  }
  .ocr-textarea:focus {
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
  }
  .ocr-textarea:hover:not(:focus) {
    border-color: var(--color-border-hover);
  }

  /* ── Analysis Panel ── */
  .analysis-toggle {
    width: 100%;
    text-align: left;
    padding: var(--space-2) var(--space-3);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
    background: var(--color-surface-raised);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
    cursor: pointer;
  }

  .analysis-toggle:hover {
    border-color: var(--color-text-muted);
  }

  .analysis-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-2);
    border: 1px solid var(--color-hairline);
    border-top: none;
    border-radius: 0 0 var(--radius-md) var(--radius-md);
    overflow: hidden;
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.018), transparent 75%),
      var(--color-surface);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.025);
  }

  .nlp-actions {
    display: flex;
    flex-direction: row;
    gap: var(--space-1);
  }

  .nlp-btn {
    display: inline-flex;
    flex-direction: row;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    flex: 1 1 25%;
    min-width: 0;
    padding: 6px var(--space-1);
    font-size: var(--font-size-xs);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
    background: var(--color-surface);
    cursor: pointer;
    color: var(--color-text-primary);
    font-family: var(--font-sans);
    text-align: center;
    white-space: nowrap;
  }

  .nlp-btn:hover:not(:disabled) {
    border-color: var(--color-accent);
    background: var(--color-surface-raised);
  }

  .nlp-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .nlp-badge {
    font-size: 10px;
    padding: 1px 6px;
    border-radius: var(--radius-full);
    text-transform: uppercase;
    font-weight: var(--font-weight-medium);
    background: var(--color-surface-raised);
    color: var(--color-text-muted);
    border: 1px solid var(--color-hairline);
  }

  .nlp-badge--running {
    background: var(--color-warning-soft);
    color: var(--color-warning);
    border-color: transparent;
  }

  .nlp-badge--pending {
    background: var(--color-info-soft);
    color: var(--color-info);
    border-color: transparent;
  }

  .nlp-badge--done {
    background: var(--color-success-soft);
    color: var(--color-success);
    border-color: transparent;
  }

  .nlp-badge--error {
    background: var(--color-danger-soft);
    color: var(--color-danger);
    border-color: transparent;
  }

  .entities-section,
  .fts-search-section,
  .triples-section,
  .similar-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .entities-section h4,
  .fts-search-section h4,
  .similar-section h4 {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
  }

  .entity-editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-3);
    min-width: 0;
  }

  .entity-editor h5 {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .entity-editor__hint {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .entity-editor__create {
    display: grid;
    grid-template-columns: 35fr 50fr 15fr;
    gap: var(--space-2);
    align-items: center;
    padding-bottom: var(--space-2);
    min-width: 0;
  }

  .entity-editor__create select {
    min-width: 0;
    padding: var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
    background: var(--color-surface);
    color: var(--color-text-primary);
    font-size: var(--font-size-xs);
  }

  .entity-editor__create input {
    min-width: 0;
    padding: var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
    background: var(--color-surface);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
  }

  .entity-editor__create .nlp-btn {
    width: 100%;
    flex-direction: row;
    justify-content: center;
    font-size: var(--font-size-sm);
    padding: var(--space-2) var(--space-3);
  }

  .fts-search-input {
    width: 100%;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
    background: var(--color-surface);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    padding: var(--space-2) var(--space-3);
    outline: none;
    font-family: var(--font-sans);
  }

  .fts-search-input:focus {
    border-color: var(--color-accent);
    box-shadow: 0 0 0 2px color-mix(in srgb, var(--color-accent) 20%, transparent);
  }

  .fts-match {
    background: color-mix(in srgb, var(--color-warning) 30%, transparent);
    color: var(--color-text-primary);
    border-radius: 2px;
    padding: 0 1px;
  }

  .fts-debug-panel {
    border: 1px dashed var(--color-hairline);
    border-radius: var(--radius-sm);
    padding: var(--space-2);
    background: var(--color-surface-raised);
  }

  .fts-debug-panel summary {
    cursor: pointer;
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    font-weight: var(--font-weight-medium);
  }

  .fts-debug-grid {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .fts-debug-row {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: var(--space-2);
    font-size: var(--font-size-xs);
  }

  .fts-debug-row--stacked {
    flex-direction: column;
  }

  .fts-debug-label {
    color: var(--color-text-secondary);
    min-width: 90px;
  }

  .fts-debug-row code {
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--color-text-primary);
    background: var(--color-surface);
    border: 1px solid var(--color-hairline);
    border-radius: 4px;
    padding: 2px 6px;
    flex: 1;
  }

  .similar-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .similar-item {
    padding: 0;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
    background: var(--color-surface-raised);
    transition:
      background var(--transition-smooth),
      border-color var(--transition-smooth);
  }

  .similar-item:hover {
    background: var(--color-surface-elevated);
    border-color: var(--color-accent);
  }

  .similar-item-btn {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    width: 100%;
    padding: var(--space-2);
    background: none;
    border: none;
    color: inherit;
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    cursor: pointer;
    text-align: left;
  }

  .similar-item-btn:hover {
    background: transparent;
  }

  .similar-item-main {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .similar-title {
    color: var(--color-text-primary);
    font-weight: var(--font-weight-medium);
    word-break: break-word;
  }

  .similar-meta {
    color: var(--color-text-secondary);
    word-break: break-word;
  }

  .similar-meta--path {
    color: var(--color-text-tertiary, var(--color-text-secondary));
    opacity: 0.8;
  }

  .similar-score {
    font-size: var(--font-size-xs);
    color: var(--color-text-tertiary, var(--color-text-secondary));
    opacity: 0.9;
    white-space: nowrap;
    margin-left: var(--space-2);
    font-weight: var(--font-weight-medium);
  }

  .triples-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .triple-item {
    display: grid;
    grid-template-columns: 1fr 1fr 1fr;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
    background: var(--color-surface-raised);
  }

  .triple-cell {
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Geo Section */
  .geo-section {
    margin-top: var(--space-4);
    padding-top: var(--space-4);
    border-top: 1px solid var(--color-hairline);
  }

  /* LLM Section */
  .llm-section {
    margin-top: var(--space-4);
    padding-top: var(--space-4);
    border-top: 1px solid var(--color-hairline);
  }

  .llm-btn {
    border-left: 3px solid var(--color-accent);
  }

  .llm-result {
    margin-top: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
    background: var(--color-surface);
  }

  .llm-result-text {
    margin: 0;
    font-size: var(--font-size-sm);
    white-space: pre-wrap;
    word-wrap: break-word;
    max-height: 300px;
    overflow-y: auto;
    line-height: 1.5;
  }
</style>
