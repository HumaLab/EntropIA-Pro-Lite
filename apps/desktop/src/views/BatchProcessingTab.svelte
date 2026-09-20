<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import { locale, t } from '$lib/i18n'
  import { getStore } from '$lib/db'
  import {
    batchProgress,
    batchStore,
    isTerminalBatchState,
    newBatchRequestId,
    processingControl,
    processingGetBatch,
    processingGetTask,
    processingListBatches,
    processingListActiveBatches,
    processingListTasks,
    processingPrepare,
    processingRetry,
    processingStart,
    taskHasDetail,
    type BatchSnapshot,
    type BatchSummary,
    type BatchTaskDetail,
    type BatchTaskSummary,
  } from '$lib/batch-processing'
  import {
    countForFilter,
    pageCountFor,
    pageWindow,
    splitIntoTables,
    tablesForWidth,
    TASKS_PER_PAGE,
  } from '$lib/batch-pagination'
  import {
    tooltip,
    ActionIcon,
    Button,
    Card,
    Checkbox,
    ConfirmDialog,
    ToolbarMenu,
    type ToolbarMenuItem,
  } from '@entropia/ui'

  interface CollectionOption {
    id: string
    name: string
    items: number
  }

  const TASK_STATE_FILTERS = [
    'pending',
    'running',
    'interrupted',
    'blocked',
    'failed',
    'succeeded',
    'cancelled',
    'skipped',
  ] as const

  // Collections
  let collections = $state<CollectionOption[]>([])
  let collectionsLoading = $state(true)
  let selected = $state<Record<string, true>>({})
  let runOcr = $state(true)
  let runEmbeddings = $state(true)

  // Draft
  let draftId = $state<string | null>(null)
  let draft = $state<BatchSnapshot | null>(null)
  let analyzing = $state(false)
  let starting = $state(false)
  let prepareRequest: { key: string; id: string } | null = null

  // Lists
  let activeBatches = $state<BatchSummary[]>([])
  let historyBatches = $state<BatchSummary[]>([])
  let historyCursor = $state<{ createdAt: number; id: string } | null>(null)
  let historyLoading = $state(false)

  // Detail
  let detailId = $state<string | null>(null)
  let detail = $state<BatchSnapshot | null>(null)
  let tasks = $state<BatchTaskSummary[]>([])
  let tasksPage = $state(1)
  let tasksLoading = $state(false)
  /** The panel's own width, which is what decides how many tables fit. */
  let listWidth = $state(0)
  let taskStateFilter = $state<string>('')
  let expandedTaskId = $state<string | null>(null)
  let expandedTask = $state<BatchTaskDetail | null>(null)

  // Actions
  let busyBatchId = $state<string | null>(null)
  let retryingTaskId = $state<string | null>(null)
  let retryingAll = $state(false)
  let cancelTarget = $state<BatchSummary | BatchSnapshot | null>(null)
  let feedback = $state<{ tone: 'success' | 'error'; text: string } | null>(null)
  let loadError = $state<string | null>(null)

  // Recovery banner
  let recovered = $state<{
    batches: number
    done: number
    todo: number
    stuck: number
  } | null>(null)
  let recoveryDismissed = $state(false)

  let selectedCount = $derived(Object.keys(selected).length)
  let allSelected = $derived(collections.length > 0 && selectedCount === collections.length)
  const currentLocale = locale

  /* The state filter used to be a native <select>. Its popup is drawn by the
     operating system — white surface, foreign type, a blue focus ring — and no
     CSS in this file reaches it. ToolbarMenu is the same choice painted with
     the app's own tokens, and `radio` is exactly what one-of-many means. */
  let taskFilterItems = $derived<ToolbarMenuItem[]>([
    {
      kind: 'radio',
      id: 'all',
      label: t('batch.filterAll'),
      checked: taskStateFilter === '',
      onselect: () => selectTaskFilter(''),
    },
    ...TASK_STATE_FILTERS.map((state) => ({
      kind: 'radio' as const,
      id: state,
      label: stateLabel(state),
      checked: taskStateFilter === state,
      onselect: () => selectTaskFilter(state),
    })),
  ])

  let taskFilterLabel = $derived(
    taskStateFilter ? stateLabel(taskStateFilter) : t('batch.filterAll')
  )

  let taskTotal = $derived(detail ? countForFilter(detail.tasksByState, taskStateFilter) : 0)

  let pageCount = $derived(pageCountFor(taskTotal))

  let tableCount = $derived(tablesForWidth(listWidth))

  /* Both the chunking and the grid read the same number. Letting a container
     query pick the column count independently would be the one way these two
     could disagree — three tracks drawn for two chunks of rows. */
  let taskTables = $derived(splitIntoTables(tasks, tableCount))

  let pageItems = $derived(pageWindow(tasksPage, pageCount))

  function selectTaskFilter(state: string): void {
    if (state === taskStateFilter) return
    taskStateFilter = state
    // A filter that matches fewer units would otherwise strand the panel on a
    // page that no longer exists.
    tasksPage = 1
    collapseTask()
    void loadTasks()
  }

  function goToPage(page: number): void {
    const next = Math.min(Math.max(1, page), pageCount)
    if (next === tasksPage) return
    tasksPage = next
    collapseTask()
    void loadTasks()
  }

  function collapseTask(): void {
    expandedTaskId = null
    expandedTask = null
  }

  function fail(error: unknown, fallback: string): void {
    const message = error instanceof Error ? error.message : String(error)
    feedback = {
      tone: 'error',
      text: message.includes('schema_not_ready')
        ? t('batch.schemaNotReady')
        : t('batch.error', { error: message || fallback }),
    }
  }

  async function loadCollections(): Promise<void> {
    collectionsLoading = true
    try {
      const store = getStore()
      const rows = await store.collections.findAll()
      const withCounts = await Promise.all(
        rows.map(async (collection) => ({
          id: collection.id,
          name: collection.name,
          items: await store.collections.countItems(collection.id),
        }))
      )
      collections = withCounts
    } catch (error) {
      loadError = error instanceof Error ? error.message : String(error)
    } finally {
      collectionsLoading = false
    }
  }

  async function loadLists(): Promise<void> {
    try {
      const [active, history] = await Promise.all([
        processingListActiveBatches(),
        processingListBatches({
          states: ['completed', 'completed_with_errors', 'cancelled'],
          limit: 20,
        }),
      ])
      activeBatches = active
      historyBatches = history.batches
      historyCursor = history.nextCursor
    } catch (error) {
      fail(error, 'lists')
    }
  }

  async function loadMoreHistory(): Promise<void> {
    if (!historyCursor || historyLoading) return
    historyLoading = true
    try {
      const page = await processingListBatches({
        states: ['completed', 'completed_with_errors', 'cancelled'],
        cursorCreatedAt: historyCursor.createdAt,
        cursorId: historyCursor.id,
        limit: 20,
      })
      historyBatches = [...historyBatches, ...page.batches]
      historyCursor = page.nextCursor
    } catch (error) {
      fail(error, 'history')
    } finally {
      historyLoading = false
    }
  }

  async function openDetail(batchId: string): Promise<void> {
    detailId = batchId
    collapseTask()
    taskStateFilter = ''
    tasksPage = 1
    await refreshDetail()
  }

  async function refreshDetail(): Promise<void> {
    if (!detailId) return
    try {
      const snapshot = await processingGetBatch(detailId)
      detail = snapshot
      // The panel stays open while the batch runs, so the unit counts move
      // under it: a page that was the last one can stop existing.
      //
      // Counted off the snapshot in hand rather than read off `pageCount`:
      // that derived belongs to the component's effect, and by the time this
      // await resolves the panel may already be gone, which is exactly when a
      // derived hands back a stale value.
      const pages = pageCountFor(countForFilter(snapshot.tasksByState, taskStateFilter))
      if (tasksPage > pages) tasksPage = pages
      await loadTasks()
    } catch (error) {
      fail(error, 'detail')
    }
  }

  async function loadTasks(): Promise<void> {
    if (!detailId) return
    tasksLoading = true
    try {
      const page = await processingListTasks({
        batchId: detailId,
        state: taskStateFilter || undefined,
        offset: (tasksPage - 1) * TASKS_PER_PAGE,
        limit: TASKS_PER_PAGE,
      })
      tasks = page.tasks
    } catch (error) {
      fail(error, 'tasks')
    } finally {
      tasksLoading = false
    }
  }

  function canPause(state: string): boolean {
    return state === 'running'
  }

  function canResume(state: string): boolean {
    // `ready` belongs here too. The draft panel is component state, so a batch
    // prepared but never started is unreachable after a reload or a trip to
    // another tab — and resume is exactly what start does to it: ask for
    // desired_state = run. Without this its only exit was cancellation.
    return state === 'ready' || state === 'pausing' || state === 'paused' || state === 'interrupted'
  }

  function canCancel(state: string): boolean {
    return !isTerminalBatchState(state) && state !== 'cancelling'
  }

  function canRetry(snapshot: BatchSnapshot): boolean {
    return (
      snapshot.state !== 'cancelling' &&
      snapshot.state !== 'cancelled' &&
      snapshot.tasksByState.some((count) => count.name === 'failed' && count.count > 0)
    )
  }

  function toggleCollection(id: string): void {
    if (selected[id]) {
      const next = { ...selected }
      delete next[id]
      selected = next
    } else {
      selected = { ...selected, [id]: true }
    }
  }

  function toggleSelectAll(): void {
    if (allSelected) {
      selected = {}
    } else {
      selected = Object.fromEntries(collections.map((collection) => [collection.id, true]))
    }
  }

  async function handleAnalyze(): Promise<void> {
    if (selectedCount === 0 || (!runOcr && !runEmbeddings)) return
    analyzing = true
    feedback = null
    try {
      const operations = [...(runOcr ? ['ocr'] : []), ...(runEmbeddings ? ['embeddings'] : [])]
      const key = JSON.stringify([Object.keys(selected).sort(), operations])
      if (prepareRequest?.key !== key) prepareRequest = { key, id: newBatchRequestId() }
      const response = await processingPrepare(prepareRequest.id, Object.keys(selected), operations)
      draftId = response.batchId
      prepareRequest = null
      draft = await processingGetBatch(response.batchId)
      // processing_prepare returns the moment the batch row exists. Classifying
      // its members and flipping planning_done runs on the supervisor thread
      // afterwards, and the only `processing:changed` emitter is the per-task
      // commit observer — so nothing announces the end of planning.
      //
      // The store polls while it can see active work, and a fresh draft IS that
      // work, but it will never look on its own: polling arms from a refresh,
      // and creating the draft is what gives it something to find. Refresh here
      // and the subscription in onMount keeps this snapshot current until
      // planningDone flips and the start button unlocks.
      await loadLists()
      void batchStore.refresh()
    } catch (error) {
      fail(error, 'analyze')
    } finally {
      analyzing = false
    }
  }

  async function handleStart(): Promise<void> {
    if (!draftId) return
    starting = true
    try {
      const snapshot = await processingStart(draftId)
      draftId = null
      draft = null
      await loadLists()
      await openDetail(snapshot.id)
      void batchStore.refresh()
    } catch (error) {
      fail(error, 'start')
    } finally {
      starting = false
    }
  }

  async function handleDiscardDraft(): Promise<void> {
    if (!draftId) return
    try {
      await processingControl('cancel', draftId)
      draftId = null
      draft = null
      await batchStore.refresh()
    } catch (error) {
      fail(error, 'cancel')
    }
  }

  async function handleControl(
    batch: BatchSummary | BatchSnapshot,
    action: 'pause' | 'resume' | 'cancel'
  ): Promise<void> {
    if (action === 'cancel') {
      cancelTarget = batch
      return
    }
    busyBatchId = batch.id
    try {
      await processingControl(action, batch.id, batch.revision)
      await loadLists()
      if (detailId === batch.id) await refreshDetail()
      void batchStore.refresh()
    } catch (error) {
      fail(error, action)
    } finally {
      busyBatchId = null
    }
  }

  async function handleConfirmCancel(): Promise<void> {
    if (!cancelTarget) return
    const batch = cancelTarget
    cancelTarget = null
    busyBatchId = batch.id
    try {
      await processingControl('cancel', batch.id, batch.revision)
      await loadLists()
      if (detailId === batch.id) await refreshDetail()
      void batchStore.refresh()
    } catch (error) {
      fail(error, 'cancel')
    } finally {
      busyBatchId = null
    }
  }

  async function handleRetryTask(batchId: string, taskId: string): Promise<void> {
    retryingTaskId = taskId
    try {
      await processingRetry(newBatchRequestId(), batchId, taskId)
      await refreshDetail()
      await loadLists()
      void batchStore.refresh()
    } catch (error) {
      fail(error, 'retry')
    } finally {
      retryingTaskId = null
    }
  }

  async function handleRetryFailed(batchId: string): Promise<void> {
    retryingAll = true
    try {
      await processingRetry(newBatchRequestId(), batchId, undefined, true)
      await refreshDetail()
      await loadLists()
      void batchStore.refresh()
    } catch (error) {
      fail(error, 'retry')
    } finally {
      retryingAll = false
    }
  }

  async function toggleExpanded(batchId: string, taskId: string): Promise<void> {
    if (expandedTaskId === taskId) {
      expandedTaskId = null
      expandedTask = null
      return
    }
    expandedTaskId = taskId
    try {
      expandedTask = await processingGetTask(batchId, taskId)
    } catch (error) {
      fail(error, 'task')
    }
  }

  function progressOf(snapshot: BatchSnapshot): {
    total: number
    settled: number
    succeeded: number
    failed: number
    ratio: number | null
  } {
    return batchProgress(snapshot)
  }

  function stateLabel(state: string): string {
    switch (state) {
      case 'pending':
        return t('batch.statePending')
      case 'running':
        return t('batch.stateRunning')
      case 'interrupted':
        return t('batch.stateInterrupted')
      case 'blocked':
        return t('batch.stateBlocked')
      case 'failed':
        return t('batch.stateFailed')
      case 'succeeded':
        return t('batch.stateSucceeded')
      case 'cancelled':
        return t('batch.stateCancelled')
      case 'skipped':
        return t('batch.stateSkipped')
      default:
        return state
    }
  }

  /* `stateLabel` names a BUCKET of tasks, so it reads in the plural
     ("Completados"). A batch row names ONE batch, and the backend hands us its
     state as a raw identifier — `completed`, `completed_with_errors`. Rendering
     that identifier is what put English snake_case in a Spanish table. */
  function batchStateLabel(state: string): string {
    switch (state) {
      case 'preparing':
        return t('batch.batchStatePreparing')
      case 'ready':
        return t('batch.batchStateReady')
      case 'running':
        return t('batch.batchStateRunning')
      case 'pausing':
        return t('batch.batchStatePausing')
      case 'paused':
        return t('batch.batchStatePaused')
      case 'interrupted':
        return t('batch.batchStateInterrupted')
      case 'cancelling':
        return t('batch.batchStateCancelling')
      case 'cancelled':
        return t('batch.batchStateCancelled')
      case 'completed':
        return t('batch.batchStateCompleted')
      case 'completed_with_errors':
        return t('batch.batchStateCompletedWithErrors')
      default:
        return state
    }
  }

  /* The summary counts units, never a `total` field, so the column has to add
     them up: settled work plus whatever is still in flight. */
  function unitTotal(batch: BatchSummary): number {
    return batch.succeededUnits + batch.failedUnits + batch.activeUnits
  }

  function formatWhen(value: number | null): string {
    if (value == null) return '—'
    $currentLocale
    return new Date(value).toLocaleString()
  }

  /* The backend's own word for how an attempt ended: `open` while it runs,
     then succeeded, failed, cancelled or interrupted. It was reaching the
     Spanish UI untranslated. */
  function attemptOutcomeLabel(outcome: string): string {
    switch (outcome) {
      case 'open':
        return t('batch.attemptRunning')
      case 'succeeded':
        return t('batch.attemptSucceeded')
      case 'failed':
        return t('batch.attemptFailed')
      case 'cancelled':
        return t('batch.attemptCancelled')
      case 'interrupted':
        return t('batch.attemptInterrupted')
      default:
        return outcome
    }
  }

  /* How long an attempt took — the one thing about it the row cannot show.
     The units are symbols, so they read the same in either language. */
  function formatDuration(startedAt: number, finishedAt: number | null): string | null {
    if (finishedAt == null) return null
    const elapsed = Math.max(0, finishedAt - startedAt)
    if (elapsed < 1000) return `${elapsed} ms`
    if (elapsed < 60_000) return `${(elapsed / 1000).toFixed(1)} s`
    return `${Math.floor(elapsed / 60_000)} min ${Math.round((elapsed % 60_000) / 1000)} s`
  }

  /* A history cell gets the day only: a full timestamp is the widest thing in
     the table and the least often read. The row's tooltip carries the rest. */
  function formatDay(value: number | null): string {
    if (value == null) return '—'
    $currentLocale
    return new Date(value).toLocaleDateString()
  }

  onMount(() => {
    void loadCollections()
    void loadLists()
    const unsubscribeStore = batchStore.subscribe((summary) => {
      const init = summary.init
      if (init?.recovered && !recoveryDismissed) {
        recovered = {
          batches: init.pendingBatches,
          done: init.succeededTasks,
          todo: init.activeTasks,
          stuck: init.interruptedTasks,
        }
      }
      if (detailId) void refreshDetail()
      if (draftId) {
        const id = draftId
        void processingGetBatch(id)
          .then((snapshot) => {
            if (draftId === id) draft = snapshot
          })
          .catch((error) => fail(error, 'draft'))
      }
      void loadLists()
    })
    void batchStore.initialize()
    const unsubscribeFocus = batchStore.subscribeFocus((focus) => {
      if (focus.batchId) void openDetail(focus.batchId)
    })
    return () => {
      unsubscribeStore()
      unsubscribeFocus()
    }
  })

  onDestroy(() => {
    // Module-level store outlives the tab (indicator keeps polling).
  })
