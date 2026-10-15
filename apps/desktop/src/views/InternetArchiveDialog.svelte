<script lang="ts">
  /**
   * Internet Archive import dialog (Phase 4, task 26a): discover an item
   * through the Buscar / Colección / Enlace tabs, list its PDF files with
   * their availability, accumulate a selection across items, pick a
   * destination collection (existing or new), review the preparation
   * (duplicates, sizes, estimated synced volume), run the import with
   * progress and cancel, and show the result.
   *
   * The MVP imports only: no post-processing option and no settings UI, so
   * every search requires `textsOnly` and every limit keeps its default.
   * Remote content is text only — Svelte interpolations escape it, and this
   * file never uses `{@html}`.
   *
   * Skipping duplicates: the prepared job already contains every available
   * file and the backend offers no command to drop a file from a job, so
   * "Omitir" is honoured by preparing again without the omitted files and
   * starting that second job, discarding the abandoned draft with
   * `iaDiscardDraft` once the new prepare succeeds (a failed discard is
   * ignored: a leftover draft is harmless). Closing from the summary phase
   * without starting discards the draft the same way. Skipped files are
   * never marked failed: `iaFailFileImport` is only called by the import
   * orchestrator for files it actually tried to import.
   */
  import { onDestroy, onMount } from 'svelte'
  import { SvelteSet } from 'svelte/reactivity'
  import { Checkbox, ConfirmDialog } from '@entropia/ui'
  import type { Collection } from '@entropia/store'
  import { getStore } from '$lib/db'
  import { locale, t } from '$lib/i18n'
  import { notifyDocumentExplorerCollectionChanged } from '$lib/document-explorer'
  import {
    iaCancelImport,
    iaDiscardDraft,
    iaGetImportStatus,
    iaGetItem,
    iaListCollection,
    iaListJobs,
    iaPrepareImport,
    iaResolveIdentifier,
    iaSearch,
    iaStartImport,
    onIaJobUpdated,
    parseIaError,
    type IaItemDetails,
    type IaJobStatus,
    type IaJobSummary,
    type IaPrepareEntry,
    type IaPrepareResult,
    type IaSearchPage,
    type IaSelection,
  } from '$lib/internet-archive'
  import {
    IaImportError,
    importVerifiedFiles,
    type IaImportOutcome,
  } from '$lib/internet-archive-import'

  let {
    open = $bindable(true),
    initialCollectionId,
    onClose,
    onImported,
  }: {
    open?: boolean
    initialCollectionId?: string
    onClose: () => void
    onImported?: (collectionId: string) => void
  } = $props()

  type Phase = 'discover' | 'item' | 'destination' | 'summary' | 'running' | 'done'
  type DiscoverTab = 'search' | 'collection' | 'link'

  const ROWS = 10
  const NEW_COLLECTION = '__new__'
  /** The synced-volume estimate above which the summary warns (plan §5). */
  const SYNC_WARN_BYTES = 1024 ** 3
  const TERMINAL_STATES = new Set(['completed', 'partial', 'failed', 'cancelled'])
  const CHOOSABLE_STATUSES = new Set(['in_other_collection', 'new_version'])
  const KNOWN_FILE_REASONS = new Set([
    'unavailable',
    'not_found',
    'file_too_large',
    'rate_limited',
    'verify_failed',
    'too_many_pages',
    'network',
    'cancelled',
    'no_space',
    'split_too_large',
  ])
  const KNOWN_AVAILABILITY = new Set([
    'item_restricted',
    'encrypted',
    'private',
    'unsafe_name',
  ])

  const currentLocale = locale
  const NUMBER_LOCALE = { es: 'es-AR', en: 'en-US' } as const
  const numberFormatter = $derived.by(
    () => new Intl.NumberFormat(NUMBER_LOCALE[$currentLocale])
  )

  let phase = $state<Phase>('discover')
  let discoverTab = $state<DiscoverTab>('search')
  let problem = $state<string | null>(null)

  // Discovery: search tab.
  let searchText = $state('')
  let searchTitle = $state('')
  let searchCreator = $state('')
  let searchFrom = $state('')
  let searchTo = $state('')
  let searchPage = $state(1)
  let searchResult = $state<IaSearchPage | null>(null)
  let searching = $state(false)

  // Discovery: collection tab.
  let collectionInput = $state('')
  let collectionPage = $state(1)
  let collectionResult = $state<IaSearchPage | null>(null)
  let listing = $state(false)

  // Discovery: link tab.
  let linkInput = $state('')
  let resolving = $state(false)

  // Item phase.
  let itemIdentifier = $state('')
  let itemDetails = $state<IaItemDetails | null>(null)
  let itemLoading = $state(false)
  /** Guards `openItem` against stale `iaGetItem` responses. */
  let itemRequest = 0

  // Selection accumulated across items.
  let selection = $state<Array<{ identifier: string; fileName: string; title: string }>>([])

  // Destination phase.
  let collections = $state<Collection[]>([])
  let collectionsLoading = $state(true)
  let destinationChoice = $state('')
  let newCollectionName = $state('')
  let preparing = $state(false)

  // Summary phase.
  let prepareResult = $state<IaPrepareResult | null>(null)
  let targetCollectionId = $state('')
  /** Entries the user chose to import as an independent copy. */
  let importChosen = new SvelteSet<string>()
  let starting = $state(false)
  /** Every (identifier, file) pair omitted by choice, across re-preparations. */
  let omittedPairs = $state<Array<{ identifier: string; fileName: string }>>([])

  // Running phase.
  let jobId = $state<string | null>(null)
  let jobStatus = $state<IaJobStatus | null>(null)
  /** Guards `refresh` against stale `iaGetImportStatus` responses. */
  let statusRequest = 0
  let importInFlight = $state(false)
  let importProgress = $state<{ displayFileName: string } | null>(null)
  let cancelRequested = $state(false)
  let cancelling = $state(false)
  let outcome = $state<IaImportOutcome>({
    imported: [],
    confirmedExisting: [],
    failed: [],
    notProcessed: [],
    cancelled: false,
  })

  let stopListening: (() => void) | null = null

  /**
   * Started jobs that survived a closed dialog (26b): only jobs the backend
   * already accepted (`ready`, `running`, `cancelling`) are offered. Drafts
   * are discarded on close, so offering them would resume nothing.
   */
  let resumableJobs = $state<IaJobSummary[]>([])

  async function loadResumableJobs(): Promise<void> {
    try {
      const jobs = await iaListJobs()
      resumableJobs = jobs.filter(
        (job) => job.state === 'ready' || job.state === 'running' || job.state === 'cancelling'
      )
    } catch {
      // Best effort: a failed listing hides the notice, never the dialog.
      resumableJobs = []
    }
  }

  /**
   * Switches the dialog to the running phase for an already-started job.
   * Everything (status, files, target collection) is read back from the
   * backend — there is no prepare result in memory for these jobs — and the
   * shared running-phase logic (`enterRunning`) takes it from there.
   */
  async function resumeJob(id: string): Promise<void> {
    problem = null
    try {
      const status = await iaGetImportStatus(id)
      if (status.targetCollectionId) targetCollectionId = status.targetCollectionId
      // After a restart the job is `ready` (or `running` with no runner):
      // its queued files only download again once the runner restarts.
      if (!status.running && (status.state === 'ready' || status.state === 'running')) {
        await iaStartImport(id)
      }
    } catch (reason) {
      problem = describeError(reason)
      return
    }
    jobId = id
    enterRunning()
  }

  onMount(() => {
    void loadCollections()
    void loadResumableJobs()
    onIaJobUpdated(handleJobEvent)
      .then((stop) => {
        stopListening = stop
      })
      .catch(() => {})
    return () => {
      stopListening?.()
      stopListening = null
    }
  })

  onDestroy(() => {
    // Covers a parent unmount that bypasses close(): the phase guard inside
    // keeps this a no-op everywhere except an abandoned summary.
    discardSummaryDraft()
    stopListening?.()
    stopListening = null
  })

  function selectionKey(identifier: string, fileName: string): string {
    return `${identifier}\n${fileName}`
  }

  function isSelected(identifier: string, fileName: string): boolean {
    return selection.some(
      (entry) => entry.identifier === identifier && entry.fileName === fileName
    )
  }

  function toggleFile(identifier: string, fileName: string, title: string): void {
    if (isSelected(identifier, fileName)) {
      selection = selection.filter(
        (entry) => !(entry.identifier === identifier && entry.fileName === fileName)
      )
      return
    }
    selection = [...selection, { identifier, fileName, title }]
  }

  function removeSelection(identifier: string, fileName: string): void {
    selection = selection.filter(
      (entry) => !(entry.identifier === identifier && entry.fileName === fileName)
    )
  }

  const groupedSelections = $derived.by((): IaSelection[] => {
    const groups: IaSelection[] = []
    for (const entry of selection) {
      const group = groups.find((candidate) => candidate.identifier === entry.identifier)
      if (group) group.fileNames.push(entry.fileName)
      else groups.push({ identifier: entry.identifier, fileNames: [entry.fileName] })
    }
    return groups
  })

  /** Every IaErrorCode the backend can produce has a translated message. */
  function describeError(reason: unknown): string {
    if (reason instanceof IaImportError) {
      return t('internetArchive.error.missing_collection')
    }
    const parsed = parseIaError(reason)
    if (parsed.code === 'unknown') {
      return t('internetArchive.error.unknown')
    }
    if (parsed.code.startsWith('http_')) {
      return t('internetArchive.error.http', { status: parsed.status ?? 0 })
    }
    return t(`internetArchive.error.${parsed.code}`)
  }

  /** A per-file failure reason, translated; never raw text or a path. */
  function fileReasonText(raw: string | null): string {
    if (!raw) return t('internetArchive.done.reason.unknown')
    if (KNOWN_FILE_REASONS.has(raw)) {
      return t(`internetArchive.done.reason.${raw}`)
    }
    const parsed = parseIaError(raw)
    if (parsed.code === 'unknown') return t('internetArchive.done.reason.unknown')
    if (parsed.code.startsWith('http_')) {
      return t('internetArchive.error.http', { status: parsed.status ?? 0 })
    }
    return t(`internetArchive.error.${parsed.code}`)
  }

  function availabilityText(tag: string): string {
    if (KNOWN_AVAILABILITY.has(tag)) {
      return t(`internetArchive.availability.${tag}`)
    }
    return t('internetArchive.availability.item_restricted')
  }

  function rejectedReasonText(reason: string): string {
    if (reason === 'not_found') return t('internetArchive.summary.rejectedNotFound')
    if (reason.startsWith('unavailable:')) {
      return availabilityText(reason.slice('unavailable:'.length))
    }
    return fileReasonText(reason)
  }

  function sourceText(source: string): string {
    if (source === 'original') return t('internetArchive.item.source.original')
    if (source === 'derivative') return t('internetArchive.item.source.derivative')
    if (source === 'unknown') return t('internetArchive.item.source.unknown')
    return source
  }

  function formatBytes(size: number | null): string {
    if (size == null) return t('internetArchive.unknownSize')
    const units = ['B', 'KB', 'MB', 'GB', 'TB']
    let value = size
    let unit = 0
    while (value >= 1024 && unit < units.length - 1) {
      value /= 1024
      unit += 1
    }
    const rounded = Number.isInteger(value)
      ? numberFormatter.format(value)
      : numberFormatter.format(Math.round(value * 10) / 10)
    return `${rounded} ${units[unit]}`
  }

  function formatCount(value: number): string {
    return numberFormatter.format(value)
  }

  function totalPages(numFound: number): number {
    return Math.max(1, Math.ceil(numFound / ROWS))
  }

  function parseYear(raw: string): number | undefined {
    const trimmed = raw.trim()
    if (trimmed === '') return undefined
    if (!/^\d{1,4}$/.test(trimmed)) return undefined
    return Number(trimmed)
  }

  async function loadCollections(): Promise<void> {
    collectionsLoading = true
    try {
      collections = await getStore().collections.findAll()
      if (
        phase === 'destination' &&
        destinationChoice === '' &&
        initialCollectionId &&
        collections.some((candidate) => candidate.id === initialCollectionId)
      ) {
        destinationChoice = initialCollectionId
      }
    } catch {
      // Store errors carry no machine code; the dialog never shows raw text.
      problem = t('internetArchive.destination.loadError')
    } finally {
      collectionsLoading = false
    }
  }

  async function submitSearch(page: number): Promise<void> {
    if (searching) return
    const yearFrom = parseYear(searchFrom)
    const yearTo = parseYear(searchTo)
    if (
      (searchFrom.trim() !== '' && yearFrom === undefined) ||
      (searchTo.trim() !== '' && yearTo === undefined)
    ) {
      problem = t('internetArchive.error.invalid_query')
      return
    }
    searching = true
    problem = null
    try {
      searchPage = page
      searchResult = await iaSearch(
        {
          text: searchText.trim() === '' ? undefined : searchText.trim(),
          title: searchTitle.trim() === '' ? undefined : searchTitle.trim(),
          creator: searchCreator.trim() === '' ? undefined : searchCreator.trim(),
          yearFrom,
          yearTo,
          textsOnly: true,
        },
        page,
        ROWS
      )
    } catch (reason) {
      problem = describeError(reason)
    } finally {
      searching = false
    }
  }

  async function submitCollection(page: number): Promise<void> {
    if (listing) return
    const identifier = collectionInput.trim()
    if (identifier === '') {
      problem = t('internetArchive.error.invalid_query')
      return
    }
    listing = true
    problem = null
    try {
      collectionPage = page
      collectionResult = await iaListCollection(identifier, page, ROWS)
    } catch (reason) {
      problem = describeError(reason)
    } finally {
      listing = false
    }
  }

  async function submitLink(): Promise<void> {
    if (resolving) return
    const input = linkInput.trim()
    if (input === '') {
      problem = t('internetArchive.error.invalid_query')
      return
    }
    resolving = true
    problem = null
    try {
      const identifier = await iaResolveIdentifier(input)
      await openItem(identifier)
    } catch (reason) {
      problem = describeError(reason)
    } finally {
      resolving = false
    }
  }

  async function openItem(identifier: string): Promise<void> {
    const request = ++itemRequest
    phase = 'item'
    problem = null
    itemLoading = true
    itemDetails = null
    itemIdentifier = identifier
    try {
      const details = await iaGetItem(identifier)
      if (request !== itemRequest) return
      itemDetails = details
    } catch (reason) {
      if (request !== itemRequest) return
      problem = describeError(reason)
    } finally {
      if (request === itemRequest) itemLoading = false
    }
  }

  function browseItemAsCollection(identifier: string): void {
    discoverTab = 'collection'
    collectionInput = identifier
    collectionPage = 1
    collectionResult = null
    phase = 'discover'
    problem = null
    void submitCollection(1)
  }

  function goDestination(): void {
    if (selection.length === 0) return
    phase = 'destination'
    problem = null
    if (
      destinationChoice === '' &&
      initialCollectionId &&
      !collectionsLoading &&
      collections.some((candidate) => candidate.id === initialCollectionId)
    ) {
      destinationChoice = initialCollectionId
    }
    if (collections.length === 0 && !collectionsLoading) {
      void loadCollections()
    }
  }

  const isNewDestination = $derived(destinationChoice === NEW_COLLECTION)
  const validDestination = $derived(
    destinationChoice !== '' && (!isNewDestination || newCollectionName.trim() !== '')
  )

  async function prepare(): Promise<void> {
    if (!validDestination || preparing) return
    preparing = true
    problem = null
    let createdId: string | null = null
    try {
      let collectionId = destinationChoice
      if (isNewDestination) {
        const created = await getStore().collections.create({
          name: newCollectionName.trim(),
          description: null,
        })
        createdId = created.id
        collectionId = created.id
      }
      const result = await iaPrepareImport(collectionId, groupedSelections)
      // A collection made only for this import must not outlive its failure.
      createdId = null
      targetCollectionId = collectionId
      applyPrepareResult(result)
      phase = 'summary'
    } catch (reason) {
      if (createdId) {
        await getStore()
          .collections.deleteIfEmpty(createdId)
          .catch(() => false)
      }
      problem = describeError(reason)
    } finally {
      preparing = false
    }
  }

  function entryKey(entry: IaPrepareEntry): string {
    return selectionKey(entry.identifier, entry.fileName)
  }

  function isChoosable(entry: IaPrepareEntry): boolean {
    return (
      entry.availability === 'available' &&
      entry.duplicate != null &&
      CHOOSABLE_STATUSES.has(entry.duplicate.status)
    )
  }

  function willImport(entry: IaPrepareEntry): boolean {
    if (entry.availability !== 'available' || entry.duplicate == null) return false
    if (entry.duplicate.status === 'new') return true
    if (entry.duplicate.status === 'already_in_target') return false
    return importChosen.has(entryKey(entry))
  }

  function applyPrepareResult(result: IaPrepareResult): void {
    prepareResult = result
    for (const entry of result.summary.entries) {
      if (isChoosable(entry) && !importChosen.has(entryKey(entry))) {
        const key = entryKey(entry)
        if (!omittedPairs.some((pair) => selectionKey(pair.identifier, pair.fileName) === key)) {
          omittedPairs = [
            ...omittedPairs,
            { identifier: entry.identifier, fileName: entry.fileName },
          ]
        }
      }
    }
  }

  function setChoice(entry: IaPrepareEntry, importCopy: boolean): void {
    const key = entryKey(entry)
    if (importCopy) importChosen.add(key)
    else importChosen.delete(key)
  }

  const summaryEntries = $derived(prepareResult?.summary.entries ?? [])
  const duplicateEntries = $derived(
    summaryEntries.filter((entry) => entry.duplicate?.status !== 'new')
  )
  const willImportEntries = $derived(summaryEntries.filter(willImport))
  const alreadyEntries = $derived(
    summaryEntries.filter(
      (entry) =>
        entry.availability === 'available' && entry.duplicate?.status === 'already_in_target'
    )
  )
  /** 2x known bytes of the files that will be imported (plan §5). */
  const syncEstimate = $derived.by(() => {
    let total = 0
    for (const entry of willImportEntries) {
      total += entry.size ?? 0
    }
    return total * 2
  })
  const showSyncWarning = $derived(syncEstimate > SYNC_WARN_BYTES)
  const canStart = $derived(willImportEntries.length > 0 || alreadyEntries.length > 0)

  function filteredSelections(): IaSelection[] {
    const omitted = new Set(
      summaryEntries
        .filter((entry) => isChoosable(entry) && !importChosen.has(entryKey(entry)))
        .map(entryKey)
    )
    if (omitted.size === 0) return groupedSelections
    const filtered: IaSelection[] = []
    for (const group of groupedSelections) {
      const fileNames = group.fileNames.filter(
        (fileName) => !omitted.has(selectionKey(group.identifier, fileName))
      )
      if (fileNames.length > 0) filtered.push({ identifier: group.identifier, fileNames })
    }
    return filtered
  }

  async function startImport(): Promise<void> {
    if (!canStart || starting || !prepareResult) return
    starting = true
    problem = null
    try {
      const omitted = summaryEntries.filter(
        (entry) => isChoosable(entry) && !importChosen.has(entryKey(entry))
      )
      if (omitted.length > 0) {
        // The prepared job already contains the omitted files and no command
        // drops them, so prepare again without them and start that job.
        const abandonedId = prepareResult.jobId
        const again = await iaPrepareImport(targetCollectionId, filteredSelections())
        applyPrepareResult(again)
        // Best effort: a leftover draft is harmless, so a failed discard
        // stays silent instead of blocking the import.
        void iaDiscardDraft(abandonedId).catch(() => {})
      }
      jobId = prepareResult.jobId
      await iaStartImport(jobId)
      enterRunning()
    } catch (reason) {
      problem = describeError(reason)
    } finally {
      starting = false
    }
  }

  function enterRunning(): void {
    phase = 'running'
    problem = null
    cancelRequested = false
    cancelling = false
    importInFlight = false
    importProgress = null
    jobStatus = null
    outcome = { imported: [], confirmedExisting: [], failed: [], notProcessed: [], cancelled: false }
    void refresh()
  }

  function handleJobEvent(updatedId: string): void {
    if (!jobId || updatedId !== jobId || phase !== 'running') return
    void refresh()
  }

  async function refresh(): Promise<void> {
    if (!jobId || phase !== 'running') return
    const request = ++statusRequest
    const currentJob = jobId
    let status: IaJobStatus
    try {
      status = await iaGetImportStatus(currentJob)
    } catch (reason) {
      if (request !== statusRequest) return
      problem = describeError(reason)
      return
    }
    if (request !== statusRequest) return
    if (jobId !== currentJob || phase !== 'running') return
    jobStatus = status
    await maybeImport()
    checkDone()
  }

  function processedIndexes(): Set<number> {
    return new Set([
      ...outcome.imported.map((entry) => entry.index),
      ...outcome.confirmedExisting.map((entry) => entry.index),
      ...outcome.failed.map((entry) => entry.index),
    ])
  }

  async function maybeImport(): Promise<void> {
    if (importInFlight || phase !== 'running' || !jobStatus || !jobId) return
    const processed = processedIndexes()
    const pending = jobStatus.files.some(
      (file) => file.state === 'verified' && !processed.has(file.index)
    )
    if (!pending) return
    importInFlight = true
    try {
      const run = await importVerifiedFiles(jobId, {
        baseErrorMessage: t('internetArchive.title'),
        onProgress: (progress) => {
          importProgress = { displayFileName: progress.displayFileName }
        },
        shouldCancel: () => cancelRequested,
      })
      outcome = {
        imported: [...outcome.imported, ...run.imported],
        confirmedExisting: [...outcome.confirmedExisting, ...run.confirmedExisting],
        failed: [...outcome.failed, ...run.failed],
        notProcessed: run.notProcessed,
        cancelled: outcome.cancelled || run.cancelled,
      }
    } catch (reason) {
      problem = describeError(reason)
    } finally {
      importInFlight = false
    }
    checkDone()
  }

  function checkDone(): void {
    if (phase !== 'running' || importInFlight || !jobStatus) return
    if (!TERMINAL_STATES.has(jobStatus.state)) return
    if (outcome.imported.length + outcome.confirmedExisting.length > 0) {
      notifyDocumentExplorerCollectionChanged(targetCollectionId)
    }
    phase = 'done'
  }

  async function cancelImport(): Promise<void> {
    if (phase !== 'running' || !jobId || !jobStatus || cancelling) return
    if (TERMINAL_STATES.has(jobStatus.state)) return
    cancelRequested = true
    cancelling = true
    problem = null
    try {
      await iaCancelImport(jobId)
    } catch (reason) {
      problem = describeError(reason)
    } finally {
      cancelling = false
    }
    await refresh()
  }

  function openCollection(): void {
    if (targetCollectionId && onImported) {
      onImported(targetCollectionId)
    }
    close()
  }

  /** Discard the prepared draft when the summary is abandoned. Never runs
   *  after `iaStartImport` succeeded: starting always leaves this phase. */
  function discardSummaryDraft(): void {
    if (phase !== 'summary' || !prepareResult) return
    void iaDiscardDraft(prepareResult.jobId).catch(() => {})
  }

  function close(): void {
    // While a job is active the dialog owns its progress: closing (or
    // Escape) must not abandon it silently, so the footer action stays
    // disabled and this is inert.
    if (phase === 'running' && jobStatus && !TERMINAL_STATES.has(jobStatus.state)) return
    discardSummaryDraft()
    stopListening?.()
    stopListening = null
    open = false
    onClose()
  }

  function handleDialogCancel(): void {
    close()
  }

  function handleDialogConfirm(): void {
    if (phase === 'discover' || phase === 'item') {
      goDestination()
    } else if (phase === 'destination') {
      void prepare()
    } else if (phase === 'summary') {
      void startImport()
    } else if (phase === 'done') {
      openCollection()
    }
  }

  const confirmLabel = $derived(
    phase === 'running'
      ? undefined
      : phase === 'done'
        ? t('internetArchive.openCollection')
        : phase === 'destination'
          ? t('internetArchive.prepare')
          : phase === 'summary'
            ? t('internetArchive.start')
            : t('internetArchive.continue')
  )

  const confirmDisabled = $derived(
    phase === 'discover' || phase === 'item'
      ? selection.length === 0
      : phase === 'destination'
        ? !validDestination || collectionsLoading
        : phase === 'summary'
          ? !canStart
          : false
  )

  const cancelLabel = $derived(
    phase === 'done' ? t('internetArchive.close') : t('internetArchive.cancel')
  )
  const cancelDisabled = $derived(phase === 'running' || preparing || starting)
  const confirming = $derived(preparing || starting)

  const doneFailed = $derived.by(() => {
    if (phase !== 'done' || !jobStatus) {
      return outcome.failed.map((entry) => ({
        key: `outcome-${entry.index}`,
        name: String(entry.index),
        reason: fileReasonText(entry.code),
      }))
    }
    const seen = new Set<number>()
    const rows: Array<{ key: string; name: string; reason: string }> = []
    for (const file of jobStatus.files) {
      if (file.state !== 'failed' && file.state !== 'unavailable' && file.state !== 'cancelled')
        continue
      seen.add(file.index)
      rows.push({
        key: `file-${file.index}`,
        name: file.displayFileName,
        reason:
          file.state === 'unavailable' && !file.error
            ? fileReasonText('unavailable')
            : fileReasonText(file.error ?? (file.state === 'cancelled' ? 'cancelled' : null)),
      })
    }
    for (const entry of outcome.failed) {
      if (seen.has(entry.index)) continue
      rows.push({
        key: `outcome-${entry.index}`,
        name: String(entry.index),
        reason: fileReasonText(entry.code),
      })
    }
    return rows
  })

  const doneCancelled = $derived(
    phase === 'done' &&
      (outcome.cancelled || jobStatus?.state === 'cancelled' || cancelRequested)
  )

  const runningProgress = $derived.by(() => {
    if (!jobStatus) return null
    const total = jobStatus.files.length
    const done = jobStatus.files.filter((file) =>
      ['imported', 'skipped_duplicate', 'failed', 'unavailable', 'cancelled'].includes(file.state)
    ).length
    return { done, total }
  })
