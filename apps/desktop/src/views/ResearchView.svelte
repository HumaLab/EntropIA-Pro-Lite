<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import { navigation } from '$lib/navigation'
  import {
    locale,
    t,
    type I18nKey,
  } from '$lib/i18n'
  import {
    describeBackendError,
    researchCreate,
    researchList,
    takeResearchHandoff,
    type ResearchCollectionSummary,
    type ResearchHandoffDraft,
    type ResearchJobPhase,
    type ResearchJobStatus,
    type ResearchJobSummary,
  } from '$lib/research'

  import { renderMarkdown } from '$lib/markdown'
  import { ActionIcon, Button, Card, Input, Panel } from '@entropia/ui'

  const currentLocale = locale

  const STATUS_LABELS: Record<ResearchJobStatus, I18nKey> = {
    planned: 'research.status.planned',
    running: 'research.status.running',
    paused: 'research.status.paused',
    awaiting_human: 'research.status.awaitingHuman',
    done: 'research.status.done',
    failed: 'research.status.failed',
  }

  const PHASE_LABELS: Record<ResearchJobPhase, I18nKey> = {
    coverage: 'research.phase.coverage',
    design: 'research.phase.design',
    plan: 'research.phase.plan',
    execution: 'research.phase.execution',
    verification: 'research.phase.verification',
    clarification: 'research.phase.clarification',
    report: 'research.phase.report',
  }

  let jobs = $state<ResearchJobSummary[]>([])
  let collections = $state<ResearchCollectionSummary[]>([])
  let loading = $state(true)
  let error = $state<string | null>(null)
  let submitError = $state<string | null>(null)
  let creating = $state(false)
  let refreshRequestId = 0
  let pollingTimer: ReturnType<typeof setInterval> | null = null
  let mounted = false

  let question = $state('')
  let project = $state('investigación')
  let selectedCollectionIds = $state<string[]>([])
  let maxLlmCalls = $state('80')
  let maxCost = $state('')
  let handoff = $state<ResearchHandoffDraft | null>(null)

  const collectionCountLabel = $derived(
    collections.length === 1
      ? t('research.collectionsOne', { count: collections.length })
      : t('research.collectionsMany', { count: collections.length }),
  )

  const selectedCollectionCountLabel = $derived(
    selectedCollectionIds.length === 1
      ? t('research.selectedCollectionsOne', { count: selectedCollectionIds.length })
      : t('research.selectedCollectionsMany', { count: selectedCollectionIds.length }),
  )

  const contextSummaryLabel = $derived.by(() => {
    if (!handoff?.context?.length) return ''
    const messageCount = handoff.context.length
    const sourceCount = handoff.context.reduce(
      (total, message) => total + (message.sources?.length ?? 0),
      0,
    )
    return t('research.contextSummary', { messages: messageCount, sources: sourceCount })
  })

  const canSubmit = $derived.by(() => {
    const trimmedQuestion = question.trim()
    const parsedCalls = Number.parseInt(maxLlmCalls, 10)
    return (
      !creating &&
      trimmedQuestion.length > 0 &&
      selectedCollectionIds.length > 0 &&
      Number.isInteger(parsedCalls) &&
      parsedCalls > 0
    )
  })

  function translate(key: I18nKey, params?: Record<string, string | number>) {
    return t(key, params)
  }

  function formatBudget(value: number | null): string {
    if (value === null) return '∞'
    return Number.isInteger(value) ? String(value) : value.toFixed(2)
  }

  function formatCount(value: number): string {
    return Number.isInteger(value) ? String(value) : value.toFixed(2)
  }

  function normalizeSelection(ids: string[]) {
    selectedCollectionIds = [...new Set(ids)]
  }

  function toggleCollection(id: string, checked: boolean) {
    if (checked) {
      normalizeSelection([...selectedCollectionIds, id])
      return
    }

    normalizeSelection(selectedCollectionIds.filter((collectionId) => collectionId !== id))
  }

  function selectAllCollections() {
    normalizeSelection(collections.map((collection) => collection.id))
  }

  function clearCollections() {
    selectedCollectionIds = []
  }

  function statusLabel(job: ResearchJobSummary): string {
    return translate(job.status === 'failed' && job.close_reason === 'blocked'
      ? 'research.status.blocked'
      : STATUS_LABELS[job.status])
  }

  function phaseLabel(job: ResearchJobSummary): string {
    return translate(PHASE_LABELS[job.phase])
  }

  function budgetLabel(job: ResearchJobSummary): string {
    const current = formatBudget(job.cost)
    const maximum = formatBudget(job.max_cost)
    return `${current} / ${maximum}`
  }

  function callsLabel(job: ResearchJobSummary): string {
    const current = formatCount(job.llm_calls)
    const maximum = job.max_llm_calls === null ? '∞' : formatCount(job.max_llm_calls)
    return `${current} / ${maximum}`
  }

  function collectionStats(collection: ResearchCollectionSummary): string {
    return translate('research.collectionStats', {
      items: collection.items,
      itemsWithChunks: collection.items_with_chunks,
      chunks: collection.chunks,
    })
  }

  function applyHandoff(draft: ResearchHandoffDraft | null) {
    if (!draft) return
    handoff = draft
    question = draft.question
    project = draft.project
  }

  async function refreshJobs({ silent = false }: { silent?: boolean } = {}) {
    const requestId = ++refreshRequestId
    if (!silent) loading = true

    try {
      const response = await researchList()
      if (!mounted || requestId !== refreshRequestId) return
      jobs = response.jobs
      collections = response.collections
      if (selectedCollectionIds.length === 0) {
        selectedCollectionIds = response.collections
          .filter((collection) => collection.chunks > 0)
          .map((collection) => collection.id)
      }
      error = null
    } catch (loadError) {
      if (!mounted || requestId !== refreshRequestId) return
      error = describeBackendError(loadError, () => translate('research.loadError'))
    } finally {
      if (mounted && requestId === refreshRequestId) {
        loading = false
      }
    }
  }

  async function handleSubmit() {
    if (!canSubmit) {
      submitError = translate('research.formInvalid')
      return
    }

    const parsedCalls = Number.parseInt(maxLlmCalls, 10)
    const parsedCost = maxCost.trim() ? Number(maxCost) : null
    if (!Number.isInteger(parsedCalls) || parsedCalls <= 0) {
      submitError = translate('research.invalidCalls')
      return
    }
    if (parsedCost !== null && !Number.isFinite(parsedCost)) {
      submitError = translate('research.invalidBudget')
      return
    }

    creating = true
    submitError = null
    try {
      const created = await researchCreate({
        question: question.trim(),
        project: project.trim() || 'investigación',
        collection_ids: [...selectedCollectionIds],
        max_llm_calls: parsedCalls,
        max_cost: parsedCost,
        context: handoff?.context ?? null,
      })

      await refreshJobs({ silent: true })
      navigation.navigate({
        name: 'investigation',
        jobId: created.id,
        title: created.question,
      })
    } catch (createError) {
      submitError = describeBackendError(createError, () => translate('research.startError'))
    } finally {
      creating = false
    }
  }

  onMount(() => {
    mounted = true
    applyHandoff(takeResearchHandoff())
    void refreshJobs()
    pollingTimer = setInterval(() => {
      void refreshJobs({ silent: true })
    }, 1500)
  })

  onDestroy(() => {
    mounted = false
    if (pollingTimer) clearInterval(pollingTimer)
  })