</script>

<section class="batch-tab" aria-label={t('batch.title')}>
  <div class="batch-tab__header">
    <div>
      <h2>{t('batch.title')}</h2>
      <p>{t('batch.subtitle')}</p>
    </div>
  </div>

  {#if feedback}
    <p
      class="surface-message"
      class:surface-message--error={feedback.tone === 'error'}
      class:surface-message--success={feedback.tone === 'success'}
      role={feedback.tone === 'error' ? 'alert' : 'status'}
    >
      {feedback.text}
    </p>
  {/if}

  {#if loadError}
    <div class="surface-message surface-message--error" role="alert">
      <span>{t('batch.loadError', { error: loadError })}</span>
      <Button
        variant="secondary"
        size="sm"
        onclick={() => {
          loadError = null
          void loadCollections()
        }}
      >
        {t('settings.retryLoad')}
      </Button>
    </div>
  {/if}

  {#if recovered && !recoveryDismissed}
    <div class="surface-message batch-tab__recovery" role="status">
      <span>
        {t('batch.recovered', {
          batches: recovered.batches,
          done: recovered.done,
          todo: recovered.todo,
          stuck: recovered.stuck,
        })}
      </span>
      <div class="batch-tab__recovery-actions">
        <Button
          variant="secondary"
          size="sm"
          onclick={() => {
            recoveryDismissed = true
          }}
        >
          {t('batch.keepPaused')}
        </Button>
        <Button
          variant="secondary"
          size="sm"
          onclick={() => {
            recoveryDismissed = true
            void processingControl('resume').catch((error) => fail(error, 'resume'))
          }}
        >
          {t('batch.resumeSelected')}
        </Button>
      </div>
    </div>
  {/if}

  {#if detail && detailId}
    <Card padding="sm">
      {@const collectionNames = detail.collections.map((collection) => collection.name).join(', ')}
      <!-- Back, title, subject and controls on one line. The title stacked
           over the collection names was most of this header's height, and the
           names are what identifies the batch — so they sit beside the title
           and take the slack as an ellipsis. -->
      <div class="batch-tab__detail-head">
        <Button variant="secondary" size="sm" onclick={() => (detailId = null)}>
          <ActionIcon name="chevron-left" size={14} />
          {t('batch.active')}
        </Button>
        <div class="batch-detail__title">
          <h3 class="batch-tab__panel-title">{t('batch.detail')}</h3>
          <span class="batch-detail__subject" use:tooltip={collectionNames}>
            {collectionNames}
          </span>
        </div>
        <div class="batch-tab__detail-actions">
          {#if canPause(detail.state)}
            <Button
              variant="secondary"
              size="sm"
              disabled={busyBatchId === detail.id}
              onclick={() => handleControl(detail!, 'pause')}
            >
              {t('batch.pause')}
            </Button>
          {:else if canResume(detail.state)}
            <Button
              variant="secondary"
              size="sm"
              disabled={busyBatchId === detail.id}
              onclick={() => handleControl(detail!, 'resume')}
            >
              {t('batch.resume')}
            </Button>
          {/if}
          {#if canCancel(detail.state)}
            <Button
              variant="secondary"
              size="sm"
              disabled={busyBatchId === detail.id}
              onclick={() => handleControl(detail!, 'cancel')}
            >
              {t('batch.cancel')}
            </Button>
          {/if}
          {#if canRetry(detail)}
            <Button
              variant="secondary"
              size="sm"
              disabled={busyBatchId === detail.id}
              onclick={() => handleRetryFailed(detail!.id)}
            >
              {retryingAll ? t('batch.retrying') : t('batch.retryFailed')}
            </Button>
          {/if}
        </div>
      </div>

      {@const progress = progressOf(detail)}
      {#if progress.ratio === null}
        <p class="batch-tab__empty">{t('batch.noWork')}</p>
      {:else}
        <!-- The counts were three sentences spread down the panel. As one row
             of stats they are comparable at a glance and cost one line. -->
        <div class="batch-stats">
          <span class="batch-stats__item">
            <b>{progress.settled}</b>
            {t('batch.summarySettled')}
          </span>
          <span class="batch-stats__item">
            <b>{progress.total - progress.settled}</b>
            {t('batch.summaryPending')}
          </span>
          <span class="batch-stats__item" class:batch-stats__item--alert={progress.failed > 0}>
            <b>{progress.failed}</b>
            {t('batch.summaryErrors')}
          </span>
          <span class="batch-stats__item">
            <b>{progress.total}</b>
            {t('batch.summaryTotal')}
          </span>
        </div>
        <div
          class="batch-tab__progress"
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(progress.ratio * 100)}
          aria-label={t('batch.resolvedOf', { done: progress.settled, total: progress.total })}
        >
          <div
            class="batch-tab__progress-bar"
            style={`width: ${Math.round(progress.ratio * 100)}%`}
          ></div>
        </div>
      {/if}

      <div class="batch-toolbar">
        <span class="batch-toolbar__label" id="batch-task-filter-label">
          {t('batch.filterState')}
        </span>
        <ToolbarMenu label={t('batch.filterState')} items={taskFilterItems}>
          {#snippet trigger(props, { open })}
            <button
              type="button"
              class="batch-select"
              class:batch-select--open={open}
              aria-labelledby="batch-task-filter-label batch-task-filter-value"
              {...props}
            >
              <span id="batch-task-filter-value">{taskFilterLabel}</span>
              <ActionIcon name="chevron-down" size={12} />
            </button>
          {/snippet}
        </ToolbarMenu>
        <span class="batch-toolbar__count">
          {t('batch.resultCount', { count: taskTotal })}
        </span>
      </div>

      <!-- Asked of the snapshot's own counts, not of `tasks`, which is empty
           for a tick every time a page is fetched: "sin resultados" flashing
           over a list that is merely loading is a lie, and a brief one is
           still one. -->
      {#if taskTotal === 0}
        <p class="batch-tab__empty">{t('batch.noFilterResults')}</p>
      {:else}
        <!-- One page of units, dealt into as many tables as the panel is wide
             enough to hold. Each unit carries four short values, so a single
             full-width table spends most of its width on nothing. -->
        <div
          class="batch-tasks__grid"
          style={`grid-template-columns: repeat(${tableCount}, minmax(0, 1fr))`}
          bind:clientWidth={listWidth}
        >
          {#each taskTables as table, tableIndex (tableIndex)}
            <div class="batch-table__scroll">
              <div
                class="batch-table batch-table--tasks"
                role="table"
                aria-label={t('batch.tasksTable')}
              >
                <div class="batch-table__row batch-table__row--head" role="row">
                  <span class="batch-table__cell batch-table__cell--text" role="columnheader">
                    {t('batch.colOperation')}
                  </span>
                  <span class="batch-table__cell batch-table__cell--text" role="columnheader">
                    {t('batch.colState')}
                  </span>
                  <span class="batch-table__cell batch-table__cell--num" role="columnheader">
                    {t('batch.colProgress')}
                  </span>
                  <span class="batch-table__cell batch-table__cell--num" role="columnheader">
                    {t('batch.colAttempts')}
                  </span>
                </div>
                {#each table as task (task.taskId)}
                  <div class="batch-table__row" role="row">
                    <span class="batch-table__cell batch-table__cell--text" role="cell">
                      <!-- The whole row used to be the disclosure button, which
                           a grid row carrying `role="row"` cannot also be. The
                           control lives in the first cell instead, with the
                           chevron next to the operation it opens — and only on
                           the units that have something under it to open. -->
                      {#if taskHasDetail(task)}
                        <button
                          type="button"
                          class="batch-table__open batch-tasks__trigger"
                          aria-expanded={expandedTaskId === task.taskId}
                          aria-label={t('batch.expandTask', {
                            kind: task.kind,
                            asset: task.assetId,
                          })}
                          onclick={() => toggleExpanded(detailId!, task.taskId)}
                        >
                          <ActionIcon
                            name={expandedTaskId === task.taskId ? 'chevron-down' : 'chevron-right'}
                            size={12}
                          />
                          <span>{task.kind}</span>
                        </button>
                      {:else}
                        <span class="batch-tasks__plain">{task.kind}</span>
                      {/if}
                    </span>
                    <!-- The message has no column of its own: it is empty on
                         every row that succeeded, which is nearly all of them.
                         It reaches the eye through the tooltip and the whole of
                         it through the chevron. -->
                    <span
                      class="batch-table__cell batch-table__cell--text"
                      class:batch-table__cell--alert={Boolean(task.errorMessage)}
                      role="cell"
                      use:tooltip={task.errorMessage ?? undefined}
                    >
                      {stateLabel(task.state)}
                    </span>
                    <span class="batch-table__cell batch-table__cell--num" role="cell">
                      {#if task.progressTotal > 0}
                        {task.progressDone}/{task.progressTotal}
                      {:else}
                        —
                      {/if}
                    </span>
                    <span class="batch-table__cell batch-table__cell--num" role="cell">
                      {task.attemptCount}
                    </span>
                  </div>
                  {#if expandedTaskId === task.taskId && expandedTask}
                    <!-- Its own row with one full-width cell, not a stowaway
                         inside the task's row: a row may hold cells and nothing
                         else. -->
                    <div class="batch-table__row batch-tasks__detail-row" role="row">
                      <div class="batch-table__cell batch-tasks__detail" role="cell">
                        {#if task.errorMessage}
                          <p class="batch-tasks__detail-error">{task.errorMessage}</p>
                        {/if}
                        {#if expandedTask.nextRetryAt}
                          <p>
                            {t('batch.nextRetry', { when: formatWhen(expandedTask.nextRetryAt) })}
                          </p>
                        {/if}
                        <p>
                          {t('batch.cycle', { n: expandedTask.retryCycle })} ·
                          {t('batch.checkpointCount', { count: expandedTask.checkpoints.length })}
                        </p>
                        {#if expandedTask.sharedWithBatches.length > 1}
                          <p>
                            {t('batch.sharedWith', {
                              ids: expandedTask.sharedWithBatches.join(', '),
                            })}
                          </p>
                        {/if}
                        <ol>
                          {#each expandedTask.attempts as attempt (attempt.attemptNumber)}
                            {@const took = formatDuration(attempt.startedAt, attempt.finishedAt)}
                            <li>
                              #{attempt.attemptNumber} · {attemptOutcomeLabel(attempt.outcome)}{took
                                ? ` · ${took}`
                                : ''}{attempt.errorCode
                                ? ` · ${attempt.errorCode}`
                                : ''}{attempt.errorMessage ? ` · ${attempt.errorMessage}` : ''}
                            </li>
                          {/each}
                        </ol>
                        {#if task.state === 'failed'}
                          <div class="batch-tasks__detail-actions">
                            <Button
                              variant="secondary"
                              size="sm"
                              disabled={retryingTaskId === task.taskId}
                              onclick={() => handleRetryTask(detailId!, task.taskId)}
                            >
                              {t('batch.retry')}
                            </Button>
                          </div>
                        {/if}
                      </div>
                    </div>
                  {/if}
                {/each}
              </div>
            </div>
          {/each}
        </div>

        {@const from = (tasksPage - 1) * TASKS_PER_PAGE + 1}
        {@const to = Math.min(tasksPage * TASKS_PER_PAGE, taskTotal)}
        <nav class="batch-pager" aria-label={t('batch.pagination')}>
          <span class="batch-pager__range">
            {t('batch.pageRange', { from, to, total: taskTotal })}
          </span>
          <div class="batch-pager__controls">
            <button
              type="button"
              class="batch-pager__step"
              disabled={tasksPage === 1 || tasksLoading}
              aria-label={t('batch.firstPage')}
              use:tooltip={t('batch.firstPage')}
              onclick={() => goToPage(1)}
            >
              <ActionIcon name="chevrons-left" size={14} />
            </button>
            <button
              type="button"
              class="batch-pager__step"
              disabled={tasksPage === 1 || tasksLoading}
              aria-label={t('batch.prevPage')}
              use:tooltip={t('batch.prevPage')}
              onclick={() => goToPage(tasksPage - 1)}
            >
              <ActionIcon name="chevron-left" size={14} />
            </button>
            {#each pageItems as item, index (index)}
              {#if item === 'gap'}
                <span class="batch-pager__gap" aria-hidden="true">…</span>
              {:else}
                <button
                  type="button"
                  class="batch-pager__page"
                  class:batch-pager__page--current={item === tasksPage}
                  aria-current={item === tasksPage ? 'page' : undefined}
                  aria-label={t('batch.goToPage', { page: item })}
                  disabled={tasksLoading}
                  onclick={() => goToPage(item)}
                >
                  {item}
                </button>
              {/if}
            {/each}
            <button
              type="button"
              class="batch-pager__step"
              disabled={tasksPage === pageCount || tasksLoading}
              aria-label={t('batch.nextPage')}
              use:tooltip={t('batch.nextPage')}
              onclick={() => goToPage(tasksPage + 1)}
            >
              <ActionIcon name="chevron-right" size={14} />
            </button>
            <button
              type="button"
              class="batch-pager__step"
              disabled={tasksPage === pageCount || tasksLoading}
              aria-label={t('batch.lastPage')}
              use:tooltip={t('batch.lastPage')}
              onclick={() => goToPage(pageCount)}
            >
              <ActionIcon name="chevrons-right" size={14} />
            </button>
          </div>
        </nav>
      {/if}
    </Card>
  {:else}
    <Card padding="sm">
      <h3 class="batch-tab__panel-title">{t('batch.newBatch')}</h3>
      {#if collectionsLoading}
        <p role="status">{t('batch.preparing')}</p>
      {:else}
        <!-- A <fieldset> renders its <legend> OUTSIDE the flex formatting
             context it establishes, so no `gap` and no `align-items` ever
             reached it: that is what let the legend sit on top of the row it
             was supposed to head. `role="group"` + `aria-labelledby` names the
             group exactly as a legend does, and the label becomes an ordinary
             flex item that the row can actually lay out. -->
        <div class="batch-field" role="group" aria-labelledby="batch-collections-label">
          <div class="batch-field__head">
            <span class="batch-field__legend" id="batch-collections-label"
              >{t('batch.collections')}</span
            >
            <Checkbox
              class="batch-field__select-all"
              checked={allSelected}
              onchange={toggleSelectAll}
            >
              {t('batch.selectAll')} · {t('batch.selectedCount', { count: selectedCount })}
            </Checkbox>
          </div>
          <div class="batch-field__scope-list">
            {#each collections as collection (collection.id)}
              <Checkbox
                class="batch-field__scope"
                checked={Boolean(selected[collection.id])}
                onchange={() => toggleCollection(collection.id)}
              >
                <span class="batch-field__scope-row">
                  <span class="batch-field__scope-name" use:tooltip={collection.name}
                    >{collection.name}</span
                  >
                  <span class="batch-field__count">{collection.items}</span>
                </span>
              </Checkbox>
            {/each}
          </div>
        </div>
        <div class="batch-ops" role="group" aria-labelledby="batch-ops-label">
          <span class="batch-field__legend" id="batch-ops-label">{t('batch.operations')}</span>
          <Checkbox class="batch-ops__toggle" bind:checked={runOcr}>{t('batch.opOcr')}</Checkbox>
          <!-- The hint used to be a paragraph under the row, which is the one
               place a single-line control strip cannot afford. The tooltip
               action also wires `aria-describedby`, so the sentence still
               reaches a screen reader off the checkbox it explains. -->
          <span class="batch-ops__hinted" use:tooltip={t('batch.opEmbeddingsHint')}>
            <Checkbox class="batch-ops__toggle" bind:checked={runEmbeddings}
              >{t('batch.opEmbeddings')}</Checkbox
            >
          </span>
          <Button
            class="batch-ops__action"
            variant="secondary"
            size="sm"
            disabled={selectedCount === 0 || (!runOcr && !runEmbeddings) || analyzing}
            onclick={handleAnalyze}
          >
            {t('batch.analyze')}
          </Button>
        </div>
        {#if draft && draftId}
          <div class="batch-tab__draft">
            <p>{t('batch.preparing')} {draft.membersClassified}/{draft.membersTotal}</p>
            <div class="batch-tab__actions">
              <Button variant="secondary" size="sm" onclick={handleDiscardDraft}>
                {t('batch.discard')}
              </Button>
              <Button
                variant="secondary"
                size="sm"
                disabled={starting || !draft.planningDone}
                onclick={handleStart}
              >
                {t('batch.start')}
              </Button>
            </div>
          </div>
        {/if}
      {/if}
    </Card>

    <div class="batch-tab__panels">
      <Card padding="sm">
        <h3 class="batch-tab__panel-title">{t('batch.active')}</h3>
        {#if activeBatches.length === 0}
          <p class="batch-tab__empty">{t('batch.noWork')}</p>
        {:else}
          <ul class="batch-tab__batches">
            {#each activeBatches as batch (batch.id)}
              {@const summary = batch}
              <li class="batch-tab__batch">
                <button
                  type="button"
                  class="batch-tab__batch-row"
                  onclick={() => openDetail(batch.id)}
                >
                  <span class="batch-tab__batch-id" use:tooltip={summary.id}>{summary.id}</span>
                  <span class="batch-tab__batch-state">{batchStateLabel(summary.state)}</span>
                  <span class="batch-tab__counts">
                    <span class="batch-tab__count" use:tooltip={t('batch.stateSucceeded')}
                      >{summary.succeededUnits}
                      <ActionIcon name="circle-check" size={12} /><span class="sr-only"
                        >{t('batch.stateSucceeded')}</span
                      ></span
                    >
                    <span aria-hidden="true">·</span>
                    <span class="batch-tab__count" use:tooltip={t('batch.stateFailed')}
                      >{summary.failedUnits}
                      <ActionIcon name="circle-x" size={12} /><span class="sr-only"
                        >{t('batch.stateFailed')}</span
                      ></span
                    >
                    <span aria-hidden="true">·</span>
                    <span class="batch-tab__count" use:tooltip={t('batch.stateRunning')}
                      >{summary.activeUnits}
                      <ActionIcon name="loader" size={12} /><span class="sr-only"
                        >{t('batch.stateRunning')}</span
                      ></span
                    >
                  </span>
                </button>
                <div class="batch-tab__batch-actions">
                  {#if canPause(batch.state)}
                    <Button
                      variant="secondary"
                      size="sm"
                      disabled={busyBatchId === batch.id}
                      onclick={() => handleControl(batch, 'pause')}
                      aria-label={t('batch.pause')}
                    >
                      <ActionIcon name="pause" size={16} />
                    </Button>
                  {:else if canResume(batch.state)}
                    <Button
                      variant="secondary"
                      size="sm"
                      disabled={busyBatchId === batch.id}
                      onclick={() => handleControl(batch, 'resume')}
                      aria-label={t('batch.resume')}
                    >
                      <ActionIcon name="play" size={16} />
                    </Button>
                  {/if}
                  {#if canCancel(batch.state)}
                    <Button
                      variant="secondary"
                      size="sm"
                      disabled={busyBatchId === batch.id}
                      onclick={() => handleControl(batch, 'cancel')}
                      aria-label={t('batch.cancel')}
                    >
                      <ActionIcon name="close" size={16} />
                    </Button>
                  {/if}
                </div>
              </li>
            {/each}
          </ul>
        {/if}
      </Card>

      <Card padding="sm">
        <h3 class="batch-tab__panel-title">{t('batch.history')}</h3>
        {#if historyBatches.length === 0}
          <p class="batch-tab__empty">{t('batch.noWork')}</p>
        {:else}
          <!-- Every number here used to be a clause in a sentence ("4 ✓ · 1 ✗"),
               which is unscannable down a column: a table lets the eye compare
               row to row instead of re-reading each line.

               Built from ARIA roles rather than <table>, because an {#each}
               inside a real one compiles to a template whose root is a bare
               <tr>, and the test environment's parser drops a row with no
               table around it — the fragment comes back empty and the block
               throws on mount. The roles give assistive tech the same table;
               the grid gives the columns something <table> cannot: one
               declaration that every row, header included, shares. -->
          <div class="batch-table__scroll">
            <div
              class="batch-table batch-table--history"
              role="table"
              aria-label={t('batch.historyTable')}
            >
              <div class="batch-table__row batch-table__row--head" role="row">
                <span class="batch-table__cell batch-table__cell--id" role="columnheader">
                  {t('batch.colBatch')}
                </span>
                <span class="batch-table__cell batch-table__cell--text" role="columnheader">
                  {t('batch.colState')}
                </span>
                <span class="batch-table__cell batch-table__cell--text" role="columnheader">
                  {t('batch.colDate')}
                </span>
                <span
                  class="batch-table__cell batch-table__cell--flag"
                  role="columnheader"
                  use:tooltip={t('batch.opOcr')}
                >
                  {t('batch.colOcrShort')}
                </span>
                <span
                  class="batch-table__cell batch-table__cell--flag"
                  role="columnheader"
                  use:tooltip={t('batch.opEmbeddings')}
                >
                  {t('batch.colEmbeddingsShort')}
                </span>
                <span class="batch-table__cell batch-table__cell--num" role="columnheader">
                  {t('batch.colTotal')}
                </span>
                <span class="batch-table__cell batch-table__cell--num" role="columnheader">
                  {t('batch.colErrors')}
                </span>
              </div>
              {#each historyBatches as batch (batch.id)}
                <div class="batch-table__row" role="row">
                  <span class="batch-table__cell batch-table__cell--id" role="cell">
                    <button
                      type="button"
                      class="batch-table__open"
                      use:tooltip={batch.id}
                      onclick={() => openDetail(batch.id)}
                    >
                      {batch.id}
                    </button>
                  </span>
                  <span
                    class="batch-table__cell batch-table__cell--text"
                    role="cell"
                    use:tooltip={batchStateLabel(batch.state)}
                  >
                    {batchStateLabel(batch.state)}
                  </span>
                  <span class="batch-table__cell batch-table__cell--text" role="cell">
                    {formatDay(batch.createdAt)}
                  </span>
                  <span class="batch-table__cell batch-table__cell--flag" role="cell">
                    {#if batch.operations.includes('ocr')}
                      <ActionIcon name="check" size={12} />
                      <span class="sr-only">{t('batch.opIncluded')}</span>
                    {:else}
                      <span aria-hidden="true">—</span>
                      <span class="sr-only">{t('batch.opNotIncluded')}</span>
                    {/if}
                  </span>
                  <span class="batch-table__cell batch-table__cell--flag" role="cell">
                    {#if batch.operations.includes('embeddings')}
                      <ActionIcon name="check" size={12} />
                      <span class="sr-only">{t('batch.opIncluded')}</span>
                    {:else}
                      <span aria-hidden="true">—</span>
                      <span class="sr-only">{t('batch.opNotIncluded')}</span>
                    {/if}
                  </span>
                  <span class="batch-table__cell batch-table__cell--num" role="cell">
                    {unitTotal(batch)}
                  </span>
                  <span
                    class="batch-table__cell batch-table__cell--num"
                    class:batch-table__cell--alert={batch.failedUnits > 0}
                    role="cell"
                  >
                    {batch.failedUnits}
                  </span>
                </div>
              {/each}
            </div>
          </div>
          {#if historyCursor}
            <Button
              variant="secondary"
              size="sm"
              disabled={historyLoading}
              onclick={loadMoreHistory}
            >
              {t('batch.loadMore')}
            </Button>
          {/if}
        {/if}
      </Card>
    </div>
  {/if}

  {#if cancelTarget}
    <ConfirmDialog
      title={t('batch.confirmCancelTitle')}
      titleId="batch-cancel-title"
      message={t('batch.confirmCancelMessage')}
      cancelLabel={t('settings.discardCancel')}
      confirmLabel={t('batch.confirmCancel')}
      variant="destructive"
      oncancel={() => (cancelTarget = null)}
      onconfirm={handleConfirmCancel}
    />
  {/if}
</section>

<style>
  .batch-tab {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  /* The two panels below the composer share the row 50/50 and drop to one
     column when the row is too narrow for both. `auto-fit` + `minmax` measures
     the GRID's own box, which is the only measurement that is right here: the
     tab sits beside the Configuración sidebar, so the viewport width a media
     query would read is not the width these panels get.
     `min(100%, 320px)` keeps the floor from exceeding the track in the
     one-column case, which is what would otherwise force an overflow.
     `align-items: start` is what stops an empty "Sin trabajo pendiente" panel
     from being stretched to the height of a long history. */
  .batch-tab__panels {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 320px), 1fr));
    gap: var(--space-4);
    align-items: start;
  }

  .batch-tab__panel-title {
    margin: 0;
  }

  .batch-tab__empty {
    margin: 0;
    color: var(--color-text-secondary);
  }

  .batch-field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  /* Label and "seleccionar todas" share one line instead of stacking: two
     rows of chrome above a picker is most of the empty space this panel had. */
  .batch-field__head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .batch-field__legend {
    padding: 0;
    color: var(--color-text-secondary);
    font-weight: var(--font-weight-medium);
  }

  .batch-field__head :global(.batch-field__select-all) {
    padding: var(--space-1) var(--space-2);
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  /* Operaciones is one strip: label, both toggles, and the action. The action
     takes the leftover space as a start margin, so it sits hard right on a
     full-width row and still lands at the END of whatever line it wraps onto —
     an absolute or floated right would have been the thing that overlaps. */
  .batch-ops {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    padding-top: var(--space-2);
    border-top: 1px solid var(--border-subtle);
  }

  .batch-ops :global(.batch-ops__action) {
    margin-inline-start: auto;
  }

  /* The tooltip action measures its own element, so the wrapper has to be a
     real box — `display: contents` would give it a zero rect to anchor to. */
  .batch-ops__hinted {
    display: inline-flex;
    min-width: 0;
  }

  .batch-ops :global(.batch-ops__toggle) {
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border-color: var(--border-subtle);
    background: var(--surface-input);
  }

  .batch-ops :global(.batch-ops__toggle:has(input:checked)) {
    background: var(--surface-toolbar);
    border-color: var(--border-panel);
  }

  /* The collection picker is the Colecciones grid in miniature: the same 260px
     track floor that page uses, and the card inside it is what gets tighter,
     since the only job here is ticking boxes.

     260 rather than a smaller floor because of what it yields, not by analogy:
     across the Lotes panel it steps 6 / 5 / 4 / 2 / 1 cards per row as the
     window narrows. A 220 floor packs eight into a wide window, which is denser
     than a picker wants.

     `auto-fit` + `minmax` measures the grid's OWN box, so the cards already
     reflow to whatever width the panel has — a container query would only
     restate what the track function is doing. */
  .batch-field__scope-list {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
    gap: var(--space-2);
    max-height: 200px;
    overflow-y: auto;
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    background: var(--surface-input);
  }

  /* The card IS the Checkbox, not a div wrapped around one. Its <label> already
     makes the whole surface a click target, already toggles on Space, and
     already draws exactly one focus ring — so there is no wrapper handler to
     fire twice and no second ring to hide.

     What is left to state is the lift off the sunken list, and a checked border
     firmer than the resting one: the component's own checked rule lands on
     --border-subtle, which is this card's RESTING border, so without this the
     selected state would announce itself with nothing but the tick. */
  .batch-field__scope-list :global(.batch-field__scope) {
    gap: var(--space-2);
    padding: var(--space-2);
    min-width: 0;
    background: var(--surface-panel);
    border-color: var(--border-subtle);
  }

  .batch-field__scope-list :global(.batch-field__scope:hover) {
    background: var(--surface-toolbar);
  }

  .batch-field__scope-list :global(.batch-field__scope:has(input:checked)) {
    background: var(--surface-toolbar);
    border-color: var(--border-panel);
  }

  .batch-field__scope-row {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    min-width: 0;
  }

  /* The name takes the slack and gives it back as an ellipsis, so a long
     collection can never widen its own card or push the count off the edge. */
  .batch-field__scope-name {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: var(--font-weight-medium);
  }

  .batch-field__count {
    flex: none;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    font-variant-numeric: tabular-nums;
  }

  .batch-tab__progress {
    height: var(--space-1);
    border-radius: var(--radius-full);
    background: var(--surface-input);
    border: 1px solid var(--border-subtle);
    overflow: hidden;
  }

  /* Progress reads by fill, not by hue: the palette stays monochrome. */
  .batch-tab__progress-bar {
    height: 100%;
    background: color-mix(in srgb, var(--color-text-primary) 45%, transparent);
    transition: width var(--transition-smooth);
  }

  .batch-tab__batches {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .batch-tab__batch-row {
    display: flex;
    gap: var(--space-3);
    align-items: baseline;
    width: 100%;
    text-align: start;
    background: none;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    color: inherit;
    cursor: pointer;
    padding: var(--space-2) var(--space-3);
    transition: background-color var(--transition-base);
  }

  .batch-tab__batch {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  /* Three tracks, not a baseline flex row: a grid is what keeps the state and
     the counts in the same place down the list however long an id is. */
  .batch-tab__batch-row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    gap: var(--space-2);
    align-items: center;
    padding: var(--space-1) var(--space-2);
  }

  .batch-tab__batch-id {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .batch-tab__batch-state {
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    white-space: nowrap;
  }

  .batch-tab__batch-row:hover {
    background: var(--surface-toolbar);
  }

  .batch-tab__batch-row:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  /* The counts sit in a baseline-aligned row, so each one is its own
     inline-flex box: the icon centres against its own number rather than
     dropping to the row's baseline. */
  .batch-tab__counts,
  .batch-tab__count {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }

  /* ActionIcon is aria-hidden by contract, so the glyph it replaced ('✓', '✗')
     took its meaning with it. This puts that meaning back as text the row's
     accessible name picks up. */
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    clip-path: inset(50%);
    white-space: nowrap;
    border: 0;
  }

  .batch-tab__detail-actions,
  .batch-tab__batch-actions,
  .batch-tab__actions,
  .batch-tab__recovery-actions {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .batch-tab__batch-actions {
    flex: none;
  }

  .batch-tab__actions {
    justify-content: flex-end;
  }

  /* Below its min-width the table scrolls sideways inside the panel rather
     than widening it — a grid track that grows is what breaks the 50/50. */
  .batch-table__scroll {
    overflow-x: auto;
  }

  .batch-table {
    font-size: var(--font-size-xs);
    font-variant-numeric: tabular-nums;
  }

  /* One flexible column takes the slack and gives it back as an ellipsis; the
     rest are sized so the numbers line up down the panel. Each row repeats the
     track list rather than the rows sharing one grid through
     `display: contents`, which drops a role-bearing element out of the
     accessibility tree in browsers that still carry that bug. */
  .batch-table__row {
    display: grid;
    align-items: center;
    gap: var(--space-2);
  }

  .batch-table--history {
    min-width: 32rem;
  }

  .batch-table--history .batch-table__row {
    grid-template-columns: minmax(0, 1fr) 8.5rem 6.5rem 2.75rem 2.75rem 3.5rem 3.5rem;
  }

  /* Four short values per unit, so the table is narrow on purpose: that is
     what lets two or three of them stand side by side instead of one spending
     its width on an empty band. Below this floor its own wrapper scrolls
     rather than the columns collapsing into each other. */
  .batch-table--tasks {
    min-width: 19rem;
  }

  .batch-table--tasks .batch-table__row {
    grid-template-columns: minmax(4.5rem, 1fr) minmax(5rem, 1.2fr) 4.5rem 4rem;
  }

  /* One track per table, from the same count that chunked the rows. */
  .batch-tasks__grid {
    display: grid;
    gap: var(--space-4);
    align-items: start;
  }

  .batch-table--tasks .batch-tasks__detail-row {
    grid-template-columns: minmax(0, 1fr);
  }

  .batch-table__cell {
    padding: var(--space-1) var(--space-2);
    min-width: 0;
    white-space: nowrap;
  }

  .batch-table__row--head .batch-table__cell {
    color: var(--color-text-muted);
    font-weight: var(--font-weight-medium);
  }

  /* One hairline under the header and between rows, nothing around the whole
     thing: the Card already draws the box, so a full border would be a second
     frame inside the first. */
  .batch-table__row--head {
    border-bottom: 1px solid var(--border-subtle);
  }

  .batch-table__row + .batch-table__row:not(.batch-table__row--head) {
    border-top: 1px solid color-mix(in srgb, var(--border-subtle) 55%, transparent);
  }

  .batch-table__row:not(.batch-table__row--head):hover {
    background: var(--surface-toolbar);
  }

  .batch-table__cell--id,
  .batch-table__cell--text {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .batch-table__cell--text {
    color: var(--color-text-secondary);
  }

  .batch-table__cell--flag {
    text-align: center;
    color: var(--color-text-muted);
  }

  .batch-table__cell--flag :global(svg) {
    vertical-align: middle;
  }

  .batch-table__cell--num {
    text-align: end;
  }

  .batch-table__cell--alert {
    color: var(--color-text-primary);
    font-weight: var(--font-weight-medium);
  }

  .batch-table__open {
    display: block;
    width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    text-align: start;
    padding: 0;
    background: none;
    border: 0;
    border-radius: var(--radius-xs);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }

  .batch-table__open:hover {
    text-decoration: underline;
  }

  .batch-table__open:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .batch-tasks__trigger {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }

  .batch-tasks__trigger :global(svg) {
    flex: none;
    color: var(--color-text-muted);
  }

  .batch-tasks__trigger > span {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Indented by exactly the chevron it does not have, so the operation names
     line up down the column whether or not a row can be opened. */
  .batch-tasks__plain {
    display: block;
    padding-inline-start: calc(12px + var(--space-1));
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .batch-tasks__detail {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2) var(--space-3);
    color: var(--color-text-secondary);
    white-space: normal;
  }

  .batch-tasks__detail p {
    margin: 0;
  }

  .batch-tasks__detail ol {
    margin: 0;
    padding-inline-start: var(--space-4);
  }

  .batch-tasks__detail-error {
    color: var(--color-text-primary);
  }

  .batch-tasks__detail-actions {
    display: flex;
    padding-top: var(--space-1);
  }

  .batch-pager {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  .batch-pager__range {
    font-variant-numeric: tabular-nums;
  }

  /* An auto start margin, so the controls sit hard right on a full line and
     still land at the end of whichever line they wrap onto. */
  .batch-pager__controls {
    display: flex;
    align-items: center;
    gap: 2px;
    margin-inline-start: auto;
  }

  .batch-pager__step,
  .batch-pager__page {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: var(--control-height-sm);
    height: var(--control-height-sm);
    padding: 0 var(--space-1);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--color-text-secondary);
    font: inherit;
    font-variant-numeric: tabular-nums;
    cursor: pointer;
    transition:
      background-color var(--transition-base),
      border-color var(--transition-base),
      color var(--transition-base);
  }

  .batch-pager__step:hover:not(:disabled),
  .batch-pager__page:hover:not(:disabled) {
    background: var(--surface-toolbar);
    color: var(--color-text-primary);
  }

  .batch-pager__step:focus-visible,
  .batch-pager__page:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .batch-pager__step:disabled,
  .batch-pager__page:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  /* The page you are on reads as contrast, never as hue. */
  .batch-pager__page--current {
    background: var(--surface-input);
    border-color: var(--border-panel);
    color: var(--color-text-primary);
  }

  .batch-pager__gap {
    padding: 0 var(--space-1);
    color: var(--color-text-muted);
  }

  /* Header, subject and controls on one line; the subject is what shrinks. */
  .batch-detail__title {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    min-width: 0;
  }

  .batch-detail__subject {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .batch-stats {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-2) var(--space-4);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .batch-stats__item {
    display: inline-flex;
    align-items: baseline;
    gap: var(--space-1);
  }

  .batch-stats__item b {
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    font-variant-numeric: tabular-nums;
  }

  .batch-toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  .batch-toolbar__count {
    margin-inline-start: auto;
    color: var(--color-text-muted);
    font-variant-numeric: tabular-nums;
  }

  /* The trigger of the state filter. It replaces a native <select>, so it
     states every surface the operating system used to decide: the sunken
     input background, one hairline, the app's control radius and height. */
  .batch-select {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    min-height: var(--control-height-sm);
    padding: 0 var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-control);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    cursor: pointer;
    transition:
      background-color var(--transition-base),
      border-color var(--transition-base);
  }

  .batch-select:hover,
  .batch-select--open {
    background: var(--surface-toolbar);
    border-color: var(--border-panel);
  }

  .batch-select:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .batch-select :global(svg) {
    color: var(--color-text-muted);
  }

  .batch-tab__detail-head {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2) var(--space-3);
    align-items: center;
    justify-content: space-between;
  }

  .batch-tab__draft {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-top: var(--space-3);
    border-top: 1px solid var(--border-subtle);
  }

  @media (prefers-reduced-motion: reduce) {
    .batch-tab__progress-bar,
    .batch-select,
    .batch-pager__step,
    .batch-pager__page,
    .batch-tab__batch-row {
      transition: none;
    }
  }
</style>
