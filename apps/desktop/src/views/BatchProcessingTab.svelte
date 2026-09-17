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
    type BatchSnapshot,
    type BatchSummary,
    type BatchTaskDetail,
    type BatchTaskSummary,
  } from '$lib/batch-processing'
  import { ActionIcon, Button, Card, Checkbox, ConfirmDialog } from '@entropia/ui'

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
  let tasksCursor = $state<string | null>(null)
  let tasksLoading = $state(false)
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
    expandedTaskId = null
    expandedTask = null
    taskStateFilter = ''
    tasksCursor = null
    await refreshDetail()
  }

  async function refreshDetail(): Promise<void> {
    if (!detailId) return
    try {
      detail = await processingGetBatch(detailId)
      await loadTasks(true)
    } catch (error) {
      fail(error, 'detail')
    }
  }

  async function loadTasks(reset: boolean): Promise<void> {
    if (!detailId) return
    if (reset) {
      tasks = []
      tasksCursor = null
    }
    tasksLoading = true
    try {
      const page = await processingListTasks({
        batchId: detailId,
        state: taskStateFilter || undefined,
        afterTaskId: tasksCursor ?? undefined,
        limit: 50,
      })
      tasks = reset ? page.tasks : [...tasks, ...page.tasks]
      tasksCursor = page.nextCursor
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

  function formatWhen(value: number | null): string {
    if (value == null) return '—'
    $currentLocale
    return new Date(value).toLocaleString()
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
    <Card>
      <div class="batch-tab__detail-head">
        <Button variant="secondary" size="sm" onclick={() => (detailId = null)}>
          ← {t('batch.active')}
        </Button>
        <div>
          <h3>{t('batch.detail')}</h3>
          <p>{detail.collections.map((collection) => collection.name).join(', ')}</p>
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
        <p>{t('batch.noWork')}</p>
      {:else}
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
        <p>{t('batch.resolvedOf', { done: progress.settled, total: progress.total })}</p>
      {/if}

      <div class="batch-tab__filters">
        <label>
          <span>{t('batch.filterAll')}</span>
          <select
            value={taskStateFilter}
            onchange={(event) => {
              taskStateFilter = event.currentTarget.value
              void loadTasks(true)
            }}
          >
            <option value="">{t('batch.filterAll')}</option>
            {#each TASK_STATE_FILTERS as state (state)}
              <option value={state}>{stateLabel(state)}</option>
            {/each}
          </select>
        </label>
      </div>

      <ul class="batch-tab__tasks">
        {#each tasks as task (task.taskId)}
          <li class="batch-tab__task">
            <button
              type="button"
              class="batch-tab__task-row"
              onclick={() => toggleExpanded(detailId!, task.taskId)}
              aria-expanded={expandedTaskId === task.taskId}
            >
              <span class="batch-tab__task-kind">{task.kind}</span>
              <span class="batch-tab__task-state">{stateLabel(task.state)}</span>
              <span class="batch-tab__task-progress">
                {#if task.progressTotal > 0}
                  {task.progressDone}/{task.progressTotal}
                {:else}
                  —
                {/if}
              </span>
              <span class="batch-tab__task-attempts">
                {t('batch.attempts', { count: task.attemptCount })}
              </span>
              {#if task.errorMessage}
                <span class="batch-tab__task-error">{task.errorMessage}</span>
              {/if}
            </button>
            {#if task.state === 'failed'}
              <Button
                variant="secondary"
                size="sm"
                disabled={retryingTaskId === task.taskId}
                onclick={() => handleRetryTask(detailId!, task.taskId)}
              >
                {t('batch.retry')}
              </Button>
            {/if}
            {#if expandedTaskId === task.taskId && expandedTask}
              <div class="batch-tab__task-detail">
                {#if expandedTask.errorMessage}
                  <p>{expandedTask.errorMessage}</p>
                {/if}
                {#if expandedTask.nextRetryAt}
                  <p>{t('batch.nextRetry', { when: formatWhen(expandedTask.nextRetryAt) })}</p>
                {/if}
                <p>
                  {t('batch.cycle', { n: expandedTask.retryCycle })} · checkpoints:
                  {expandedTask.checkpoints.length}
                </p>
                {#if expandedTask.sharedWithBatches.length > 1}
                  <p>shared: {expandedTask.sharedWithBatches.join(', ')}</p>
                {/if}
                <ol>
                  {#each expandedTask.attempts as attempt (attempt.attemptNumber)}
                    <li>
                      #{attempt.attemptNumber} · {attempt.outcome}{attempt.errorCode
                        ? ` · ${attempt.errorCode}`
                        : ''}{attempt.errorMessage ? ` · ${attempt.errorMessage}` : ''}
                    </li>
                  {/each}
                </ol>
              </div>
            {/if}
          </li>
        {/each}
      </ul>
      {#if tasksCursor}
        <Button
          variant="secondary"
          size="sm"
          disabled={tasksLoading}
          onclick={() => loadTasks(false)}
        >
          +
        </Button>
      {/if}
    </Card>
  {:else}
    <Card>
      <h3>{t('batch.newBatch')}</h3>
      {#if collectionsLoading}
        <p role="status">{t('batch.preparing')}</p>
      {:else}
        <fieldset class="batch-field">
          <legend class="batch-field__legend">{t('batch.collections')}</legend>
          <Checkbox
            class="batch-field__select-all"
            checked={allSelected}
            onchange={toggleSelectAll}
          >
            {t('batch.selectAll')} · {t('batch.selectedCount', { count: selectedCount })}
          </Checkbox>
          <div class="batch-field__scope-list">
            {#each collections as collection (collection.id)}
              <Checkbox
                checked={Boolean(selected[collection.id])}
                onchange={() => toggleCollection(collection.id)}
              >
                <strong>{collection.name}</strong>
                <span class="batch-field__count">{collection.items}</span>
              </Checkbox>
            {/each}
          </div>
        </fieldset>
        <fieldset class="batch-field">
          <legend class="batch-field__legend">{t('batch.operations')}</legend>
          <div class="batch-field__options">
            <Checkbox bind:checked={runOcr}>{t('batch.opOcr')}</Checkbox>
            <Checkbox bind:checked={runEmbeddings}>{t('batch.opEmbeddings')}</Checkbox>
          </div>
          <p class="batch-field__hint">{t('batch.opEmbeddingsHint')}</p>
        </fieldset>
        <div class="batch-tab__actions">
          <Button
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

    <Card>
      <h3>{t('batch.active')}</h3>
      {#if activeBatches.length === 0}
        <p>{t('batch.noWork')}</p>
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
                <span>{summary.id}</span>
                <span>{summary.state}</span>
                <span
                  >{summary.succeededUnits} ✓ · {summary.failedUnits} ✗ · {summary.activeUnits} …</span
                >
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

    <Card>
      <h3>{t('batch.history')}</h3>
      {#if historyBatches.length === 0}
        <p>{t('batch.noWork')}</p>
      {:else}
        <ul class="batch-tab__batches">
          {#each historyBatches as batch (batch.id)}
            <li class="batch-tab__batch">
              <button
                type="button"
                class="batch-tab__batch-row"
                onclick={() => openDetail(batch.id)}
              >
                <span>{batch.id}</span>
                <span>{batch.state}</span>
                <span>{batch.succeededUnits} ✓ · {batch.failedUnits} ✗</span>
              </button>
            </li>
          {/each}
        </ul>
        {#if historyCursor}
          <Button variant="secondary" size="sm" disabled={historyLoading} onclick={loadMoreHistory}>
            +
          </Button>
        {/if}
      {/if}
    </Card>
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

  /* Grouping stays a fieldset for the semantics; the browser's inset border
     and its notched legend are what looked pasted in. */
  .batch-field {
    border: 0;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .batch-field + .batch-field {
    margin-top: var(--space-5);
  }

  .batch-field__legend {
    padding: 0;
    color: var(--color-text-secondary);
    font-weight: var(--font-weight-medium);
  }

  .batch-field__scope-list {
    display: grid;
    gap: var(--space-1);
    max-height: 220px;
    overflow: auto;
    padding: var(--space-1);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    background: var(--surface-input);
  }

  .batch-field__options {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .batch-field__count {
    color: var(--color-text-secondary);
  }

  .batch-field__hint {
    margin: 0;
    color: var(--color-text-secondary);
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

  .batch-tab__tasks,
  .batch-tab__batches {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .batch-tab__task-row,
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

  .batch-tab__task-row:hover,
  .batch-tab__batch-row:hover {
    background: var(--surface-toolbar);
  }

  .batch-tab__task-row:focus-visible,
  .batch-tab__batch-row:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .batch-tab__task-error {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 24rem;
    color: var(--color-text-secondary);
  }

  .batch-tab__detail-actions,
  .batch-tab__batch-actions,
  .batch-tab__actions,
  .batch-tab__recovery-actions {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .batch-tab__actions {
    margin-top: var(--space-4);
    justify-content: flex-end;
  }

  .batch-tab__detail-head {
    display: flex;
    gap: var(--space-4);
    align-items: flex-start;
    justify-content: space-between;
  }

  .batch-tab__draft {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-4);
    padding-top: var(--space-4);
    border-top: 1px solid var(--border-subtle);
  }

  @media (prefers-reduced-motion: reduce) {
    .batch-tab__progress-bar,
    .batch-tab__task-row,
    .batch-tab__batch-row {
      transition: none;
    }
  }
</style>
