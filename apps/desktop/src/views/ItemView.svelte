<script lang="ts">
  import { getStore } from '$lib/db'
  import { deleteAssetFile, duplicateAssetFile, getAssetUrl } from '$lib/file-import'
  import {
    DebouncedMetadataPersistor,
    buildTechnicalMetadata,
    getAssetPathLabel,
    getAssetTypeLabel,
    normalizeMetadataKey,
    parseImportedFileMetadata,
    parseMetadataRecord,
    type ImportedFileMetadata,
  } from '$lib/item-metadata'
  import {
    appendImageEditUndoEntry,
    createImageEditUndoEntry,
    createImageUpdatedPayload,
    discardLatestImageEditUndoEntry,
    getLatestImageEditUndoEntry,
    updateAssetPathInList,
    type ImageEditUndoEntry,
  } from '$lib/item-view-image-edit'
  import {
    DebouncedAnnotationPersistor,
    loadViewerAnnotationsForAsset,
    toAnnotationPersistenceInputs,
  } from '$lib/item-view-annotation-persistence'
  import {
    buildManualEntityCreatePayload,
    buildManualEntityUpdatePayload,
    normalizeManualEntityValue,
    type EditableEntityType,
  } from '$lib/item-view-entities'
  import {
    canCancelDelete,
    getNextExpandedNoteId,
    getNoteStateAfterDelete,
    loadNotesForAssetScope,
  } from '$lib/item-view-notes'
  import { FtsSearchController } from '$lib/item-view-search'
  import { DebouncedAssetTextPersistor } from '$lib/item-view-text-persistence'
  import { LatestRequestGuard } from '$lib/item-view-load-guards'
  import {
    getActiveLlmTarget,
    getErrorMessage,
    isLlmCorrectOcrJob,
    isLlmSummaryJob,
    isLlmTriplesJob,
    runScopedLlmAction,
    selectOcrCorrectionAssetId,
  } from '$lib/item-view-llm-orchestration'
  import {
    cropAnnotations,
    normalizeAnnotationsForAsset,
    normalizedToPixels,
    rotateAnnotations,
  } from '$lib/item-view-geometry'
  import ItemSearchPanel from './ItemSearchPanel.svelte'
  import SimilarAssetPreviewDialog from './SimilarAssetPreviewDialog.svelte'
  import ItemMetadataPanel from './ItemMetadataPanel.svelte'
  import ItemNotesPanel from './ItemNotesPanel.svelte'
  import ItemLayoutPanel from './ItemLayoutPanel.svelte'
  import ItemTextPanel from './ItemTextPanel.svelte'
  import ItemAnalysisPanel from './ItemAnalysisPanel.svelte'
  import ItemMapViewer from './ItemMapViewer.svelte'
  import ItemAssetPanel from './ItemAssetPanel.svelte'
  import {
    buildLayoutBlockViews,
    countLayoutBlocksByFilter,
    filterBlocksByPage,
    filterRegionsByPage,
    filterLayoutBlocksByType,
    findLayoutBlockById,
    getLayoutInteractionStateFromBlockId,
    getLayoutInteractionStateFromRegionId,
    getBlockCountByPage,
    getLayoutByAsset,
    getPagesFromLayout,
    LAYOUT_BLOCK_FILTERS,
    pruneLayoutInteractionSelectionState,
    type LayoutBlockFilterId,
  } from '$lib/layouts'
  import { runPendingAssetJob } from '$lib/item-view-media-jobs'
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
    llmExtractTriples,
    llmSummarizeAsset,
    llmCorrectOcrAsset,
    llmCanRestoreOriginalOcrAsset,
    llmRestoreOriginalOcrAsset,
    llmExtractTriplesAsset,
    llmIsAvailable,
    llmOcrCorrectionIsAvailable,
    llmGetResult,
    llmGetResults,
  } from '$lib/llm'
  import { GeoStore, geocodeEntity } from '$lib/geo'
  import {
    ActionIcon,
    IconButton,
    Panel,
    TabButton,
    TabList,
    isNoteHtmlEffectivelyEmpty,
  } from '@entropia/ui'
  import type { MapMarker } from '@entropia/ui'
  import { onMount, onDestroy, untrack } from 'svelte'
  import { listen, emit } from '@tauri-apps/api/event'
  import { invoke } from '@tauri-apps/api/core'
  import { navigation } from '$lib/navigation'
  import { registerEscapeInterceptor } from '$lib/keyboard'
  import { LOCAL_ML } from '$lib/capabilities'
  import {
    DOCUMENT_ASSET_DELETED_EVENT,
    DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT,
    DOCUMENT_EXPLORER_ASSET_SELECTED_EVENT,
    type DocumentAssetDeletedDetail,
    type DocumentExplorerAssetDetail,
    type DocumentExplorerCollectionChangedDetail,
  } from '$lib/document-explorer'
  import { locale, t, type I18nKey, type I18nParams } from '$lib/i18n'
  import type { Item, Asset, Collection, Note } from '@entropia/store'
  import type {
    Entity,
    ViewerAnnotation,
    ViewerLayoutRegion,
    EditTool,
    ImageEditResult,
  } from '@entropia/ui'
  import { TranscriptionRepo } from '@entropia/store'

  const isDev = import.meta.env.DEV

  type PdfCropResult = {
    path: string
    size: number
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

  let itemViewEl: HTMLElement | undefined = $state()
  let dragCleanup: (() => void) | null = null

  function selectAssetById(assetId: string | null | undefined) {
    if (!assetId) return false
    const nextIndex = assets.findIndex((asset) => asset.id === assetId)
    if (nextIndex < 0) return false
    if (selectedAssetIndex !== nextIndex) {
      selectedAssetIndex = nextIndex
    }
    return true
  }

  function getSelectedAssetBreadcrumbLabel(asset: Asset) {
    return getAssetPathLabel(asset.path)
  }

  function onResizeHandlePointerDown(e: PointerEvent) {
    e.preventDefault()

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
  let lastHandledNavigationAssetId: string | null = null
  let savingMetadata = $state(false)
  let annotations = $state<ViewerAnnotation[]>([])
  let selectedAnnotationId = $state<string | null>(null)
  let annotationTool = $state<'select' | 'rectangle' | 'underline'>('select')
  let annotationColor = $state('var(--color-accent)')
  let annotationSaveError = $state<string | null>(null)

  let assetLayout = $state<Awaited<ReturnType<typeof getLayoutByAsset>>>(null)
  let layoutLoading = $state(false)
  let layoutError = $state<string | null>(null)
  let showLayout = $state(false)
  let layoutTypeFilter = $state<LayoutBlockFilterId>('all')
  let layoutHoveredBlockId = $state<string | null>(null)
  let layoutSelectedBlockId = $state<string | null>(null)
  let layoutHoveredRegionId = $state<string | null>(null)
  let layoutSelectedRegionId = $state<string | null>(null)
  const itemLoadGuard = new LatestRequestGuard()
  const layoutLoadGuard = new LatestRequestGuard()
  const notesLoadGuard = new LatestRequestGuard()
  const selectedAssetStateLoadGuard = new LatestRequestGuard()
  const entitiesLoadGuard = new LatestRequestGuard()
  const geoMarkersLoadGuard = new LatestRequestGuard()
  const triplesLoadGuard = new LatestRequestGuard()
  const similarAssetsLoadGuard = new LatestRequestGuard()
  const ftsSearchLoadGuard = new LatestRequestGuard()
  const llmSummaryLoadGuard = new LatestRequestGuard()
  const ocrRestoreStateLoadGuards = new Map<string, LatestRequestGuard>()
  function ocrRestoreStateLoadGuardFor(assetId: string): LatestRequestGuard {
    const existing = ocrRestoreStateLoadGuards.get(assetId)
    if (existing) return existing
    const guard = new LatestRequestGuard()
    ocrRestoreStateLoadGuards.set(assetId, guard)
    return guard
  }
  let viewerPage = $state(1)
  let viewerTotalPages = $state(1)

  // Image edit state
  let editTool = $state<EditTool>('none')
  let imageVersion = $state(0)

  let undoStack = $state<ImageEditUndoEntry[]>([])
  let redoStack = $state<ImageEditUndoEntry[]>([])
  let editInProgress = $state(false)
  let undoInProgress = $state(false)
  let duplicateAssetInProgress = $state(false)
  let canUndo = $derived(
    undoStack.length > 0 && !editInProgress && !undoInProgress && !duplicateAssetInProgress
  )
  let canRedo = $derived(
    redoStack.length > 0 && !editInProgress && !undoInProgress && !duplicateAssetInProgress
  )
  let lastSelectedAssetId = $state<string | null>(null)
  let lastViewerHistoryPage = $state(1)

  // OCR state — plain TS class, updated via Tauri events
  const ocrStore = new OcrStore({
    onComplete: (assetId, _method, createdPageAssetCount) => {
      const nextEditedText = new Map(ocrEditedText)
      nextEditedText.delete(assetId)
      ocrEditedText = nextEditedText
      const nextCorrectedAssets = new Set(ocrCorrectedAssets)
      nextCorrectedAssets.delete(assetId)
      ocrCorrectedAssets = nextCorrectedAssets
      const nextRestorableAssets = new Set(ocrRestorableAssets)
      nextRestorableAssets.delete(assetId)
      ocrRestorableAssets = nextRestorableAssets
      ocrRestoreStateLoadGuardFor(assetId).next()
      ocrTick++

      if (selectedAsset && selectedAsset.id === assetId) {
        if (createdPageAssetCount) {
          // GLM PDF OCR created child pages; reload so they become selectable.
          void loadData()
        }
        void reloadSelectedAssetPersistedState({ layout: true })
      }
    },
  })
  // Reactive tick counter: incremented on every OCR event to force Svelte re-evaluation
  let ocrTick = $state(0)
  // Edited text per asset — tracks user corrections to OCR output
  let ocrEditedText = $state(new Map<string, string>())

  // Transcription state — mirrors OcrStore pattern for audio assets
  const transcriptionStore = new TranscriptionStore()
  let transcriptionTick = $state(0)

  let transEditedText = $state(new Map<string, string>())

  const PERSIST_IDLE_MS = 500
  // Refresh FTS and embeddings only after the user has
  // been idle for a moment following a manual text correction.
  const REANALYSIS_IDLE_MS = 1500

  // Debounce timers per asset for downstream NLP reprocessing after user inactivity.
  const assetReanalysisTimers = new Map<string, ReturnType<typeof setTimeout>>()

  /**
   * After a manual OCR/transcription edit is persisted, re-run the local
   * search and embeddings for that asset. Debounced so rapid
   * keystrokes coalesce into a single reanalysis pass. This is Pro-local: Lite
   * delegates post-edit reprocessing to the backend, but Pro runs it on the
   * frontend because all inference happens locally.
   */
  function scheduleAssetReanalysis(assetId: string) {
    const existing = assetReanalysisTimers.get(assetId)
    if (existing) clearTimeout(existing)

    const timer = setTimeout(async () => {
      const jobs: Array<[string, () => Promise<unknown>]> = [
        ['fts', () => indexFts(itemId)],
        ['embed', () => embedAsset(itemId, assetId)],
      ]

      try {
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

  function cancelAllAssetReanalysis() {
    for (const timer of assetReanalysisTimers.values()) {
      clearTimeout(timer)
    }
    assetReanalysisTimers.clear()
  }

  const ocrTextPersistor = new DebouncedAssetTextPersistor({
    delayMs: PERSIST_IDLE_MS,
    persist: (assetId, text) =>
      invoke('update_extraction_text_cmd', { assetId, textContent: text }),
    afterPersist: (assetId) => {
      // Keep search and similarity data current after the edit settles.
      scheduleAssetReanalysis(assetId)
    },
    onError: (error) => {
      console.error('[ItemView] Failed to persist OCR correction:', error)
    },
  })

  const isOcrProcessing = (assetId: string | null | undefined) => {
    if (!assetId) return false
    const state = getOcrState(assetId)
    return state.status === 'pending' || state.status === 'running'
  }

  const transcriptionTextPersistor = new DebouncedAssetTextPersistor({
    delayMs: PERSIST_IDLE_MS,
    persist: (assetId, text) =>
      invoke('update_transcription_text_cmd', { assetId, textContent: text }),
    afterPersist: (assetId) => {
      scheduleAssetReanalysis(assetId)
    },
    onError: (error) => {
      console.error('[ItemView] Failed to persist transcription correction:', error)
    },
  })

  const annotationPersistor = new DebouncedAnnotationPersistor({
    delayMs: PERSIST_IDLE_MS,
    persist: async (assetId, page, nextAnnotations) => {
      const saved = await persistAnnotations(assetId, page, nextAnnotations)
      if (!saved) throw new Error('Failed to persist annotations')
    },
    onError: (error) => {
      console.error('[ItemView] Failed to persist annotations:', error)
    },
  })

  /** Save manual OCR edits; search data refreshes after successful persistence. */
  function schedulePersist(assetId: string, text: string) {
    ocrTextPersistor.schedule(assetId, text)
  }

  /** Save manual transcription edits; search data refreshes after successful persistence. */
  function scheduleTranscriptionPersist(assetId: string, text: string) {
    transcriptionTextPersistor.schedule(assetId, text)
  }

  // NLP state — mirrors OcrStore pattern
  const nlpStore = new NlpStore()
  let nlpTick = $state(0)
  let entities = $state<Entity[]>([])
  let newEntityValue = $state('')
  let newEntityType = $state<EditableEntityType>('organization')
  let editingEntityId = $state<string | null>(null)
  let editingEntityValue = $state('')
  let entityActionError = $state<string | null>(null)
  let similarAssets = $state<SimilarAsset[]>([])
  let previewedSimilarAsset = $state<SimilarAsset | null>(null)
  let ftsQuery = $state('')
  let ftsResults = $state<
    Array<{ itemId: string; title: string; rank: number; collectionId: string }>
  >([])
  let ftsSearching = $state(false)
  let ftsSearchError = $state<string | null>(null)
  let ftsIndexedRows = $state<number | null>(null)
  let ftsDebug = $state<{
    rawQuery: string
    sanitizedQuery: string
    strategy: 'empty' | 'strict' | 'relaxed'
    matchCount: number
    hydratedCount: number
    resultIds: string[]
  } | null>(null)
  let triples = $state<
    Array<{ id: string; subject: string; predicate: string; object: string }>
  >([])
  let tripleActionError = $state<string | null>(null)
  let rightPanelTab = $state<
    'notes' | 'text' | 'analysis' | 'map' | 'search' | 'layout' | 'metadata'
  >('notes')
  let rightPanelOpen = $state(true)
  const metadataEditorLabels = $derived.by(() => {
    $currentLocale
    return {
      keyPlaceholder: translate('item.metadataKeyPlaceholder'),
      valuePlaceholder: translate('item.metadataValuePlaceholder'),
      removeFieldAria: translate('item.metadataRemoveField'),
      addField: translate('item.metadataAddField'),
      fieldLabel: translate('item.metadataFieldLabel'),
      valueLabel: translate('item.metadataValueLabel'),
      emptyText: translate('item.metadataEmpty'),
    }
  })

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
      redo: translate('item.toolbar.redo'),
      redoTitle: translate('item.toolbar.redoTitle'),
      duplicateAsset: translate('item.toolbar.duplicateAsset'),
      panTool: translate('item.toolbar.pan'),
      rectangleTool: translate('item.toolbar.rectangle'),
      underlineTool: translate('item.toolbar.underline'),
      cropTool: translate('item.toolbar.crop'),
      eraseTool: translate('item.toolbar.erase'),
      rotateLeft: translate('item.toolbar.rotateLeft'),
      rotateRight: translate('item.toolbar.rotateRight'),
      fineRotateLeft: translate('item.toolbar.fineRotateLeft'),
      fineRotateRight: translate('item.toolbar.fineRotateRight'),
      fineRotationAngle: (degrees: number) =>
        translate('item.toolbar.fineRotationAngle', {
          degrees: `${degrees > 0 ? '+' : ''}${degrees}°`,
        }),
      zoomOut: translate('item.toolbar.zoomOut'),
      zoomIn: translate('item.toolbar.zoomIn'),
      deleteSelected: translate('item.toolbar.deleteAnnotation'),
      colorAriaLabel: (label: string) => translate('item.toolbar.colorAria', { label }),
    }
  })

  async function handleDuplicateAsset() {
    const sourceAsset = selectedAsset
    if (
      !sourceAsset ||
      (sourceAsset.type !== 'image' && sourceAsset.type !== 'pdf') ||
      duplicateAssetInProgress ||
      editInProgress ||
      undoInProgress
    ) {
      return
    }

    duplicateAssetInProgress = true
    error = null
    let copiedPath: string | null = null

    try {
      const duplicate = await duplicateAssetFile(
        sourceAsset.path,
        assets.filter((asset) => asset.type === sourceAsset.type).map((asset) => asset.path)
      )
      copiedPath = duplicate.path

      const createdAsset = await getStore().assets.create({
        itemId: sourceAsset.itemId,
        path: duplicate.path,
        type: sourceAsset.type,
        // Sort after every existing asset so the copy keeps the same position
        // in-session and after reload (orderAssetsForDisplay sorts by sortIndex).
        sortIndex: Math.max(0, ...assets.map((asset) => asset.sortIndex ?? 0)) + 1,
        size: sourceAsset.size,
      })

      assets = [...assets, createdAsset]
      selectedAssetIndex = assets.length - 1
      lastHandledNavigationAssetId = null

      window.dispatchEvent(
        new CustomEvent<DocumentExplorerCollectionChangedDetail>(
          DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT,
          { detail: { collectionId, itemId } }
        )
      )
    } catch (duplicateError) {
      if (copiedPath) {
        try {
          await deleteAssetFile(copiedPath)
        } catch (cleanupError) {
          console.warn('[ItemView] Failed to clean up duplicated asset file:', cleanupError)
        }
      }
      const message =
        duplicateError instanceof Error ? duplicateError.message : String(duplicateError)
      error = translate('item.error.duplicateAsset', { message })
    } finally {
      duplicateAssetInProgress = false
    }
  }

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
      orderedList: translate('item.noteEditor.orderedList'),
      quote: translate('item.noteEditor.quote'),
      addLink: translate('item.noteEditor.addLink'),
      removeLink: translate('item.noteEditor.removeLink'),
      dictationStart: translate('item.noteEditor.dictationStart'),
      dictationStop: translate('item.noteEditor.dictationStop'),
      dictationProcessing: translate('item.noteEditor.dictationProcessing'),
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
  let ocrCorrectedAssets = $state(new Set<string>()) // asset IDs already OCR-corrected — hide OCRC (Pro-local idempotency)
  let ocrRestorableAssets = $state(new Set<string>())
  let restoringOriginalOcrAssetId = $state<string | null>(null)
  let correctingOcrAssetId = $state<string | null>(null)
  const llmStore = new LlmStore({
    onCorrectOcr: (id) => {
      // Fires on live completion AND on persisted-results reload, so the OCRC
      // button stays hidden across reopens once an asset has been corrected.
      ocrCorrectedAssets = new Set(ocrCorrectedAssets).add(id)
    },
    onComplete: (id, job, result) => {
      llmTick++
      // Track summary results in the dedicated map
      if (isLlmSummaryJob(job)) {
        summaryTexts.set(id, result)
        summaryTick++
      }
      // When LLM triples complete, reload triples from DB (they're now in the triples table)
      if (isLlmTriplesJob(job)) {
        loadTriples()
        nlpStore._setJobStatus(itemId, 'triples', 'done')
        nlpTick++
      }
      if (isLlmCorrectOcrJob(job)) {
        if (correctingOcrAssetId === id) {
          correctingOcrAssetId = null
        }
        ocrTick++ // Force Svelte reactivity for the textarea
        const assetId = selectOcrCorrectionAssetId({
          completedTargetId: id,
          selectedAssetId: selectedAsset?.id ?? null,
          assets,
        })
        if (assetId) {
          ocrCorrectedAssets = new Set(ocrCorrectedAssets).add(assetId)
          ocrRestoreStateLoadGuardFor(assetId).next()
          ocrRestorableAssets = new Set(ocrRestorableAssets).add(assetId)
          ocrEditedText.set(assetId, result)
          ocrStore.setTextContent(assetId, result)
          // The backend already replaced the extraction atomically with its
          // original backup and finalized image references.
          scheduleAssetReanalysis(assetId)
        }
      }
    },
    onError: (id, job, jobError) => {
      // When LLM triples extraction fails, set NLP triples status to error
      if (isLlmTriplesJob(job)) {
        nlpStore._setJobStatus(itemId, 'triples', 'error', jobError)
        nlpTick++
      }
      if (isLlmCorrectOcrJob(job)) {
        if (correctingOcrAssetId === id) {
          correctingOcrAssetId = null
        }
        const assetId = selectOcrCorrectionAssetId({
          completedTargetId: id,
          selectedAssetId: selectedAsset?.id ?? null,
          assets,
        })
        const asset = assetId ? assets.find((candidate) => candidate.id === assetId) : null
        if (asset && /stale ocr correction|extraction changed/i.test(jobError)) {
          error = jobError
          void reloadOcrCorrectionState(asset).catch((reloadError) => {
            console.error('[LLM] Failed to reload stale OCR correction state:', reloadError)
          })
        }
      }
    },
  })
  let llmTick = $state(0)

  let llmAvailable = $state(false)
  let ocrCorrectionAvailable = $state(false)
  let summaryTexts = $state(new Map<string, string>()) // assetId → summary text
  let summaryTick = $state(0) // reactivity trigger for summary display

  /**
   * Get the LLM state for the currently active context.
   * When a specific asset/page is selected (multipage), use the asset ID
   * so LLM state is scoped per-page. Otherwise fall back to item ID.
   */
  function getLlmState() {
    void llmTick
    const target = getActiveLlmTarget({ itemId, selectedAssetId: selectedAsset?.id ?? null })
    return llmStore.getState(target.targetId)
  }

  async function handleLlmSummarize() {
    error = null
    try {
      await runScopedLlmAction({
        itemId,
        selectedAssetId: selectedAsset?.id ?? null,
        runAsset: llmSummarizeAsset,
        runItem: llmSummarize,
      })
    } catch (e) {
      console.error('[LLM] summarize failed:', e)
      error = translate('item.error.summarize')
    }
  }

  async function reloadOcrCorrectionState(asset: Asset) {
    const requestToken = selectedAssetStateLoadGuard.next()
    const restoreStateGuard = ocrRestoreStateLoadGuardFor(asset.id)
    const restoreRequestToken = restoreStateGuard.next()
    const [extraction, canRestore, persistedResults] = await Promise.all([
      getStore().extractions.findByAsset(asset.id),
      llmCanRestoreOriginalOcrAsset(asset.id).catch(() => false),
      llmGetResults(asset.id, 'asset').catch(() => []),
    ])
    if (
      !selectedAssetStateLoadGuard.isCurrent(requestToken) ||
      !restoreStateGuard.isCurrent(restoreRequestToken) ||
      !isCurrentSelectedAsset(asset)
    ) {
      return
    }

    const nextEditedText = new Map(ocrEditedText)
    if (extraction) {
      nextEditedText.set(asset.id, extraction.textContent)
      ocrStore._updateState(asset.id, {
        status: 'done',
        progress: 100,
        textLength: extraction.textContent.length,
        method: extraction.method,
        textContent: extraction.textContent,
      })
    } else {
      nextEditedText.delete(asset.id)
    }
    ocrEditedText = nextEditedText

    const nextRestorableAssets = new Set(ocrRestorableAssets)
    if (canRestore) nextRestorableAssets.add(asset.id)
    else nextRestorableAssets.delete(asset.id)
    ocrRestorableAssets = nextRestorableAssets

    const correctedResult = persistedResults.find((result) => result.job_type === 'correct_ocr')
    const nextCorrectedAssets = new Set(ocrCorrectedAssets)
    if (correctedResult) nextCorrectedAssets.add(asset.id)
    else nextCorrectedAssets.delete(asset.id)
    ocrCorrectedAssets = nextCorrectedAssets
    await llmStore.loadPersistedResults(asset.id, 'asset')
    llmTick++
    ocrTick++
  }

  async function handleLlmCorrectOcr() {
    error = null
    const asset = selectedAsset
    if (!asset) {
      error = translate('item.error.correctOcr')
      return
    }
    if (correctingOcrAssetId) return

    correctingOcrAssetId = asset.id
    let enqueued = false
    try {
      try {
        await ocrTextPersistor.flushAndWait(asset.id)
      } catch (persistenceError) {
        console.error('[ItemView] Failed to flush OCR edit before correction:', persistenceError)
        error = getErrorMessage(persistenceError)
        return
      }

      try {
        await llmCorrectOcrAsset(asset.id)
        enqueued = true
      } catch (correctionError) {
        const message = getErrorMessage(correctionError)
        console.error('[LLM] correct OCR failed:', correctionError)
        if (/stale ocr correction|extraction changed/i.test(message)) {
          await reloadOcrCorrectionState(asset)
          error = message
        } else {
          error = translate('item.error.correctOcr')
        }
      }
    } finally {
      if (!enqueued && correctingOcrAssetId === asset.id) {
        correctingOcrAssetId = null
      }
    }
  }

  async function handleRestoreOriginalOcr() {
    const asset = selectedAsset
    if (!asset || !ocrRestorableAssets.has(asset.id) || restoringOriginalOcrAssetId) return
    if (isOcrProcessing(asset.id)) return

    error = null
    restoringOriginalOcrAssetId = asset.id
    try {
      await ocrTextPersistor.cancelAndWait(asset.id)
      const original = await llmRestoreOriginalOcrAsset(asset.id)
      const nextEditedText = new Map(ocrEditedText)
      nextEditedText.set(asset.id, original)
      ocrEditedText = nextEditedText
      ocrStore.setTextContent(asset.id, original)

      const nextCorrectedAssets = new Set(ocrCorrectedAssets)
      nextCorrectedAssets.delete(asset.id)
      ocrCorrectedAssets = nextCorrectedAssets
      const nextRestorableAssets = new Set(ocrRestorableAssets)
      nextRestorableAssets.delete(asset.id)
      ocrRestorableAssets = nextRestorableAssets
      ocrRestoreStateLoadGuardFor(asset.id).next()
      scheduleAssetReanalysis(asset.id)
      ocrTick++
    } catch (restoreError) {
      console.error('[LLM] restore original OCR failed:', restoreError)
      error = translate('item.error.restoreOriginalOcr')
    } finally {
      if (restoringOriginalOcrAssetId === asset.id) {
        restoringOriginalOcrAssetId = null
      }
    }
  }

  async function handleLlmExtractTriples() {
    nlpStore._setJobStatus(itemId, 'triples', 'pending')
    nlpTick++
    try {
      await runScopedLlmAction({
        itemId,
        selectedAssetId: selectedAsset?.id ?? null,
        runAsset: llmExtractTriplesAsset,
        runItem: llmExtractTriples,
      })
    } catch (e) {
      console.error('[LLM] extract triples failed:', e)
      nlpStore._setJobStatus(itemId, 'triples', 'error', getErrorMessage(e))
      nlpTick++
    }
  }

  // Geo state (OpenStreetMap)
  const geoStore = new GeoStore({
    onEntityComplete: () => {
      reloadEntitiesAndGeoMarkers()
    },
    onItemComplete: () => {
      reloadEntitiesAndGeoMarkers()
    },
  })
  let geoMarkers = $state<MapMarker[]>([])

  async function loadGeoMarkers(currentEntities = entities, asset: Asset | null = selectedAsset) {
    const requestToken = geoMarkersLoadGuard.next()
    try {
      const placeEntitiesById = new Map(
        currentEntities
          .filter((entity) => entity.entityType === 'place')
          .map((entity) => [entity.id, entity])
      )

      if (placeEntitiesById.size === 0) {
        if (!geoMarkersLoadGuard.isCurrent(requestToken) || !isCurrentSelectedAsset(asset)) {
          return
        }
        geoMarkers = []
        return
      }

      const rows = await invoke<
        Array<{
          id: string
          value: string
          latitude: number
          longitude: number
          hasManualLocation: number
        }>
      >('db_select', {
        sql: `SELECT id, value,
                     CASE WHEN manual_lat IS NOT NULL AND manual_lon IS NOT NULL THEN manual_lat ELSE latitude END AS latitude,
                     CASE WHEN manual_lat IS NOT NULL AND manual_lon IS NOT NULL THEN manual_lon ELSE longitude END AS longitude,
                     CASE WHEN manual_lat IS NOT NULL AND manual_lon IS NOT NULL THEN 1 ELSE 0 END AS hasManualLocation
              FROM entities
              WHERE item_id = ? AND entity_type = 'place'
              AND ((manual_lat IS NOT NULL AND manual_lon IS NOT NULL)
                   OR (geo_status = 'resolved' AND latitude IS NOT NULL AND longitude IS NOT NULL))
              AND (source IS NULL OR source != 'manual_deleted')`,
        params: [itemId],
      })
      if (!geoMarkersLoadGuard.isCurrent(requestToken) || !isCurrentSelectedAsset(asset)) {
        return
      }
      geoMarkers = rows.flatMap((r) => {
        const entity = placeEntitiesById.get(r.id)
        if (!entity) return []

        return [
          {
            entityId: r.id,
            label: entity.value,
            latitude: r.latitude,
            longitude: r.longitude,
            hasManualLocation: r.hasManualLocation === 1,
          },
        ]
      })
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
  // Stable string key for asset-scoped effects. Image edits replace the asset
  // object (versioned path) while keeping the same ID; effects keyed on this
  // ID must NOT re-fire for those in-place replacements.
  let selectedAssetId = $derived(selectedAsset?.id ?? null)
  let selectedAssetType = $derived(selectedAsset?.type ?? null)
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
  let allAssetsAreImages = $derived(assets.every((asset) => asset.type === 'image'))

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
  let selectedLayoutBlock = $derived(
    findLayoutBlockById(visibleLayoutBlocks, layoutSelectedBlockId)
  )
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
  let textPanelOcrState = $derived(
    selectedAsset && selectedAsset.type !== 'audio' ? getOcrState(selectedAsset.id) : null
  )
  let textPanelOcrEditedText = $derived.by(() => {
    if (!selectedAsset || selectedAsset.type === 'audio') return ''
    const ocr = getOcrState(selectedAsset.id)
    return ocrEditedText.get(selectedAsset.id) ?? ocr.textContent ?? ''
  })
  let textPanelTranscriptionState = $derived(
    selectedAsset && selectedAsset.type === 'audio' ? getTranscriptionState(selectedAsset.id) : null
  )
  let textPanelTranscriptionEditedText = $derived.by(() => {
    if (!selectedAsset || selectedAsset.type !== 'audio') return ''
    const transcription = getTranscriptionState(selectedAsset.id)
    return transEditedText.get(selectedAsset.id) ?? transcription.text ?? ''
  })
  let textPanelLlmState = $derived(getLlmState())
  let textPanelCurrentSummary = $derived.by(() => {
    void summaryTick
    return selectedAsset ? (summaryTexts.get(selectedAsset.id) ?? null) : null
  })
  let textPanelIsSummarizing = $derived(
    textPanelLlmState.status === 'running' && textPanelLlmState.activeJob === 'summarize'
  )
  let selectedAssetHasText = $derived.by(() => {
    if (!selectedAsset) return false
    const text =
      selectedAsset.type === 'audio' ? textPanelTranscriptionEditedText : textPanelOcrEditedText
    return text.trim().length > 0
  })
  let ftsReadinessKey = $derived.by((): I18nKey | null => {
    if (!selectedAsset) return null
    if (!selectedAssetHasText) return 'item.searchReadiness.textNeeded'

    const nlpState = getNlpState()
    if (nlpState.fts !== 'done') return 'item.searchReadiness.ftsIndexNeeded'

    return null
  })
  let similarAssetsReadinessKey = $derived.by((): I18nKey | null => {
    if (!selectedAsset) return null
    if (!selectedAssetHasText) return 'item.searchReadiness.textNeeded'
    if (!llmAvailable) return 'item.searchReadiness.openRouterNeeded'

    const nlpState = getNlpState()
    if (nlpState.embed !== 'done') return 'item.searchReadiness.embeddingNeeded'

    return null
  })

  function syncLayoutHoverFromBlock(blockId: string | null) {
    const nextState = getLayoutInteractionStateFromBlockId(visibleLayoutBlocks, blockId)
    layoutHoveredBlockId = nextState.blockId
    layoutHoveredRegionId = nextState.regionId
  }

  function syncLayoutHoverFromRegion(regionId: string | null) {
    const nextState = getLayoutInteractionStateFromRegionId(visibleLayoutBlocks, regionId)
    layoutHoveredBlockId = nextState.blockId
    layoutHoveredRegionId = nextState.regionId
  }

  function setSelectedLayoutBlock(blockId: string | null) {
    const nextState = getLayoutInteractionStateFromBlockId(visibleLayoutBlocks, blockId)
    layoutSelectedBlockId = nextState.blockId
    layoutSelectedRegionId = nextState.regionId
    if (nextState.hasMatch) {
      showLayout = true
    }
  }

  function setSelectedLayoutRegion(regionId: string | null) {
    const nextState = getLayoutInteractionStateFromRegionId(visibleLayoutBlocks, regionId)
    layoutSelectedBlockId = nextState.blockId
    layoutSelectedRegionId = nextState.regionId
    if (nextState.hasMatch) {
      showLayout = true
    }
  }

  async function persistAnnotations(
    assetId: string,
    page: number,
    nextAnnotations: ViewerAnnotation[]
  ) {
    try {
      const inputs = toAnnotationPersistenceInputs(nextAnnotations)
      await getStore().annotations.replaceForAssetPage(assetId, page, inputs)
      annotationSaveError = null
      return true
    } catch {
      annotationSaveError = 'Failed to save annotations. Changes remain local until retry.'
      return false
    }
  }

  async function flushPendingAnnotationSave() {
    await annotationPersistor.flushPending()
  }

  function scheduleAnnotationPersist(
    assetId: string,
    page: number,
    nextAnnotations: ViewerAnnotation[]
  ) {
    annotationPersistor.schedule(assetId, page, nextAnnotations)
  }

  function handleAnnotationsChange(nextAnnotations: ViewerAnnotation[]) {
    if (!selectedAsset || selectedAsset.type === 'audio') {
      return
    }
    if (selectedAsset.type === 'pdf' && (editInProgress || undoInProgress)) {
      return
    }

    pushCurrentViewerStateToUndo()
    annotations = normalizeAnnotationsForAsset({
      annotations: nextAnnotations,
      assetId: selectedAsset.id,
      page: viewerPage,
      now: Date.now(),
      createId: () => crypto.randomUUID(),
    })
    annotationSaveError = null
    scheduleAnnotationPersist(selectedAsset.id, viewerPage, annotations)
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

  function currentViewerHistoryEntry(): ImageEditUndoEntry | null {
    if (!selectedAsset || selectedAsset.type === 'audio') return null
    return createImageEditUndoEntry({
      path: selectedAsset.path,
      page: viewerPage,
      width: imageNaturalW,
      height: imageNaturalH,
      annotations,
    })
  }

  function pushCurrentViewerStateToUndo() {
    const entry = currentViewerHistoryEntry()
    if (!entry) return
    undoStack = appendImageEditUndoEntry(undoStack, entry)
    redoStack = []
  }

  async function runEditOperation(operation: () => Promise<void>) {
    if (editInProgress || undoInProgress || duplicateAssetInProgress) return
    editInProgress = true
    try {
      await operation()
    } finally {
      editInProgress = false
    }
  }

  function rotateAnnotationsByQuarterTurns(
    sourceAnnotations: ViewerAnnotation[],
    quarterTurns: number
  ) {
    let rotated = sourceAnnotations
    const normalizedTurns = ((quarterTurns % 4) + 4) % 4
    for (let turn = 0; turn < normalizedTurns; turn++) {
      rotated = rotateAnnotations(rotated, 'right')
    }
    return rotated
  }

  async function performPdfEdit({
    operation,
    region = null,
    rotationDegrees,
  }: {
    operation: 'crop' | 'erase' | 'rotate'
    region?: { x: number; y: number; width: number; height: number } | null
    rotationDegrees: number
  }) {
    if (!selectedAsset || selectedAsset.type !== 'pdf') return
    const sourceAsset = selectedAsset
    const sourcePage = viewerPage
    const sourceAnnotations = annotations

    await runEditOperation(async () => {
      let historyEntryAdded = false
      let nextAnnotations: ViewerAnnotation[] | null = null
      const existingCrop = sourceAnnotations.find((annotation) => annotation.kind === 'crop')
      try {
        await flushPendingAnnotationSave()
        pushCurrentViewerStateToUndo()
        historyEntryAdded = true

        const result = await invoke<PdfCropResult>('edit_pdf', {
          path: sourceAsset.path,
          page: sourcePage,
          operation,
          rotationDegrees,
          region,
          existingCrop: existingCrop
            ? {
                x: existingCrop.x,
                y: existingCrop.y,
                width: existingCrop.width,
                height: existingCrop.height,
              }
            : null,
          existingErasures: sourceAnnotations
            .filter((annotation) => annotation.kind === 'erase')
            .map(({ x, y, width, height }) => ({ x, y, width, height })),
        })

        if (selectedAsset?.id !== sourceAsset.id || viewerPage !== sourcePage) {
          undoStack = discardLatestImageEditUndoEntry(undoStack)
          return
        }

        let regularAnnotations = sourceAnnotations.filter(
          (annotation) => annotation.kind === 'rectangle' || annotation.kind === 'underline'
        )
        if (existingCrop) {
          regularAnnotations = cropAnnotations(regularAnnotations, existingCrop)
        }
        const quarterTurns = Math.round(rotationDegrees / 90)
        nextAnnotations = rotateAnnotationsByQuarterTurns(regularAnnotations, quarterTurns)
        if (operation === 'crop' && region) {
          nextAnnotations = cropAnnotations(nextAnnotations, region)
        }
        nextAnnotations = normalizeAnnotationsForAsset({
          annotations: nextAnnotations,
          assetId: sourceAsset.id,
          page: sourcePage,
          now: Date.now(),
          createId: () => crypto.randomUUID(),
        })

        if (!(await persistAnnotations(sourceAsset.id, sourcePage, nextAnnotations))) {
          throw new Error('Failed to persist PDF edit annotations')
        }
        if (selectedAsset?.id !== sourceAsset.id || viewerPage !== sourcePage) {
          await persistAnnotations(sourceAsset.id, sourcePage, sourceAnnotations)
          undoStack = discardLatestImageEditUndoEntry(undoStack)
          return
        }

        const store = getStore()
        await store.assets.updatePath(sourceAsset.id, result.path)
        assets = updateAssetPathInList(assets, sourceAsset.id, result.path)
        imageVersion++

        if (operation === 'crop') {
          const cleanupResults = await Promise.allSettled([
            store.extractions.deleteByAsset(sourceAsset.id),
            store.layouts.deleteByAssetId(sourceAsset.id),
          ])
          for (const cleanup of cleanupResults) {
            if (cleanup.status === 'rejected') {
              console.warn('[ItemView] PDF crop cleanup failed:', cleanup.reason)
            }
          }
        }

        if (selectedAsset?.id !== sourceAsset.id || viewerPage !== sourcePage) {
          return
        }

        annotations = nextAnnotations
        selectedAnnotationId = null
        annotationSaveError = null
      } catch (e) {
        if (nextAnnotations) {
          await persistAnnotations(sourceAsset.id, sourcePage, sourceAnnotations)
          annotationSaveError = 'Failed to apply PDF edit. The previous state was restored.'
        }
        if (historyEntryAdded) {
          undoStack = discardLatestImageEditUndoEntry(undoStack)
        }
        console.error(`[ItemView] PDF ${operation} failed:`, e)
      }
    })
  }

  async function handleEditSelect(region: { x: number; y: number; width: number; height: number }) {
    if (!selectedAsset || selectedAsset.type === 'audio' || editInProgress || undoInProgress) return

    if (selectedAsset.type === 'pdf') {
      const rotation = annotations.find((annotation) => annotation.kind === 'rotation')
      if (editTool === 'crop' || editTool === 'erase') {
        await performPdfEdit({
          operation: editTool,
          region,
          rotationDegrees: Math.round(rotation?.x ?? 0) * 90 + (rotation?.y ?? 0),
        })
      }
      editTool = 'none'
      return
    }

    if (imageNaturalW === 0 || imageNaturalH === 0) return

    const asset = selectedAsset
    const pixelRegion = normalizedToPixels(region, imageNaturalW, imageNaturalH)

    await runEditOperation(async () => {
      await flushPendingAnnotationSave()

      undoStack = appendImageEditUndoEntry(
        undoStack,
        createImageEditUndoEntry({
          path: asset.path,
          page: viewerPage,
          width: imageNaturalW,
          height: imageNaturalH,
          annotations,
        })
      )
      redoStack = []

      try {
        if (editTool === 'crop') {
          const result: ImageEditResult = await invoke('crop_image', {
            path: asset.path,
            x: pixelRegion.x,
            y: pixelRegion.y,
            width: pixelRegion.width,
            height: pixelRegion.height,
          })
          annotations = cropAnnotations(annotations, region)
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
        undoStack = discardLatestImageEditUndoEntry(undoStack)
        console.error('[ItemView] Image edit failed:', e)
      }
    })
    editTool = 'none'
  }

  async function handleRotateLeft() {
    if (!selectedAsset || selectedAsset.type === 'audio' || editInProgress || undoInProgress) return
    if (selectedAsset.type === 'pdf') {
      const rotation = annotations.find((annotation) => annotation.kind === 'rotation')
      await performPdfEdit({
        operation: 'rotate',
        rotationDegrees: Math.round(rotation?.x ?? 0) * 90 + (rotation?.y ?? 0) - 90,
      })
      return
    }
    const asset = selectedAsset

    await runEditOperation(async () => {
      await flushPendingAnnotationSave()
      undoStack = appendImageEditUndoEntry(
        undoStack,
        createImageEditUndoEntry({
          path: asset.path,
          page: viewerPage,
          width: imageNaturalW,
          height: imageNaturalH,
          annotations,
        })
      )
      redoStack = []

      try {
        const result: ImageEditResult = await invoke('rotate_image', {
          path: asset.path,
          page: viewerPage,
          direction: 'left',
        })
        annotations = rotateAnnotations(annotations, 'left')
        await handleImageEditResult(result, asset.id)
      } catch (e) {
        undoStack = discardLatestImageEditUndoEntry(undoStack)
        console.error('[ItemView] Rotate left failed:', e)
      }
    })
  }

  async function handleRotateRight() {
    if (!selectedAsset || selectedAsset.type === 'audio' || editInProgress || undoInProgress) return
    if (selectedAsset.type === 'pdf') {
      const rotation = annotations.find((annotation) => annotation.kind === 'rotation')
      await performPdfEdit({
        operation: 'rotate',
        rotationDegrees: Math.round(rotation?.x ?? 0) * 90 + (rotation?.y ?? 0) + 90,
      })
      return
    }
    const asset = selectedAsset

    await runEditOperation(async () => {
      await flushPendingAnnotationSave()
      undoStack = appendImageEditUndoEntry(
        undoStack,
        createImageEditUndoEntry({
          path: asset.path,
          page: viewerPage,
          width: imageNaturalW,
          height: imageNaturalH,
          annotations,
        })
      )
      redoStack = []

      try {
        const result: ImageEditResult = await invoke('rotate_image', {
          path: asset.path,
          direction: 'right',
        })
        annotations = rotateAnnotations(annotations, 'right')
        await handleImageEditResult(result, asset.id)
      } catch (e) {
        undoStack = discardLatestImageEditUndoEntry(undoStack)
        console.error('[ItemView] Rotate right failed:', e)
      }
    })
  }

  async function handleFineRotateCommit(degrees: number) {
    if (!selectedAsset || selectedAsset.type === 'audio' || editInProgress || undoInProgress) return
    if (!Number.isFinite(degrees)) return

    if (selectedAsset.type === 'pdf') {
      const rotation = annotations.find((annotation) => annotation.kind === 'rotation')
      const rotationDegrees = Math.round(rotation?.x ?? 0) * 90 + degrees
      if (rotationDegrees === 0 && !rotation) return
      await performPdfEdit({ operation: 'rotate', rotationDegrees })
      return
    }
    if (degrees === 0) return

    const asset = selectedAsset

    await runEditOperation(async () => {
      await flushPendingAnnotationSave()
      undoStack = appendImageEditUndoEntry(
        undoStack,
        createImageEditUndoEntry({
          path: asset.path,
          page: viewerPage,
          width: imageNaturalW,
          height: imageNaturalH,
          annotations,
        })
      )
      redoStack = []

      try {
        const result: ImageEditResult = await invoke('rotate_image_degrees', {
          path: asset.path,
          degrees,
        })
        // Free-angle rotation persists the pixels; rectangular annotations remain in
        // their existing normalized coordinate model because arbitrary rotation would
        // require polygon annotations or lossy bounding-box projection.
        await handleImageEditResult(result, asset.id)
      } catch (e) {
        undoStack = discardLatestImageEditUndoEntry(undoStack)
        console.error('[ItemView] Fine rotation failed:', e)
      }
    })
  }

  async function restoreViewerHistoryEntry(
    entry: ImageEditUndoEntry,
    destination: 'undo' | 'redo'
  ) {
    if (!selectedAsset || selectedAsset.type === 'audio') return
    if (entry.page !== viewerPage) return
    const current = currentViewerHistoryEntry()
    if (!current) return

    const assetId = selectedAsset.id
    if (destination === 'redo') {
      redoStack = appendImageEditUndoEntry(redoStack, current)
    } else {
      undoStack = appendImageEditUndoEntry(undoStack, current)
    }

    if (entry.path !== selectedAsset.path) {
      const store = getStore()
      await store.assets.updatePath(assetId, entry.path)
      assets = updateAssetPathInList(assets, assetId, entry.path)

      if (selectedAsset.type === 'image') {
        imageNaturalW = entry.width
        imageNaturalH = entry.height
        imageVersion++
        try {
          await emit(
            'asset:image-updated',
            createImageUpdatedPayload({ itemId, assetId, path: entry.path })
          )
        } catch (e) {
          console.warn('[ItemView] Failed to emit asset:image-updated event:', e)
        }
      }
    }

    annotations = entry.annotations
    selectedAnnotationId = null
    await persistAnnotations(assetId, entry.page, annotations)
  }

  /** Restore the complete viewer state before the latest edit. */
  async function handleUndo() {
    if (!selectedAsset || selectedAsset.type === 'audio') return
    if (editInProgress || undoInProgress || duplicateAssetInProgress) return
    if (undoStack.length === 0) return

    await flushPendingAnnotationSave()

    const entry = getLatestImageEditUndoEntry(undoStack)
    if (!entry) return
    undoStack = discardLatestImageEditUndoEntry(undoStack)
    undoInProgress = true

    try {
      await restoreViewerHistoryEntry(entry, 'redo')
    } catch (e) {
      undoStack = appendImageEditUndoEntry(undoStack, entry)
      console.error('[ItemView] Undo failed:', e)
    } finally {
      undoInProgress = false
    }
  }

  async function handleRedo() {
    if (!selectedAsset || selectedAsset.type === 'audio') return
    if (editInProgress || undoInProgress || duplicateAssetInProgress || redoStack.length === 0)
      return

    await flushPendingAnnotationSave()
    const entry = getLatestImageEditUndoEntry(redoStack)
    if (!entry) return
    redoStack = discardLatestImageEditUndoEntry(redoStack)
    undoInProgress = true

    try {
      await restoreViewerHistoryEntry(entry, 'undo')
    } catch (e) {
      redoStack = appendImageEditUndoEntry(redoStack, entry)
      console.error('[ItemView] Redo failed:', e)
    } finally {
      undoInProgress = false
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
    assets = updateAssetPathInList(assets, assetId, result.path)

    // Force image refresh: bump version counter so the browser fetches the
    // new file (versioned paths already make the URL unique, but this helps
    // if something caches at the protocol level).
    imageVersion++

    if (selectedAsset && selectedAsset.id === assetId) {
      // Adopt the authoritative post-edit dimensions from the backend result.
      // Waiting for the <img> load event leaves a window where a follow-up
      // crop/erase computes pixel coordinates against stale dimensions.
      imageNaturalW = result.width
      imageNaturalH = result.height

      // Persist adjusted annotations
      await persistAnnotations(assetId, viewerPage, annotations)
    }

    // Notify CollectionView (and any other listeners) that the asset path
    // has changed, so they can invalidate their cached thumbnail URLs.
    try {
      await emit(
        'asset:image-updated',
        createImageUpdatedPayload({ itemId, assetId, path: result.path })
      )
    } catch (e) {
      console.warn('[ItemView] Failed to emit asset:image-updated event:', e)
    }
  }

  // Track natural image dimensions for pixel coordinate conversion
  let imageNaturalW = $state(0)
  let imageNaturalH = $state(0)

  const metadataPersistor = new DebouncedMetadataPersistor({
    getItem: () => item,
    updateItem: (id, patch) => getStore().items.update(id, patch),
    onSavingChange: (saving) => {
      savingMetadata = saving
    },
    onError: (message) => {
      error = message
    },
  })

  async function handleExtractText(asset: Asset, mode: OcrMode = 'light') {
    ocrTextPersistor.cancel(asset.id)

    await runPendingAssetJob({
      assetId: asset.id,
      updateState: (assetId, state) => ocrStore._updateState(assetId, state),
      bumpTick: () => {
        ocrTick++
      },
      execute: () => extractText(asset.id, asset.path, asset.type, mode),
      fallbackError: 'Extraction failed',
    })
  }

  async function handleTranscribeAudio(asset: Asset) {
    await runPendingAssetJob({
      assetId: asset.id,
      updateState: (assetId, state) => transcriptionStore._updateState(assetId, state),
      bumpTick: () => {
        transcriptionTick++
      },
      execute: () => transcribeAudio(asset.id, asset.path),
      fallbackError: 'Transcription failed',
    })
  }

  async function handleTranscribeDictation(audio: Blob): Promise<string> {
    return transcribeDictation(audio)
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

  function getNlpState(assetId: string | null = selectedAsset?.id ?? null) {
    void nlpTick
    return nlpStore.getState(itemId, assetId)
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

  let activeAssetSummary = $derived(
    selectedAsset
      ? `${getAssetTypeLabel(selectedAsset.type)} · ${getAssetPathLabel(selectedAsset.path)}`
      : translate('item.assetNoSelection')
  )

  function isCurrentSelectedAsset(asset: Asset | null) {
    return (selectedAsset?.id ?? null) === (asset?.id ?? null)
  }

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

    const assetId = selectedAsset.id
    nlpStore._setJobStatus(itemId, 'embed', 'pending', undefined, assetId)
    nlpTick++
    try {
      await embedAsset(itemId, assetId)
    } catch (e) {
      nlpStore._setJobStatus(
        itemId,
        'embed',
        'error',
        e instanceof Error ? e.message : 'Failed',
        assetId
      )
      nlpTick++
    }
  }

  async function handleExtractEntities() {
    const assetId = selectedAsset?.id ?? null
    nlpStore._setJobStatus(itemId, 'ner', 'pending', undefined, assetId)
    nlpTick++
    try {
      if (assetId) {
        await extractEntitiesForAsset(itemId, assetId)
      } else {
        await extractEntities(itemId)
      }
    } catch (e) {
      nlpStore._setJobStatus(
        itemId,
        'ner',
        'error',
        e instanceof Error ? e.message : 'Failed',
        assetId
      )
      nlpTick++
    }
  }

  async function loadEntities(asset: Asset | null = selectedAsset) {
    const requestToken = entitiesLoadGuard.next()
    try {
      const store = getStore()
      let allEntities: Entity[]
      if (asset) {
        allEntities = (await store.entities.findByAssetId(itemId, asset.id)) as Entity[]
      } else {
        allEntities = (await store.entities.findByItemId(itemId)) as Entity[]
      }
      // Display filter: hide low-confidence automatic entities. The floor is 0.85
      // (inclusive) so the local spaCy NER (which assigns a flat 0.85 trust score)
      // is shown, while genuinely lower-confidence LLM entities stay hidden.
      const nextEntities = allEntities.filter(
        (entity) => entity.confidence == null || entity.confidence >= 0.85
      )
      if (allEntities.length > 0 && nextEntities.length === 0) {
        console.warn('[ItemView] Confidence display filter (>= 0.85) hid all stored entities', {
          storedCount: allEntities.length,
          visibleCount: nextEntities.length,
        })
      }
      if (!entitiesLoadGuard.isCurrent(requestToken) || !isCurrentSelectedAsset(asset)) {
        return null
      }
      entities = nextEntities
      return nextEntities
    } catch {
      if (!entitiesLoadGuard.isCurrent(requestToken) || !isCurrentSelectedAsset(asset)) {
        return null
      }
      // Non-fatal: entities panel shows empty state
      entities = []
      return []
    }
  }

  async function reloadEntitiesAndGeoMarkers(asset: Asset | null = selectedAsset) {
    const nextEntities = await loadEntities(asset)
    if (!nextEntities) return
    await loadGeoMarkers(nextEntities, asset)
  }

  async function handleCreateEntity(): Promise<boolean> {
    const value = normalizeManualEntityValue(newEntityValue)
    if (!value) return false
    try {
      const createdEntity = await getStore().entities.create(
        buildManualEntityCreatePayload({
          itemId,
          assetId: selectedAsset?.id ?? null,
          entityType: newEntityType,
          value,
        })
      )
      if (newEntityType === 'place') {
        await geocodeEntity(createdEntity.id)
      }
      newEntityValue = ''
      newEntityType = 'organization'
      entityActionError = null
      await reloadEntitiesAndGeoMarkers()
      return true
    } catch (e) {
      entityActionError = e instanceof Error ? e.message : 'Failed to add entity'
      return false
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
      await getStore().entities.update(entityId, buildManualEntityUpdatePayload(entity, value))
      if (entity.entityType === 'place') {
        await geocodeEntity(entityId)
      }
      cancelEditingEntity()
      entityActionError = null
      await reloadEntitiesAndGeoMarkers()
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
      await reloadEntitiesAndGeoMarkers()
    } catch (e) {
      entityActionError = e instanceof Error ? e.message : 'Failed to delete entity'
    }
  }

  /**
   * Persiste una tripleta cargada a mano, con el mismo alcance por página que
   * las extraídas: queda atada al asset seleccionado, o al item cuando no hay
   * ninguno. Devuelve `true` solo si la escritura llegó a la base.
   */
  async function handleCreateTriple(draft: {
    subject: string
    predicate: string
    object: string
  }): Promise<boolean> {
    try {
      await getStore().triples.create({
        itemId,
        assetId: selectedAsset?.id ?? null,
        ...draft,
      })
      tripleActionError = null
      await loadTriples()
      return true
    } catch (e) {
      tripleActionError = e instanceof Error ? e.message : 'Failed to add triple'
      return false
    }
  }

  /**
   * Persiste la edición de UNA tripleta. Devuelve `true` solo si la escritura
   * llegó a la base: el panel usa ese booleano para decidir si cierra la fila
   * o la deja abierta con lo que el usuario había tipeado.
   */
  async function handleSaveTriple(
    tripleId: string,
    draft: { subject: string; predicate: string; object: string }
  ): Promise<boolean> {
    try {
      await getStore().triples.update(tripleId, draft)
      tripleActionError = null
      await loadTriples()
      return true
    } catch (e) {
      tripleActionError = e instanceof Error ? e.message : 'Failed to save triple'
      return false
    }
  }

  async function handleDeleteTriple(tripleId: string) {
    try {
      await getStore().triples.delete(tripleId)
      tripleActionError = null
      await loadTriples()
    } catch (e) {
      tripleActionError = e instanceof Error ? e.message : 'Failed to delete triple'
    }
  }

  async function handleSaveMapLocation(entityId: string, latitude: number, longitude: number) {
    await getStore().entities.setManualLocation(entityId, latitude, longitude)
    await loadGeoMarkers()
  }

  async function handleResetMapLocation(entityId: string) {
    await getStore().entities.resetManualLocation(entityId)
    await loadGeoMarkers()
  }

  async function loadSimilarAssets(asset: Asset | null = selectedAsset) {
    const requestToken = similarAssetsLoadGuard.next()
    if (!asset) {
      similarAssets = []
      return
    }

    try {
      const nextSimilarAssets = await fetchSimilarAssets(asset.id, 5)
      if (!similarAssetsLoadGuard.isCurrent(requestToken) || !isCurrentSelectedAsset(asset)) {
        return
      }
      similarAssets = nextSimilarAssets
    } catch {
      if (!similarAssetsLoadGuard.isCurrent(requestToken) || !isCurrentSelectedAsset(asset)) {
        return
      }
      similarAssets = []
    }
  }

  function navigateToFtsItem(item: { itemId: string; title: string; collectionId: string }) {
    navigation.replace({
      name: 'item',
      itemId: item.itemId,
      collectionId: item.collectionId,
      collectionName: '',
      itemTitle: item.title || item.itemId,
    })
  }

  function openSimilarAssetPreview(asset: SimilarAsset) {
    previewedSimilarAsset = asset
  }

  async function loadSimilarAssetFullText(assetId: string): Promise<string | null> {
    const store = getStore()
    const [extraction, transcription] = await Promise.all([
      store.extractions.findByAsset(assetId),
      store.transcriptions.findByAsset(assetId),
    ])
    const parts = [extraction?.textContent, transcription?.textContent]
      .map((text) => text?.trim() ?? '')
      .filter(Boolean)

    return parts.length > 0 ? parts.join('\n\n') : null
  }

  function closeSimilarAssetPreview() {
    previewedSimilarAsset = null
  }

  function resetFtsSearchState() {
    ftsResults = []
    ftsSearchError = null
    ftsSearching = false
    ftsDebug = null
  }

  async function runFtsSearch(rawQuery: string) {
    const requestToken = ftsSearchLoadGuard.next()
    const query = rawQuery.trim()
    if (!query) {
      if (ftsSearchLoadGuard.isCurrent(requestToken)) {
        resetFtsSearchState()
      }
      return
    }

    ftsSearching = true
    ftsSearchError = null

    try {
      const store = getStore()
      if (isDev) {
        const stats = await store.fts.stats()
        if (!ftsSearchLoadGuard.isCurrent(requestToken)) return
        ftsIndexedRows = stats.totalRows
      }

      const response = await store.fts.searchWithDebug(query, 10)
      if (!ftsSearchLoadGuard.isCurrent(requestToken)) return
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

      if (!ftsSearchLoadGuard.isCurrent(requestToken)) return
      ftsResults = hydrated.filter(
        (row): row is { itemId: string; title: string; rank: number; collectionId: string } => !!row
      )

      if (isDev) {
        if (!ftsSearchLoadGuard.isCurrent(requestToken)) return
        ftsDebug = {
          ...response.debug,
          hydratedCount: ftsResults.length,
        }
      }
    } catch {
      if (!ftsSearchLoadGuard.isCurrent(requestToken)) return
      ftsResults = []
      ftsSearchError = 'No se pudo ejecutar la búsqueda full-text.'
      if (isDev) {
        ftsDebug = null
      }
    } finally {
      if (ftsSearchLoadGuard.isCurrent(requestToken)) {
        ftsSearching = false
      }
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

  const ftsSearchController = new FtsSearchController({
    getQuery: () => ftsQuery,
    setQuery: (value) => {
      ftsQuery = value
    },
    reset: resetFtsSearchState,
    search: runFtsSearch,
  })

  function handleFtsInput(event: Event) {
    const value = (event.currentTarget as HTMLInputElement).value
    ftsSearchLoadGuard.invalidate()
    ftsSearchController.handleInput(value)
  }

  function handleFtsKeydown(event: KeyboardEvent) {
    if (event.key === 'Enter' || event.key === 'Escape') {
      ftsSearchLoadGuard.invalidate()
    }
    ftsSearchController.handleKeydown(event)
  }

  function handleFtsClear() {
    ftsSearchLoadGuard.invalidate()
    ftsSearchController.handleInput('')
  }

  async function loadTriples(asset: Asset | null = selectedAsset) {
    const requestToken = triplesLoadGuard.next()
    try {
      const store = getStore()
      const nextTriples = asset
        ? await store.triples.findByAssetId(itemId, asset.id)
        : await store.triples.findByItemId(itemId)
      if (!triplesLoadGuard.isCurrent(requestToken) || !isCurrentSelectedAsset(asset)) {
        return
      }
      triples = nextTriples
    } catch {
      if (!triplesLoadGuard.isCurrent(requestToken) || !isCurrentSelectedAsset(asset)) {
        return
      }
      triples = []
    }
  }

  async function refreshNotesForAsset(asset: Asset | null = selectedAsset) {
    const requestToken = notesLoadGuard.next()
    const loadedNotes = await loadNotesForAsset(asset)
    if (!notesLoadGuard.isCurrent(requestToken) || !isCurrentSelectedAsset(asset)) {
      return false
    }
    notes = loadedNotes
    return true
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
      reloads.push(reloadEntitiesAndGeoMarkers(asset))
    }
    if (options.triples) {
      reloads.push(loadTriples(asset))
    }
    if (options.similarAssets) {
      reloads.push(loadSimilarAssets(asset))
    }

    await Promise.allSettled(reloads)
  }

  function handleMetadataChange(metadata: Record<string, string>) {
    metadataPersistor.schedule(metadata)
  }

  async function handleSaveNote(content: string) {
    const asset = selectedAsset
    try {
      error = null
      const store = getStore()
      await store.notes.create({ itemId, assetId: asset?.id ?? null, content })
      await refreshNotesForAsset(asset)
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to save note'
    }
  }

  let pendingDeleteNoteId = $state<string | null>(null)
  let deletingNote = $state(false)

  async function handleDeleteNote(noteId: string) {
    const asset = selectedAsset
    try {
      error = null
      deletingNote = true
      const store = getStore()
      await store.notes.delete(noteId)
      await refreshNotesForAsset(asset)
      const nextNoteState = getNoteStateAfterDelete(
        { expandedNoteId, editingNoteId, pendingDeleteNoteId },
        noteId
      )
      expandedNoteId = nextNoteState.expandedNoteId
      editingNoteId = nextNoteState.editingNoteId
      pendingDeleteNoteId = nextNoteState.pendingDeleteNoteId
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
    pendingDeleteNoteId = noteId
  }

  function handleDeleteNoteCancel() {
    if (!canCancelDelete(deletingNote)) return
    pendingDeleteNoteId = null
  }

  async function handleDeleteNoteConfirm() {
    if (!pendingDeleteNoteId || deletingNote) return
    await handleDeleteNote(pendingDeleteNoteId)
  }

  function handleEditNote(note: Note) {
    editingNoteId = note.id
  }

  function toggleNoteExpanded(noteId: string) {
    expandedNoteId = getNextExpandedNoteId(expandedNoteId, noteId)
  }

  async function handleSaveEdit(noteId: string, content: string) {
    if (isNoteHtmlEffectivelyEmpty(content)) return
    const asset = selectedAsset
    try {
      error = null
      const store = getStore()
      await store.notes.update(noteId, content)
      await refreshNotesForAsset(asset)
      editingNoteId = null
    } catch (e) {
      error = e instanceof Error ? e.message : 'Failed to update note'
    }
  }

  function handleCancelEdit() {
    editingNoteId = null
  }

  /** Load notes scoped to the current asset (plus item-level notes). */
  async function loadNotesForAsset(asset: Asset | null = selectedAsset): Promise<Note[]> {
    const store = getStore()
    return loadNotesForAssetScope({
      itemId,
      asset,
      findByItem: (itemId) => store.notes.findByItem(itemId),
      findByAsset: (itemId, assetId) => store.notes.findByAsset(itemId, assetId),
    })
  }

  async function loadData() {
    const requestToken = itemLoadGuard.next()
    try {
      loading = true
      error = null
      selectedAssetIndex = 0 // Reset page selection on item change
      lastHandledNavigationAssetId = null
      const store = getStore()
      const [loadedItem, loadedAssets, loadedCollection] = await Promise.all([
        store.items.findById(itemId),
        store.assets.findByItem(itemId),
        store.collections.findById(collectionId),
      ])
      // Discard stale responses: a newer navigation may have started another
      // loadData while this one was awaiting.
      if (!itemLoadGuard.isCurrent(requestToken)) return
      item = loadedItem
      const parentAssetIds = new Set(
        loadedAssets
          .filter((asset) => asset.parentAssetId)
          .map((asset) => asset.parentAssetId as string)
      )
      assets = loadedAssets.filter((asset) => !parentAssetIds.has(asset.id))
      if (correctingOcrAssetId && !assets.some((asset) => asset.id === correctingOcrAssetId)) {
        correctingOcrAssetId = null
      }
      collection = loadedCollection
      if (navigation.current.name === 'item' && navigation.current.itemId === itemId) {
        selectAssetById(navigation.current.assetId)
      }
      // Asset-scoped data (notes, entities, triples, similar assets) will be loaded by the selectedAsset effect
      void loadTopics()
      void loadTopicSuggestions()
    } catch (e) {
      if (!itemLoadGuard.isCurrent(requestToken)) return
      error = e instanceof Error ? e.message : 'Failed to load item'
    } finally {
      // The newer invocation owns `loading`; it set it to true synchronously.
      if (itemLoadGuard.isCurrent(requestToken)) {
        loading = false
      }
    }
  }

  async function reloadLayoutForAsset(asset: Asset | null) {
    const requestToken = layoutLoadGuard.next()

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
      if (!layoutLoadGuard.isCurrent(requestToken) || selectedAsset?.id !== asset.id) {
        return
      }

      assetLayout = layout
      if (!layout || layout.blocks.length === 0) {
        showLayout = false
      }
    } catch (e) {
      if (!layoutLoadGuard.isCurrent(requestToken) || selectedAsset?.id !== asset.id) {
        return
      }

      assetLayout = null
      layoutError = e instanceof Error ? e.message : 'Failed to load layout'
      showLayout = false
    } finally {
      if (layoutLoadGuard.isCurrent(requestToken) && selectedAsset?.id === asset.id) {
        layoutLoading = false
      }
    }
  }

  $effect(() => {
    const currentAssetId = selectedAssetId
    const asset = selectedAssetType === 'pdf' ? untrack(() => selectedAsset) : selectedAsset
    const switchedAsset = currentAssetId !== lastSelectedAssetId
    const switchedPage = !switchedAsset && viewerPage !== lastViewerHistoryPage

    lastSelectedAssetId = currentAssetId
    lastViewerHistoryPage = viewerPage

    if (switchedAsset || switchedPage) {
      annotations = []
      selectedAnnotationId = null
      annotationTool = 'select'
      editTool = 'none'
      if (switchedAsset) {
        viewerPage = 1
        viewerTotalPages = 1
      }
      showLayout = false
      layoutTypeFilter = 'all'
      layoutHoveredBlockId = null
      layoutSelectedBlockId = null
      layoutHoveredRegionId = null
      layoutSelectedRegionId = null
      // Reset undo stack only when switching to a DIFFERENT asset by id.
      // Editing the same asset creates a new versioned path, which should NOT
      // clear undo history.
      undoStack = []
      redoStack = []
    }

    const pendingAnnotationAssetId = annotationPersistor.getPendingAssetId()

    if (pendingAnnotationAssetId !== null && pendingAnnotationAssetId !== currentAssetId) {
      void flushPendingAnnotationSave()
    }

    if (!asset || asset.type === 'audio') {
      annotations = []
      annotationSaveError = null
      return
    }

    let cancelled = false

    void (async () => {
      try {
        annotationSaveError = null
        const loadedAnnotations = await loadViewerAnnotationsForAsset(
          asset.id,
          viewerPage,
          getStore().annotations.findByAsset.bind(getStore().annotations)
        )
        if (!cancelled && selectedAsset?.id === asset.id) {
          annotations = loadedAnnotations
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
    const current = $navigation.current
    assets

    if (current.name !== 'item' || current.itemId !== itemId) return
    const navigationAssetId = current.assetId ?? null
    if (!navigationAssetId || navigationAssetId === lastHandledNavigationAssetId) return
    if (selectAssetById(navigationAssetId)) {
      lastHandledNavigationAssetId = navigationAssetId
    }
  })

  $effect(() => {
    if (loading) return

    const asset = selectedAsset
    const assetLabel = asset ? getSelectedAssetBreadcrumbLabel(asset) : null

    if (
      navigation.current.name === 'item' &&
      navigation.current.itemId === itemId &&
      (navigation.current.assetId !== (asset?.id ?? null) ||
        navigation.current.assetLabel !== assetLabel)
    ) {
      navigation.replace({
        ...navigation.current,
        assetId: asset?.id ?? null,
        assetLabel,
      })
    }

    window.dispatchEvent(
      new CustomEvent<DocumentExplorerAssetDetail>(DOCUMENT_EXPLORER_ASSET_SELECTED_EVENT, {
        detail: {
          itemId,
          assetId: asset?.id ?? null,
          assetLabel,
        },
      })
    )
  })

  $effect(() => {
    void reloadLayoutForAsset(selectedAsset)
  })

  $effect(() => {
    const nextState = pruneLayoutInteractionSelectionState(visibleLayoutBlocks, {
      selectedBlockId: layoutSelectedBlockId,
      selectedRegionId: layoutSelectedRegionId,
      hoveredBlockId: layoutHoveredBlockId,
      hoveredRegionId: layoutHoveredRegionId,
    })

    if (layoutSelectedBlockId !== nextState.selectedBlockId) {
      layoutSelectedBlockId = nextState.selectedBlockId
    }
    if (layoutSelectedRegionId !== nextState.selectedRegionId) {
      layoutSelectedRegionId = nextState.selectedRegionId
    }
    if (layoutHoveredBlockId !== nextState.hoveredBlockId) {
      layoutHoveredBlockId = nextState.hoveredBlockId
    }
    if (layoutHoveredRegionId !== nextState.hoveredRegionId) {
      layoutHoveredRegionId = nextState.hoveredRegionId
    }
  })

  // Reload asset-scoped data when the selected asset changes.
  // Keyed on the asset ID (not the object): editing the same asset swaps the
  // object for one with a new versioned path, which must NOT reset the right
  // panel tab nor reload asset-scoped state. Switching to a DIFFERENT asset
  // (new ID) still resets the tab and refreshes everything below.
  $effect(() => {
    if (!selectedAssetId) return
    const asset = untrack(() => selectedAsset)
    if (!asset) return
    const requestToken = selectedAssetStateLoadGuard.next()

    rightPanelTab = 'notes'

    // Reload notes for this asset (plus item-level notes)
    void refreshNotesForAsset(asset)

    // Load existing extraction text for this asset
    const store = getStore()
    void store.extractions.findByAsset(asset.id).then((extraction) => {
      if (
        selectedAssetStateLoadGuard.isCurrent(requestToken) &&
        isCurrentSelectedAsset(asset) &&
        extraction
      ) {
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
        if (
          selectedAssetStateLoadGuard.isCurrent(requestToken) &&
          isCurrentSelectedAsset(asset) &&
          transcription
        ) {
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

  // Reload analysis data when the selected asset changes (keyed on the asset
  // ID for the same reason as the asset-scoped effect above).
  $effect(() => {
    if (!selectedAssetId) return
    const asset = untrack(() => selectedAsset)
    if (!asset) return
    void reloadEntitiesAndGeoMarkers(asset)
    void loadTriples(asset)
    void loadSimilarAssets(asset)
    // Load persisted LLM results for this asset so previous
    // asset-level results (summarize, correct_ocr, etc.) are visible.
    llmStore.loadPersistedResults(asset.id, 'asset')
    const restoreStateGuard = ocrRestoreStateLoadGuardFor(asset.id)
    const restoreRequestToken = restoreStateGuard.next()
    void llmCanRestoreOriginalOcrAsset(asset.id)
      .then((canRestore) => {
        if (!restoreStateGuard.isCurrent(restoreRequestToken) || !isCurrentSelectedAsset(asset)) {
          return
        }
        const nextRestorableAssets = new Set(ocrRestorableAssets)
        if (canRestore) nextRestorableAssets.add(asset.id)
        else nextRestorableAssets.delete(asset.id)
        ocrRestorableAssets = nextRestorableAssets
      })
      .catch(() => {
        // Legacy corrections without a durable backup are intentionally not restorable.
      })
    const requestToken = llmSummaryLoadGuard.next()
    llmGetResult(asset.id, 'summarize', 'asset')
      .then((result) => {
        if (
          llmSummaryLoadGuard.isCurrent(requestToken) &&
          isCurrentSelectedAsset(asset) &&
          result
        ) {
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
    // Escape first cancels an active editing mode (crop/erase region
    // selection or annotation drawing) instead of navigating back; with no
    // active mode it falls through to the global back-navigation.
    return registerEscapeInterceptor(() => {
      if (editTool !== 'none') {
        editTool = 'none'
        return true
      }
      if (annotationTool !== 'select') {
        annotationTool = 'select'
        return true
      }
      return false
    })
  })

  onMount(() => {
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
      .catch((e) => console.error('[ItemView] OCR listener setup failed:', e))

    nlpStore
      .startListening((eventName, callback) =>
        listen(eventName, callback).then((unlisten) => () => unlisten())
      )
      .then(() => {
        const origSet = nlpStore._setJobStatus.bind(nlpStore)
        nlpStore._setJobStatus = (id, job, status, err, assetId) => {
          origSet(id, job, status, err, assetId)
          nlpTick++
          // After NER completes, reload entities for the current context
          if (job === 'ner' && status === 'done' && id === itemId) {
            void reloadEntitiesAndGeoMarkers()
          }
          if (job === 'embed' && status === 'done' && id === itemId) {
            void reloadSelectedAssetPersistedState({ similarAssets: true })
          }
          if (job === 'triples' && status === 'done' && id === itemId) {
            void reloadSelectedAssetPersistedState({ triples: true })
          }
        }
      })
      .catch((e) => console.error('[ItemView] NLP listener setup failed:', e))

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
      .catch((e) => console.error('[ItemView] Transcription listener setup failed:', e))

    llmStore
      .startListening()
      .then(() => {
        llmStore.onChange(() => {
          llmTick++
          const target = getActiveLlmTarget({ itemId, selectedAssetId: selectedAsset?.id ?? null })
          const llmState = llmStore.getState(target.targetId)
          if (isLlmTriplesJob(llmState.activeJob ?? '') && llmState.status === 'running') {
            nlpStore._setJobStatus(itemId, 'triples', 'running')
            nlpTick++
          }
        })
        // Load persisted LLM results for the item (legacy item-level results).
        // Asset-level results are loaded in the selectedAsset effect below.
        llmStore.loadPersistedResults(itemId, 'item')
      })
      .catch((e) => console.error('[ItemView] LLM listener setup failed:', e))

    llmIsAvailable()
      .then((available) => {
        llmAvailable = available
      })
      .catch(() => {
        llmAvailable = false
      })

    llmOcrCorrectionIsAvailable()
      .then((available) => {
        ocrCorrectionAvailable = available
      })
      .catch(() => {
        ocrCorrectionAvailable = false
      })

    geoStore
      .startListening()
      .catch((e) => console.error('[ItemView] Geo listener setup failed:', e))
    return () => metadataPersistor.cancel()
  })

  onMount(() => {
    const handleAssetDeleted = (event: Event) => {
      const detail = (event as CustomEvent<DocumentAssetDeletedDetail>).detail
      if (detail.itemId !== itemId) return

      const deletedIndex = assets.findIndex((asset) => asset.id === detail.assetId)
      if (deletedIndex < 0) return

      ocrTextPersistor.cancel(detail.assetId)
      transcriptionTextPersistor.cancel(detail.assetId)
      if (annotationPersistor.getPendingAssetId() === detail.assetId) {
        annotationPersistor.cancelAll()
      }

      assets = assets.filter((asset) => asset.id !== detail.assetId)
      if (correctingOcrAssetId === detail.assetId) {
        correctingOcrAssetId = null
      }
      selectedAssetIndex = Math.min(deletedIndex, Math.max(0, assets.length - 1))
      lastHandledNavigationAssetId = null
    }

    window.addEventListener(DOCUMENT_ASSET_DELETED_EVENT, handleAssetDeleted)
    return () => window.removeEventListener(DOCUMENT_ASSET_DELETED_EVENT, handleAssetDeleted)
  })

  onDestroy(() => {
    itemLoadGuard.invalidate()
    layoutLoadGuard.invalidate()
    notesLoadGuard.invalidate()
    selectedAssetStateLoadGuard.invalidate()
    entitiesLoadGuard.invalidate()
    geoMarkersLoadGuard.invalidate()
    triplesLoadGuard.invalidate()
    similarAssetsLoadGuard.invalidate()
    llmSummaryLoadGuard.invalidate()
    ftsSearchLoadGuard.invalidate()
    ocrStore.stopListening()
    nlpStore.stopListening()
    transcriptionStore.stopListening()
    llmStore.stopListening()
    geoStore.stopListening()
    // Clear any pending debounce timers to avoid stale persist after unmount
    ocrTextPersistor.cancelAll()
    transcriptionTextPersistor.cancelAll()
    annotationPersistor.cancelAll()
    cancelAllAssetReanalysis()
    ftsSearchController.cancel()
    metadataPersistor.cancel()
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
    <Panel variant="glass" padding="none" class="left-panel">
      <ItemAssetPanel
        {selectedAsset}
        {viewerSrc}
        {viewerType}
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
        {canRedo}
        {viewerPage}
        {annotationSaveError}
        ocrState={textPanelOcrState}
        ocrEditedText={textPanelOcrEditedText}
        transcriptionState={textPanelTranscriptionState}
        transcriptionEditedText={textPanelTranscriptionEditedText}
        canRestoreOriginalOcr={selectedAsset ? ocrRestorableAssets.has(selectedAsset.id) : false}
        ocrProcessing={isOcrProcessing(selectedAsset?.id)}
        restoringOriginalOcr={restoringOriginalOcrAssetId === selectedAsset?.id}
        onRestoreOriginalOcr={handleRestoreOriginalOcr}
        {documentViewerLabels}
        {annotationToolbarLabels}
        {translate}
        onAnnotationsChange={handleAnnotationsChange}
        onSelectedAnnotationIdChange={handleSelectedAnnotationIdChange}
        onAnnotationToolChange={handleAnnotationToolChange}
        onAnnotationColorChange={handleAnnotationColorChange}
        onLayoutRegionHoverChange={syncLayoutHoverFromRegion}
        onLayoutRegionSelect={setSelectedLayoutRegion}
        onEditSelect={handleEditSelect}
        onEditToolChange={(tool: EditTool) => {
          editTool = tool
          if (tool !== 'none') annotationTool = 'select'
        }}
        onRotateLeft={handleRotateLeft}
        onRotateRight={handleRotateRight}
        onFineRotateCommit={handleFineRotateCommit}
        onUndo={handleUndo}
        onRedo={handleRedo}
        onDuplicateAsset={handleDuplicateAsset}
        duplicateAssetDisabled={duplicateAssetInProgress || editInProgress || undoInProgress}
        onPageChange={(page: number, totalPages: number) => {
          viewerPage = page
          viewerTotalPages = totalPages
        }}
        onDimensionsChange={(dims: { width: number; height: number }) => {
          imageNaturalW = dims.width
          imageNaturalH = dims.height
        }}
      />

      {#if assets.length > 1}
        <div class="asset-pagination">
          <IconButton
            size="sm"
            variant="secondary"
            disabled={selectedAssetIndex <= 0}
            onclick={() => (selectedAssetIndex = Math.max(0, selectedAssetIndex - 1))}
            label={translate('item.previousPage')}
          >
            <ActionIcon name="chevron-left" size={14} />
          </IconButton>
          <span class="pagination-info">
            {selectedAssetIndex + 1} / {assets.length}
          </span>
          <IconButton
            size="sm"
            variant="secondary"
            disabled={selectedAssetIndex >= assets.length - 1}
            onclick={() =>
              (selectedAssetIndex = Math.min(assets.length - 1, selectedAssetIndex + 1))}
            label={translate('item.nextPage')}
          >
            <ActionIcon name="chevron-right" size={14} />
          </IconButton>
        </div>
      {/if}
    </Panel>

    <!-- Right panel toggle -->
    <IconButton
      class="right-panel-toggle"
      variant="ghost"
      size="sm"
      label={rightPanelOpen ? 'Ocultar panel derecho' : 'Mostrar panel derecho'}
      onclick={() => {
        rightPanelOpen = !rightPanelOpen
      }}
      title={rightPanelOpen ? 'Ocultar panel' : 'Mostrar panel'}
    >
      <ActionIcon name={rightPanelOpen ? 'chevron-right' : 'chevron-left'} size={14} />
    </IconButton>

    {#if rightPanelOpen}
      <div
        class="resize-handle"
        role="separator"
        aria-orientation="vertical"
        onpointerdown={onResizeHandlePointerDown}
      ></div>

      <Panel variant="default" padding="none" class="right-panel">
        <header class="item-header">
          <span class="item-header__eyebrow">{translate('item.activeDocument')}</span>
          <h2 class="item-title">{item.title}</h2>
          <p class="item-header__meta">{activeAssetSummary}</p>
        </header>

        {#if error}
          <p class="error">{error}</p>
        {/if}

        <TabList class="right-panel-tabs" aria-label={translate('item.rightPanel')}>
          <TabButton
            active={rightPanelTab === 'notes'}
            class="right-panel-tab"
            onclick={() => {
              rightPanelTab = 'notes'
            }}
          >
            {translate('item.notesTab')}
          </TabButton>
          <TabButton
            active={rightPanelTab === 'text'}
            class="right-panel-tab"
            onclick={() => {
              rightPanelTab = 'text'
            }}
          >
            {translate('item.textTab')}
          </TabButton>
          <TabButton
            active={rightPanelTab === 'analysis'}
            class="right-panel-tab"
            onclick={() => {
              rightPanelTab = 'analysis'
              reloadEntitiesAndGeoMarkers()
              loadTriples()
            }}
          >
            {translate('item.analysisTab')}
          </TabButton>
          <TabButton
            active={rightPanelTab === 'map'}
            class="right-panel-tab"
            onclick={() => {
              rightPanelTab = 'map'
              reloadEntitiesAndGeoMarkers()
            }}
          >
            {translate('item.mapTab')}
          </TabButton>
          <TabButton
            active={rightPanelTab === 'search'}
            class="right-panel-tab"
            onclick={() => {
              rightPanelTab = 'search'
              loadSimilarAssets()
              loadFtsStats()
            }}
          >
            {translate('item.searchTab')}
          </TabButton>
          <TabButton
            active={rightPanelTab === 'layout'}
            class="right-panel-tab"
            onclick={() => {
              rightPanelTab = 'layout'
            }}
          >
            {translate('item.layoutTab')}
          </TabButton>
          <TabButton
            active={rightPanelTab === 'metadata'}
            class="right-panel-tab"
            onclick={() => {
              rightPanelTab = 'metadata'
            }}
          >
            {translate('item.metadataTab')}
          </TabButton>
        </TabList>

        <div class="right-panel-content">
          <div class="right-panel-pane" class:is-hidden={rightPanelTab !== 'notes'}>
            <ItemNotesPanel
              {itemTopics}
              {topicSuggestions}
              assetsCount={assets.length}
              {selectedAssetIndex}
              {notes}
              {editingNoteId}
              {expandedNoteId}
              {pendingDeleteNoteId}
              {deletingNote}
              {noteEditorLabels}
              {translate}
              onTopicsChange={handleTopicsChange}
              onSaveNote={handleSaveNote}
              onTranscribeDictation={handleTranscribeDictation}
              onSaveEdit={handleSaveEdit}
              onCancelEdit={handleCancelEdit}
              onEditNote={handleEditNote}
              onOpenDeleteNoteConfirm={openDeleteNoteConfirm}
              onDeleteNoteCancel={handleDeleteNoteCancel}
              onDeleteNoteConfirm={handleDeleteNoteConfirm}
              onToggleNoteExpanded={toggleNoteExpanded}
            />
          </div>

          <div class="right-panel-pane" class:is-hidden={rightPanelTab !== 'metadata'}>
            <ItemMetadataPanel
              {savingMetadata}
              {fileMetadataEntries}
              {metadataValue}
              {metadataEditorLabels}
              {translate}
              onMetadataChange={handleMetadataChange}
            />
          </div>

          <div class="right-panel-pane" class:is-hidden={rightPanelTab !== 'layout'}>
            <ItemLayoutPanel
              selectedAssetType={selectedAsset?.type ?? null}
              {viewerType}
              {assetLayout}
              {layoutLoading}
              {layoutError}
              {showLayout}
              {layoutActivePage}
              {layoutBlockCountsByPage}
              {layoutBlocks}
              layoutPageRegionCount={layoutPageRegions.length}
              layoutRegionCount={assetLayout?.regions.length ?? 0}
              {layoutPageOptions}
              {layoutTypeFilter}
              {layoutFilterLabels}
              {layoutFilterCounts}
              {layoutPageBlocks}
              {visibleLayoutBlocks}
              {layoutHoveredBlockId}
              {layoutSelectedBlockId}
              {selectedLayoutBlock}
              {hasLayoutData}
              {translate}
              onToggleLayout={(nextShowLayout) => {
                showLayout = nextShowLayout
              }}
              onPageChange={(page) => {
                viewerPage = page
              }}
              onFilterChange={(filter) => {
                layoutTypeFilter = filter
              }}
              onHoverBlock={syncLayoutHoverFromBlock}
              onSelectBlock={setSelectedLayoutBlock}
            />
          </div>

          <div class="right-panel-pane" class:is-hidden={rightPanelTab !== 'text'}>
            <ItemTextPanel
              {selectedAsset}
              assetsCount={assets.length}
              {allAssetsAreImages}
              {selectedAssetIndex}
              ocrState={textPanelOcrState}
              ocrEditedText={textPanelOcrEditedText}
              transcriptionState={textPanelTranscriptionState}
              transcriptionEditedText={textPanelTranscriptionEditedText}
              llmState={textPanelLlmState}
              {llmAvailable}
              {ocrCorrectionAvailable}
              localOcrAvailable={LOCAL_ML}
              isOcrCorrected={selectedAsset ? ocrCorrectedAssets.has(selectedAsset.id) : false}
              ocrEditingDisabled={correctingOcrAssetId === selectedAsset?.id ||
                ((textPanelLlmState.status === 'pending' ||
                  textPanelLlmState.status === 'running') &&
                  textPanelLlmState.activeJob === 'correct_ocr')}
              currentSummary={textPanelCurrentSummary}
              isSummarizing={textPanelIsSummarizing}
              {translate}
              onExtractText={handleExtractText}
              onCorrectOcr={handleLlmCorrectOcr}
              onSummarize={handleLlmSummarize}
              onTranscribeAudio={handleTranscribeAudio}
              onOcrTextInput={(assetId, value) => {
                if (restoringOriginalOcrAssetId === assetId || correctingOcrAssetId === assetId)
                  return
                ocrEditedText.set(assetId, value)
                ocrStore.setTextContent(assetId, value)
                schedulePersist(assetId, value)
                ocrTick++
              }}
              onTranscriptionTextInput={(assetId, value) => {
                transEditedText.set(assetId, value)
                transcriptionStore.setTextContent(assetId, value)
                scheduleTranscriptionPersist(assetId, value)
                transcriptionTick++
              }}
            />
          </div>

          <div class="right-panel-pane" class:is-hidden={rightPanelTab !== 'analysis'}>
            <ItemAnalysisPanel
              assetsCount={assets.length}
              selectedAsset={Boolean(selectedAsset)}
              nlpState={getNlpState()}
              {llmAvailable}
              {geoMarkers}
              visible={rightPanelTab === 'analysis'}
              {entities}
              {editingEntityId}
              {editingEntityValue}
              {newEntityType}
              {newEntityValue}
              {entityActionError}
              {triples}
              {tripleActionError}
              {translate}
              onIndexFts={handleIndexFts}
              onEmbedAsset={handleEmbedAsset}
              onExtractEntities={handleExtractEntities}
              onExtractTriples={handleLlmExtractTriples}
              onEntityClick={startEditingEntity}
              onEditValueChange={handleEditingEntityValueChange}
              onSaveEntity={handleSaveEntity}
              onCancelEntityEdit={cancelEditingEntity}
              onDeleteEntity={handleDeleteEntity}
              onNewEntityTypeChange={(type) => {
                newEntityType = type
              }}
              onNewEntityValueChange={(value) => {
                newEntityValue = value
              }}
              onCreateEntity={handleCreateEntity}
              onCreateTriple={handleCreateTriple}
              onSaveTriple={handleSaveTriple}
              onDeleteTriple={handleDeleteTriple}
              onSaveMapLocation={handleSaveMapLocation}
              onResetMapLocation={handleResetMapLocation}
            />
          </div>

          <div
            class="right-panel-pane right-panel-pane--map"
            class:is-hidden={rightPanelTab !== 'map'}
          >
            {#if rightPanelTab === 'map'}
              <ItemMapViewer
                {entities}
                {geoMarkers}
                height="100%"
                visible={true}
                {translate}
                onSaveMapLocation={handleSaveMapLocation}
                onResetMapLocation={handleResetMapLocation}
              />
            {/if}
          </div>

          <div class="right-panel-pane" class:is-hidden={rightPanelTab !== 'search'}>
            <ItemSearchPanel
              assetsCount={assets.length}
              selectedAsset={Boolean(selectedAsset)}
              {selectedAssetIndex}
              {ftsQuery}
              {ftsResults}
              {ftsSearching}
              {ftsSearchError}
              {ftsIndexedRows}
              {ftsDebug}
              {ftsReadinessKey}
              {similarAssets}
              {similarAssetsReadinessKey}
              {isDev}
              {translate}
              onFtsInput={handleFtsInput}
              onFtsKeydown={handleFtsKeydown}
              onFtsClear={handleFtsClear}
              onNavigateToFtsItem={navigateToFtsItem}
              onPreviewSimilarAsset={openSimilarAssetPreview}
            />
          </div>
        </div>
      </Panel>
    {/if}
  </div>

  {#if previewedSimilarAsset}
    <SimilarAssetPreviewDialog
      asset={previewedSimilarAsset}
      {translate}
      {documentViewerLabels}
      loadFullText={loadSimilarAssetFullText}
      onclose={closeSimilarAssetPreview}
    />
  {/if}
{/if}

<style>
  .item-view {
    display: grid;
    /* grid-template-columns set via inline style */
    gap: var(--space-3);
    height: 100%;
    min-height: 0;
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-dialog);
    background: var(--surface-app);
  }
  :global(.left-panel) {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    overflow-y: auto;
    padding: var(--space-2);
    min-height: 0;
  }
  :global(.right-panel) {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    overflow: hidden;
    padding: 0;
    min-height: 0;
  }
  :global(.right-panel-tabs) {
    display: flex;
    flex-wrap: wrap;
    align-self: stretch;
    margin: 0 var(--space-3);
    background: var(--surface-input);
    border-color: var(--border-subtle);
  }
  :global(.right-panel-tab) {
    flex: 1 1 auto;
    min-width: fit-content;
  }
  .right-panel-content {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    margin: 0 var(--space-3) var(--space-3);
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
  .right-panel-pane--map {
    box-sizing: border-box;
    overflow: hidden;
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
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-normal);
    letter-spacing: 0.15em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }
  :global(.icon-button.right-panel-toggle) {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: auto;
    flex-shrink: 0;
    border-radius: var(--radius-dialog);
    background: var(--surface-input);
    border: 1px solid var(--border-subtle);
    color: var(--color-text-muted);
    cursor: pointer;
  }
  :global(.icon-button.right-panel-toggle:hover) {
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
  .asset-pagination {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: var(--space-2) 0;
  }
  .pagination-info {
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    min-width: 60px;
    text-align: center;
    font-variant-numeric: tabular-nums;
  }
  .status {
    color: var(--color-text-secondary);
    text-align: center;
  }
  .error {
    color: var(--color-danger);
  }
</style>