</script>

{#if open}
  <ConfirmDialog
    title={t('internetArchive.title')}
    titleId="internet-archive-dialog-title"
    cancelLabel={$currentLocale && cancelLabel}
    cancelDisabled={cancelDisabled}
    dismissOnOverlay={false}
    confirmLabel={confirmLabel !== undefined ? $currentLocale && confirmLabel : undefined}
    confirmDisabled={confirmDisabled}
    confirming={confirming}
    error={problem}
    oncancel={handleDialogCancel}
    onconfirm={handleDialogConfirm}
  >
    {#if phase === 'discover'}
      {#if resumableJobs.length > 0}
        <div class="ia-resume" role="status">
          <p class="ia-resume__title">{t('internetArchive.resume.title')}</p>
          <ul class="ia-resume__list">
            {#each resumableJobs as job (job.id)}
              <li class="ia-resume__item">
                <span class="ia-resume__label">
                  {t('internetArchive.resume.job', {
                    state: t(`internetArchive.jobState.${job.state}`),
                    done: formatCount(job.verifiedFiles),
                    total: formatCount(job.totalFiles),
                  })}
                </span>
                <button
                  type="button"
                  class="ia-results__action"
                  onclick={() => void resumeJob(job.id)}
                >
                  {t('internetArchive.resume.action')}
                </button>
              </li>
            {/each}
          </ul>
        </div>
      {/if}
      <div class="ia-tabs" role="group" aria-label={t('internetArchive.discover.group')}>
        <button
          type="button"
          class="ia-tabs__tab"
          class:ia-tabs__tab--active={discoverTab === 'search'}
          aria-pressed={discoverTab === 'search'}
          onclick={() => (discoverTab = 'search')}
        >
          {t('internetArchive.discover.tab.search')}
        </button>
        <button
          type="button"
          class="ia-tabs__tab"
          class:ia-tabs__tab--active={discoverTab === 'collection'}
          aria-pressed={discoverTab === 'collection'}
          onclick={() => (discoverTab = 'collection')}
        >
          {t('internetArchive.discover.tab.collection')}
        </button>
        <button
          type="button"
          class="ia-tabs__tab"
          class:ia-tabs__tab--active={discoverTab === 'link'}
          aria-pressed={discoverTab === 'link'}
          onclick={() => (discoverTab = 'link')}
        >
          {t('internetArchive.discover.tab.link')}
        </button>
      </div>

      {#if discoverTab === 'search'}
        <form
          class="ia-form"
          onsubmit={(event) => {
            event.preventDefault()
            void submitSearch(1)
          }}
        >
          <label class="ia-field">
            <span class="ia-field__label">{t('internetArchive.search.text')}</span>
            <input type="text" class="ia-field__input" bind:value={searchText} />
          </label>
          <label class="ia-field">
            <span class="ia-field__label">{t('internetArchive.search.title')}</span>
            <input type="text" class="ia-field__input" bind:value={searchTitle} />
          </label>
          <label class="ia-field">
            <span class="ia-field__label">{t('internetArchive.search.creator')}</span>
            <input type="text" class="ia-field__input" bind:value={searchCreator} />
          </label>
          <div class="ia-form__row">
            <label class="ia-field">
              <span class="ia-field__label">{t('internetArchive.search.yearFrom')}</span>
              <input
                type="number"
                class="ia-field__input"
                min="0"
                max="9999"
                bind:value={searchFrom}
              />
            </label>
            <label class="ia-field">
              <span class="ia-field__label">{t('internetArchive.search.yearTo')}</span>
              <input type="number" class="ia-field__input" min="0" max="9999" bind:value={searchTo} />
            </label>
          </div>
          <button type="submit" class="ia-submit" disabled={searching}>
            {searching
              ? t('internetArchive.search.searching')
              : t('internetArchive.search.submit')}
          </button>
        </form>
        {#if searching && !searchResult}
          <p class="ia-status" role="status" aria-live="polite">
            {t('internetArchive.search.searching')}
          </p>
        {:else if searchResult}
          {#if searchResult.numFound === 0}
            <p class="ia-status" role="status">{t('internetArchive.search.empty')}</p>
          {:else}
            <p class="ia-status" role="status">
              {searchResult.numFound === 1
                ? t('internetArchive.search.results.one')
                : t('internetArchive.search.results.other', {
                    count: formatCount(searchResult.numFound),
                  })}
            </p>
            <ul class="ia-results">
              {#each searchResult.items as hit (hit.identifier)}
                <li class="ia-results__item">
                  <span class="ia-results__title">{hit.title ?? hit.identifier}</span>
                  {#if hit.creators.length > 0}
                    <span class="ia-results__meta">{hit.creators.join(', ')}</span>
                  {/if}
                  {#if hit.date}
                    <span class="ia-results__meta">{hit.date}</span>
                  {/if}
                  <span class="ia-results__meta ia-results__identifier">{hit.identifier}</span>
                  {#if hit.restricted}
                    <span class="ia-badge">{t('internetArchive.restrictedBadge')}</span>
                  {/if}
                  <button
                    type="button"
                    class="ia-results__action"
                    onclick={() => void openItem(hit.identifier)}
                  >
                    {t('internetArchive.search.viewFiles')}
                  </button>
                </li>
              {/each}
            </ul>
            <div class="ia-paging">
              <button
                type="button"
                disabled={searchPage <= 1 || searching}
                onclick={() => void submitSearch(searchPage - 1)}
              >
                {t('internetArchive.search.prev')}
              </button>
              <span class="ia-paging__label" role="status">
                {t('internetArchive.search.page', {
                  page: formatCount(searchPage),
                  pages: formatCount(totalPages(searchResult.numFound)),
                })}
              </span>
              <button
                type="button"
                disabled={searchPage >= totalPages(searchResult.numFound) || searching}
                onclick={() => void submitSearch(searchPage + 1)}
              >
                {t('internetArchive.search.next')}
              </button>
            </div>
          {/if}
        {/if}
      {:else if discoverTab === 'collection'}
        <form
          class="ia-form"
          onsubmit={(event) => {
            event.preventDefault()
            void submitCollection(1)
          }}
        >
          <label class="ia-field">
            <span class="ia-field__label">{t('internetArchive.collection.idLabel')}</span>
            <input
              type="text"
              class="ia-field__input"
              placeholder={t('internetArchive.collection.idPlaceholder')}
              bind:value={collectionInput}
            />
          </label>
          <button type="submit" class="ia-submit" disabled={listing}>
            {listing
              ? t('internetArchive.collection.loading')
              : t('internetArchive.collection.submit')}
          </button>
        </form>
        {#if listing && !collectionResult}
          <p class="ia-status" role="status" aria-live="polite">
            {t('internetArchive.collection.loading')}
          </p>
        {:else if collectionResult}
          {#if collectionResult.numFound === 0}
            <p class="ia-status" role="status">{t('internetArchive.collection.empty')}</p>
          {:else}
            <ul class="ia-results">
              {#each collectionResult.items as hit (hit.identifier)}
                <li class="ia-results__item">
                  <span class="ia-results__title">{hit.title ?? hit.identifier}</span>
                  {#if hit.creators.length > 0}
                    <span class="ia-results__meta">{hit.creators.join(', ')}</span>
                  {/if}
                  {#if hit.date}
                    <span class="ia-results__meta">{hit.date}</span>
                  {/if}
                  <span class="ia-results__meta ia-results__identifier">{hit.identifier}</span>
                  {#if hit.restricted}
                    <span class="ia-badge">{t('internetArchive.restrictedBadge')}</span>
                  {/if}
                  <button
                    type="button"
                    class="ia-results__action"
                    onclick={() => void openItem(hit.identifier)}
                  >
                    {t('internetArchive.search.viewFiles')}
                  </button>
                </li>
              {/each}
            </ul>
            <div class="ia-paging">
              <button
                type="button"
                disabled={collectionPage <= 1 || listing}
                onclick={() => void submitCollection(collectionPage - 1)}
              >
                {t('internetArchive.search.prev')}
              </button>
              <span class="ia-paging__label" role="status">
                {t('internetArchive.search.page', {
                  page: formatCount(collectionPage),
                  pages: formatCount(totalPages(collectionResult.numFound)),
                })}
              </span>
              <button
                type="button"
                disabled={collectionPage >= totalPages(collectionResult.numFound) || listing}
                onclick={() => void submitCollection(collectionPage + 1)}
              >
                {t('internetArchive.search.next')}
              </button>
            </div>
          {/if}
        {/if}
      {:else}
        <form
          class="ia-form"
          onsubmit={(event) => {
            event.preventDefault()
            void submitLink()
          }}
        >
          <label class="ia-field">
            <span class="ia-field__label">{t('internetArchive.link.label')}</span>
            <input
              type="text"
              class="ia-field__input"
              placeholder={t('internetArchive.link.placeholder')}
              bind:value={linkInput}
            />
          </label>
          <p class="ia-hint">{t('internetArchive.link.hint')}</p>
          <button type="submit" class="ia-submit" disabled={resolving}>
            {resolving ? t('internetArchive.link.resolving') : t('internetArchive.link.submit')}
          </button>
        </form>
      {/if}

      {@render selectionPanel()}
    {:else if phase === 'item'}
      <button type="button" class="ia-back" onclick={() => (phase = 'discover')}>
        {t('internetArchive.back')}
      </button>
      {#if itemLoading}
        <p class="ia-status" role="status" aria-live="polite">
          {t('internetArchive.item.loading')}
        </p>
      {:else if itemDetails}
        <div class="ia-item">
          <p class="ia-item__title">{itemDetails.title ?? itemDetails.identifier}</p>
          {#if itemDetails.creators.length > 0}
            <p class="ia-item__meta">{itemDetails.creators.join(', ')}</p>
          {/if}
          {#if itemDetails.date}
            <p class="ia-item__meta">{itemDetails.date}</p>
          {/if}
          <p class="ia-item__meta ia-results__identifier">{itemDetails.identifier}</p>
          {#if itemDetails.restricted}
            <p><span class="ia-badge">{t('internetArchive.restrictedBadge')}</span></p>
          {/if}
        </div>
        {#if itemDetails.mediatype === 'collection'}
          <p class="ia-status" role="status">{t('internetArchive.item.isCollection')}</p>
          <button
            type="button"
            class="ia-submit"
            onclick={() => browseItemAsCollection(itemDetails?.identifier ?? itemIdentifier)}
          >
            {t('internetArchive.item.browseCollection')}
          </button>
        {:else}
          <h4 class="ia-heading">{t('internetArchive.item.files')}</h4>
          {#if itemDetails.pdfs.length === 0}
            <p class="ia-status" role="status">{t('internetArchive.item.noPdfs')}</p>
          {:else}
            <ul class="ia-files">
              {#each itemDetails.pdfs as file (file.name)}
                {@const selected = isSelected(itemDetails.identifier, file.name)}
                {@const available = file.availability === 'available'}
                <li class="ia-files__item">
                  <Checkbox
                    checked={selected}
                    disabled={!available}
                    label={t('internetArchive.item.select', { name: file.name })}
                    onchange={() =>
                      toggleFile(
                        itemDetails?.identifier ?? itemIdentifier,
                        file.name,
                        itemDetails?.title ?? itemDetails?.identifier ?? itemIdentifier
                      )}
                  >
                    <span class="ia-files__name">{file.name}</span>
                  </Checkbox>
                  <span class="ia-files__meta">
                    {t('internetArchive.item.size', { size: formatBytes(file.size) })}
                  </span>
                  <span class="ia-files__meta">
                    {t('internetArchive.item.source', { source: sourceText(file.source) })}
                  </span>
                  {#if !available}
                    <span class="ia-files__reason">{availabilityText(file.availability)}</span>
                  {/if}
                </li>
              {/each}
            </ul>
            {#if itemDetails.truncated}
              <p class="ia-status" role="status">{t('internetArchive.item.truncated')}</p>
            {/if}
          {/if}
        {/if}
      {/if}

      {@render selectionPanel()}
    {:else if phase === 'destination'}
      <button type="button" class="ia-back" onclick={() => (phase = 'discover')}>
        {t('internetArchive.back')}
      </button>
      <fieldset class="ia-destination">
        <legend class="ia-legend">{t('internetArchive.destination.title')}</legend>
        {#if collectionsLoading}
          <p class="ia-status" role="status" aria-live="polite">
            {t('internetArchive.destination.loading')}
          </p>
        {:else}
          <div class="ia-destination__options">
            {#each collections as option (option.id)}
              <label class="ia-destination__option">
                <input
                  type="radio"
                  name="ia-destination"
                  value={option.id}
                  bind:group={destinationChoice}
                />
                <span class="ia-destination__option-name">{option.name}</span>
              </label>
            {/each}
            <label class="ia-destination__option">
              <input
                type="radio"
                name="ia-destination"
                value={NEW_COLLECTION}
                bind:group={destinationChoice}
              />
              <span class="ia-destination__option-name ia-destination__option-name--fixed">
                {t('internetArchive.destination.newCollection')}
              </span>
              <input
                type="text"
                class="ia-destination__new-name"
                placeholder={t('internetArchive.destination.newCollectionPlaceholder')}
                aria-label={t('internetArchive.destination.newCollectionAriaLabel')}
                bind:value={newCollectionName}
                disabled={!isNewDestination}
                onfocus={() => (destinationChoice = NEW_COLLECTION)}
              />
            </label>
          </div>
        {/if}
      </fieldset>
    {:else if phase === 'summary' && prepareResult}
      <h4 class="ia-heading">{t('internetArchive.summary.heading')}</h4>
      <p class="ia-status" role="status">
        {t('internetArchive.summary.toImport', { count: formatCount(willImportEntries.length) })}
      </p>
      {#if willImportEntries.length > 0}
        <ul class="ia-results">
          {#each willImportEntries as entry (selectionKey(entry.identifier, entry.fileName))}
            <li class="ia-results__item">
              <span class="ia-results__title">{entry.fileName}</span>
              <span class="ia-results__meta ia-results__identifier">{entry.identifier}</span>
            </li>
          {/each}
        </ul>
      {/if}
      <p class="ia-status">
        {t('internetArchive.summary.knownSize', {
          size: formatBytes(prepareResult.summary.knownBytes),
        })}
      </p>
      {#if prepareResult.summary.unknownSizeFiles > 0}
        <p class="ia-status">
          {prepareResult.summary.unknownSizeFiles === 1
            ? t('internetArchive.summary.unknownSizes.one')
            : t('internetArchive.summary.unknownSizes.other', {
                count: formatCount(prepareResult.summary.unknownSizeFiles),
              })}
        </p>
      {/if}

      <h4 class="ia-heading">{t('internetArchive.summary.duplicates')}</h4>
      <ul class="ia-results">
        {#each duplicateEntries as entry, index (selectionKey(entry.identifier, entry.fileName))}
          <li class="ia-results__item">
            <span class="ia-results__title">{entry.fileName}</span>
            <span class="ia-results__meta ia-results__identifier">{entry.identifier}</span>
            {#if entry.availability !== 'available'}
              <span class="ia-files__reason">{availabilityText(entry.availability)}</span>
            {:else if entry.duplicate?.status === 'already_in_target'}
              <span class="ia-files__reason">
                {t('internetArchive.summary.alreadyInTarget')}
              </span>
            {:else if isChoosable(entry)}
              <span class="ia-files__reason">
                {entry.duplicate?.status === 'in_other_collection'
                  ? t('internetArchive.summary.inOther')
                  : t('internetArchive.summary.newVersion')}
              </span>
              <div
                class="ia-choice"
                role="group"
                aria-label={t('internetArchive.summary.choice')}
              >
                <label class="ia-choice__option">
                  <input
                    type="radio"
                    name={`ia-choice-${index}`}
                    checked={importChosen.has(entryKey(entry))}
                    onchange={() => setChoice(entry, true)}
                  />
                  {t('internetArchive.summary.importCopy')}
                </label>
                <label class="ia-choice__option">
                  <input
                    type="radio"
                    name={`ia-choice-${index}`}
                    checked={!importChosen.has(entryKey(entry))}
                    onchange={() => setChoice(entry, false)}
                  />
                  {t('internetArchive.summary.omit')}
                </label>
              </div>
            {/if}
          </li>
        {/each}
      </ul>

      {#if prepareResult.rejected.length > 0}
        <h4 class="ia-heading">{t('internetArchive.summary.rejected')}</h4>
        <ul class="ia-results">
          {#each prepareResult.rejected as rejected (selectionKey(rejected.identifier, rejected.fileName))}
            <li class="ia-results__item">
              <span class="ia-results__title">{rejected.fileName}</span>
              <span class="ia-results__meta ia-results__identifier">{rejected.identifier}</span>
              <span class="ia-files__reason">{rejectedReasonText(rejected.reason)}</span>
            </li>
          {/each}
        </ul>
      {/if}

      <p class="ia-status">
        {t('internetArchive.summary.syncVolume', { size: formatBytes(syncEstimate) })}
      </p>
      {#if showSyncWarning}
        <p class="ia-warning" role="alert">{t('internetArchive.summary.syncWarning')}</p>
      {/if}
      {#if !canStart}
        <p class="ia-status" role="alert">{t('internetArchive.summary.nothingToImport')}</p>
      {/if}
    {:else if phase === 'running'}
      <h4 class="ia-heading">{t('internetArchive.running.heading')}</h4>
      {#if runningProgress}
        <div class="ia-progress" role="status" aria-live="polite">
          <progress
            value={runningProgress.done}
            max={Math.max(1, runningProgress.total)}
            aria-label={t('internetArchive.running.progressLabel')}
          ></progress>
          <p class="ia-status">
            {t('internetArchive.running.progress', {
              done: formatCount(runningProgress.done),
              total: formatCount(runningProgress.total),
            })}
          </p>
        </div>
      {/if}
      {#if importProgress}
        <p class="ia-status">
          {t('internetArchive.running.current', { name: importProgress.displayFileName })}
        </p>
      {/if}
      {#if jobStatus}
        <ul class="ia-results">
          {#each jobStatus.files as file (file.index)}
            <li class="ia-results__item">
              <span class="ia-results__title">{file.displayFileName}</span>
              <span class="ia-results__meta">
                {t(`internetArchive.fileState.${file.state}`)}
              </span>
              {#if (file.state === 'failed' || file.state === 'unavailable' || file.state === 'cancelled') && file.error}
                <span class="ia-files__reason">{fileReasonText(file.error)}</span>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
      <button
        type="button"
        class="ia-submit ia-submit--danger"
        disabled={cancelling || !jobStatus || TERMINAL_STATES.has(jobStatus.state)}
        onclick={() => void cancelImport()}
      >
        {cancelling ? t('internetArchive.running.cancelling') : t('internetArchive.cancel')}
      </button>
    {:else if phase === 'done'}
      <h4 class="ia-heading">{t('internetArchive.done.heading')}</h4>
      {#if doneCancelled}
        <p class="ia-status" role="status">{t('internetArchive.done.cancelledNote')}</p>
      {/if}
      <div class="ia-counts" role="status">
        <p class="ia-status">
          {t('internetArchive.done.imported', { count: formatCount(outcome.imported.length) })}
        </p>
        <p class="ia-status">
          {t('internetArchive.done.present', {
            count: formatCount(outcome.confirmedExisting.length),
          })}
        </p>
        <p class="ia-status">
          {t('internetArchive.done.failed', { count: formatCount(doneFailed.length) })}
        </p>
        {#if omittedPairs.length > 0}
          <p class="ia-status">
            {t('internetArchive.done.omitted', { count: formatCount(omittedPairs.length) })}
          </p>
        {/if}
      </div>
      {#if doneFailed.length > 0}
        <ul class="ia-results">
          {#each doneFailed as failure (failure.key)}
            <li class="ia-results__item">
              <span class="ia-results__title">{failure.name}</span>
              <span class="ia-files__reason">{failure.reason}</span>
            </li>
          {/each}
        </ul>
      {/if}
      {#if omittedPairs.length > 0}
        <ul class="ia-results">
          {#each omittedPairs as omitted (selectionKey(omitted.identifier, omitted.fileName))}
            <li class="ia-results__item">
              <span class="ia-results__title">{omitted.fileName}</span>
              <span class="ia-results__meta ia-results__identifier">{omitted.identifier}</span>
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  </ConfirmDialog>
{/if}

{#snippet selectionPanel()}
  <div class="ia-selection">
    <h4 class="ia-heading">{t('internetArchive.selection.title')}</h4>
    <p class="ia-status" role="status">
      {t('internetArchive.selection.count', { count: formatCount(selection.length) })}
    </p>
    {#if selection.length === 0}
      <p class="ia-status">{t('internetArchive.selection.empty')}</p>
    {:else}
      <ul class="ia-results">
        {#each selection as entry (selectionKey(entry.identifier, entry.fileName))}
          <li class="ia-results__item">
            <span class="ia-results__title">{entry.fileName}</span>
            <span class="ia-results__meta ia-results__identifier">{entry.identifier}</span>
            <button
              type="button"
              class="ia-results__action"
              aria-label={t('internetArchive.selection.remove', {
                name: entry.fileName,
              })}
              onclick={() => removeSelection(entry.identifier, entry.fileName)}
            >
              {t('internetArchive.selection.remove', { name: entry.fileName })}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
{/snippet}

<style>
  .ia-tabs {
    display: flex;
    gap: var(--space-1);
    margin-bottom: var(--space-3);
    padding: var(--space-1);
    border-radius: var(--radius-control);
    background-color: color-mix(in srgb, var(--color-surface-sunken) 88%, transparent);
  }

  .ia-resume {
    margin-bottom: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-control);
    background-color: color-mix(in srgb, var(--color-surface-sunken) 88%, transparent);
  }

  .ia-resume__title {
    margin: 0 0 var(--space-1);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-primary);
  }

  .ia-resume__list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .ia-resume__item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .ia-resume__label {
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  .ia-tabs__tab {
    flex: 1;
    padding: var(--space-2);
    border: none;
    border-radius: var(--radius-control);
    background: transparent;
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    cursor: pointer;
  }

  .ia-tabs__tab--active {
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
    font-weight: var(--font-weight-medium);
  }

  .ia-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-bottom: var(--space-3);
  }

  .ia-form__row {
    display: flex;
    gap: var(--space-2);
  }

  .ia-form__row .ia-field {
    flex: 1;
    min-width: 0;
  }

  .ia-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .ia-field__label {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.075em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }

  .ia-field__input {
    height: var(--control-height-sm);
    padding: 0 var(--space-2);
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
    background-color: color-mix(in srgb, var(--color-surface-sunken) 88%, transparent);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
    outline: none;
  }

  .ia-field__input:focus,
  .ia-field__input:focus-visible {
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
  }

  .ia-submit {
    align-self: flex-start;
    min-height: var(--control-height-sm);
    padding: 0 var(--space-4);
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-on-accent, var(--color-surface-raised));
    background-color: var(--color-accent);
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    cursor: pointer;
  }

  .ia-submit:disabled {
    cursor: not-allowed;
    opacity: 0.56;
  }

  .ia-submit--danger {
    background-color: var(--color-danger, var(--color-accent));
  }

  .ia-back {
    align-self: flex-start;
    margin-bottom: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border: none;
    border-radius: var(--radius-control);
    background: transparent;
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    cursor: pointer;
  }

  .ia-back:hover {
    background: var(--color-accent-faint);
  }

  .ia-hint,
  .ia-status {
    margin: 0 0 var(--space-2);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    overflow-wrap: anywhere;
  }

  .ia-heading {
    margin: var(--space-3) 0 var(--space-1);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.075em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }

  .ia-results {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-height: 240px;
    margin: 0 0 var(--space-2);
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }

  .ia-results__item {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-1) var(--space-2);
    padding: var(--space-2);
    border-radius: var(--radius-control);
  }

  .ia-results__title {
    flex: 1 1 100%;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
  }

  .ia-results__meta {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .ia-results__identifier {
    font-family: var(--font-mono);
    overflow-wrap: anywhere;
  }

  .ia-results__action {
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-control);
    background: transparent;
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    cursor: pointer;
  }

  .ia-results__action:hover {
    background: var(--color-accent-faint);
  }

  .ia-badge {
    padding: 0 var(--space-2);
    border-radius: var(--radius-control);
    background-color: var(--color-warning-soft, var(--color-accent-faint));
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.075em;
    text-transform: uppercase;
    color: var(--color-warning, var(--color-text-secondary));
  }

  .ia-paging {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }

  .ia-paging button {
    padding: var(--space-1) var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-control);
    background: transparent;
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    cursor: pointer;
  }

  .ia-paging button:disabled {
    cursor: not-allowed;
    opacity: 0.48;
  }

  .ia-paging__label {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .ia-files {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-height: 240px;
    margin: 0 0 var(--space-2);
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }

  .ia-files__item {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2);
    border-radius: var(--radius-control);
  }

  .ia-files__name {
    display: block;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
  }

  .ia-files__meta {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .ia-files__reason {
    font-size: var(--font-size-xs);
    color: var(--color-warning, var(--color-text-secondary));
  }

  .ia-item__title {
    margin: 0;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-primary);
    overflow-wrap: anywhere;
  }

  .ia-item__meta {
    margin: var(--space-1) 0 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    overflow-wrap: anywhere;
  }

  .ia-selection {
    margin-top: var(--space-3);
  }

  .ia-destination {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    border: none;
  }

  .ia-legend {
    padding: 0;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.075em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }

  .ia-destination__options {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-height: 240px;
    overflow-y: auto;
  }

  .ia-destination__option {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2);
    border-radius: var(--radius-control);
    cursor: pointer;
  }

  .ia-destination__option:hover {
    background: var(--color-accent-faint);
  }

  .ia-destination__option-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
  }

  .ia-destination__option-name--fixed {
    flex: 0 0 auto;
  }

  .ia-destination__new-name {
    flex: 1;
    min-width: 0;
    height: var(--control-height-sm);
    padding: 0 var(--space-2);
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
    background-color: color-mix(in srgb, var(--color-surface-sunken) 88%, transparent);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
    outline: none;
  }

  .ia-destination__new-name:focus,
  .ia-destination__new-name:focus-visible {
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
  }

  .ia-destination__new-name:disabled {
    cursor: not-allowed;
    opacity: 0.56;
  }

  .ia-choice {
    display: flex;
    flex: 1 1 100%;
    gap: var(--space-3);
  }

  .ia-choice__option {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    cursor: pointer;
  }

  .ia-warning {
    margin: 0 0 var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background-color: var(--color-warning-soft, var(--color-accent-faint));
    font-size: var(--font-size-sm);
    color: var(--color-warning, var(--color-text-primary));
    overflow-wrap: anywhere;
  }

  .ia-progress {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }

  .ia-counts {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0 0 var(--space-2);
  }

  .ia-counts .ia-status {
    margin: 0;
  }
</style>