</script>

<div class="research-view page-shell">
  <section class="page-header research-view__header" aria-labelledby="research-title">
    <div class="page-header__content">
      <span class="page-header__eyebrow">{$currentLocale && t('research.eyebrow')}</span>
      <h1 id="research-title">{$currentLocale && t('research.title')}</h1>
      <p>{$currentLocale && t('research.subtitle')}</p>
      <span class="page-header__meta">{collectionCountLabel}</span>
    </div>
  </section>

  {#if error}
    <p class="surface-message surface-message--error" role="alert">{error}</p>
  {/if}

  <div class="research-view__layout">
    <section class="research-view__jobs" aria-labelledby="research-jobs-title">
      <div class="research-view__section-header">
        <h2 id="research-jobs-title">{$currentLocale && t('research.jobsTitle')}</h2>
        <span class="research-view__section-meta">{jobs.length}</span>
      </div>

      {#if loading && jobs.length === 0}
        <p class="surface-message surface-message--center">{$currentLocale && t('research.loading')}</p>
      {:else if jobs.length === 0}
        <div class="surface-message surface-message--center">
          <p>{$currentLocale && t('research.empty')}</p>
        </div>
      {:else}
        <div class="research-view__job-list">
          {#each jobs as job (job.id)}
            <Panel variant="raised" padding="md" class="research-job-card">
              <div class="research-job-card__header">
                <div class="research-job-card__copy">
                  <h3 class="research-job-card__question">{job.question}</h3>
                  <p class="research-job-card__meta">
                    <span>{statusLabel(job)}</span>
                  </p>
                </div>
                <Button
                  variant="secondary"
                  size="sm"
                  onclick={() =>
                    navigation.navigate({
                      name: 'investigation',
                      jobId: job.id,
                      title: job.question,
                    })}
                >
                  <ActionIcon name="external-link" size={14} />
                  <span>{$currentLocale && t('research.openDetail')}</span>
                </Button>

              </div>
            </Panel>
          {/each}
        </div>
      {/if}
    </section>

    <section class="research-view__form-column" aria-labelledby="research-form-title">
      <Card>
        <form class="research-form" onsubmit={(event) => { event.preventDefault(); void handleSubmit() }}>
          <div class="research-form__copy">
            <h2 id="research-form-title">{$currentLocale && t('research.formTitle')}</h2>
            <p>{$currentLocale && t('research.formDescription')}</p>
          </div>
          <label class="research-form__field">
            <span>{$currentLocale && t('research.questionLabel')}</span>
            <textarea
              class="research-form__textarea"
              rows="4"
              value={question}
              placeholder={$currentLocale && t('research.questionPlaceholder')}
              oninput={(event) => {
                question = event.currentTarget.value
                submitError = null
              }}
            ></textarea>
          </label>

          <details class="research-form__advanced">
            <summary>{$currentLocale && t('research.collectionsLabel')}</summary>
            <fieldset class="research-form__scope">
              <div class="research-form__scope-header">
                <span class="research-form__scope-count">{selectedCollectionCountLabel}</span>
                <div class="research-form__scope-actions">
                  <Button variant="ghost" size="sm" onclick={selectAllCollections}>
                    {$currentLocale && t('research.selectAll')}
                  </Button>
                </div>
              </div>
              {#if collections.length === 0}
                <p class="research-form__hint research-form__hint--empty">
                  {$currentLocale && t('research.noCollections')}
                </p>
              {:else}
                <div class="research-form__scope-list">
                  {#each collections as collection (collection.id)}
                    <label class="research-form__scope-option">
                      <input
                        type="checkbox"
                        checked={selectedCollectionIds.includes(collection.id)}
                        onchange={(event) => {
                          toggleCollection(collection.id, event.currentTarget.checked)
                          submitError = null
                        }}
                      />
                      <span>
                        <strong>{collection.name}</strong>
                      </span>
                    </label>
                  {/each}
                </div>
              {/if}
            </fieldset>
          </details>

          {#if handoff}
            <Card padding="sm">
              <div class="research-handoff">
                <div class="research-handoff__copy">
                  <h3>{$currentLocale && t('research.handoffTitle')}</h3>
                  <p>{$currentLocale && t('research.handoffDescription')}</p>
                  {#if contextSummaryLabel}
                    <p class="research-handoff__summary">{contextSummaryLabel}</p>
                  {/if}
                </div>
                {#if handoff.context && handoff.context.length > 0}
                  <details class="research-handoff__details">
                    <summary>{$currentLocale && t('research.handoffPreview')}</summary>
                    <ul class="research-handoff__messages">
                      {#each handoff.context as message, index (`${message.role}-${index}`)}
                        <li class="research-handoff__message">
                          <span class="research-handoff__role">{message.role}</span>
                          <div class="research-handoff__body">
                            <!-- markdown: renderMarkdown escapes all HTML before emitting tags -->
                            <!-- eslint-disable-next-line svelte/no-at-html-tags -->
                            {@html renderMarkdown(message.content)}
                          </div>
                          {#if message.sources && message.sources.length > 0}
                            <small class="research-handoff__sources">
                              {$currentLocale && t('research.handoffSources', { count: message.sources.length })}
                            </small>
                          {/if}
                        </li>
                      {/each}
                    </ul>
                  </details>
                {/if}
              </div>
            </Card>
          {/if}

          {#if submitError}
            <p class="surface-message surface-message--error" role="alert">{submitError}</p>
          {/if}

          <div class="research-form__actions">
            <Button variant="primary" type="submit" loading={creating} disabled={!canSubmit}>
              <ActionIcon name="message-circle-plus" size={16} />
              <span>{creating ? ($currentLocale && t('research.starting')) : ($currentLocale && t('research.start'))}</span>
            </Button>
          </div>
        </form>
      </Card>
    </section>
  </div>
</div>

<style>
  .research-view {
    min-height: 100%;
  }

  .research-view__layout {
    display: grid;
    grid-template-columns: minmax(0, 1.2fr) minmax(360px, 0.8fr);
    gap: var(--space-4);
    align-items: start;
  }

  .research-view__jobs,
  .research-view__form-column {
    min-width: 0;
  }

  .research-view__section-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    margin-bottom: var(--space-3);
  }

  .research-view__section-meta,
  .page-header__meta {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .research-view__job-list {
    display: grid;
    gap: var(--space-3);
  }


  .research-job-card__header {
    display: flex;
    align-items: start;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .research-job-card__copy {
    display: grid;
    gap: var(--space-1);
  }

  .research-job-card__question {
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
    line-height: var(--line-height-tight);
  }

  .research-job-card__meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .research-job-card__stats {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: var(--space-1) var(--space-2);
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .research-job-card__stats strong {
    color: var(--color-text-primary);
    font-weight: var(--font-weight-medium);
  }

  .research-form {
    display: grid;
    gap: var(--space-4);
  }

  .research-form__copy {
    display: grid;
    gap: var(--space-1);
  }

  .research-form__copy p {
    color: var(--color-text-secondary);
  }

  .research-form__field,
  .research-form__scope {
    display: grid;
    gap: var(--space-2);
  }

  .research-form__field > span,
  .research-form__scope legend {
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
  }

  .research-form__hint {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .research-form__hint--empty {
    font-style: italic;
  }

  .research-form__scope-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .research-form__scope-actions {
    display: inline-flex;
    gap: var(--space-2);
    flex-wrap: wrap;
    justify-content: end;
  }

  .research-form__scope-list {
    display: grid;
    gap: var(--space-2);
    max-height: 220px;
    overflow: auto;
    padding-right: var(--space-1);
  }

  .research-form__scope-option {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: var(--space-3);
    align-items: start;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    background: var(--surface-card);
  }

  .research-form__scope-option input {
    margin-top: 3px;
  }

  .research-form__scope-option strong {
    display: block;
    font-weight: var(--font-weight-medium);
  }

  .research-form__scope-option small {
    color: var(--color-text-muted);
    display: block;
    margin-top: var(--space-1);
  }

  .research-form__budget-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-3);
  }

  .research-form__textarea,
  .research-form__number {
    width: 100%;
    box-sizing: border-box;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-input);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font: inherit;
  }

  .research-form__textarea {
    resize: vertical;
    min-height: 96px;
  }

  .research-form__textarea:focus,
  .research-form__number:focus {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
  }

  .research-form__actions {
    display: flex;
    justify-content: end;
  }

  .research-form__actions :global(.btn) {
    min-width: 180px;
  }

  .research-handoff {
    display: grid;
    gap: var(--space-3);
  }

  .research-handoff__copy {
    display: grid;
    gap: var(--space-1);
  }

  .research-handoff__copy p {
    color: var(--color-text-secondary);
  }

  .research-handoff__summary {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .research-handoff__details {
    display: grid;
    gap: var(--space-2);
  }

  .research-handoff__details summary {
    cursor: pointer;
    font-weight: var(--font-weight-medium);
  }

  .research-handoff__messages {
    display: grid;
    gap: var(--space-2);
    padding-top: var(--space-2);
  }

  .research-handoff__message {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    background: var(--surface-card);
  }

  .research-handoff__role {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }

  .research-handoff__body :global(p:last-child) {
    margin-bottom: 0;
  }

  .research-handoff__body :global(p) {
    margin-bottom: var(--space-2);
  }

  .research-handoff__sources {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  @media (max-width: 980px) {
    .research-view__layout {
      grid-template-columns: 1fr;
    }
  }

  @media (max-width: 640px) {
    .research-form__budget-grid {
      grid-template-columns: 1fr;
    }

    .research-job-card__header {
      flex-direction: column;
    }

    .research-form__scope-header {
      flex-direction: column;
      align-items: start;
    }

    .research-form__actions {
      justify-content: stretch;
    }

    .research-form__actions :global(.btn) {
      width: 100%;
      min-width: 0;
    }
  }

  .research-form__advanced {
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    padding: var(--space-3);
  }

  .research-form__advanced summary {
    cursor: pointer;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }
</style>
