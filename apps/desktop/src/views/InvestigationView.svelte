<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import { navigation } from '$lib/navigation'
  import { getStore } from '$lib/db'
  import { locale, t, type I18nKey } from '$lib/i18n'
  import { renderMarkdown } from '$lib/markdown'
  import { getAssetPathLabel } from '$lib/item-metadata'
  import {
    researchRequest,
    researchAnswer,
    researchCancel,
    researchDecision,
    currentClarificationRound,
    researchGet,
    researchPause,
    researchResume,
    researchRevise,
    describeBackendError,
    researchSource,
    type ResearchArtifact,
    type ResearchEvent,
    type ResearchGate,
    type ResearchJobPhase,
    type ResearchJobStatus,
    type ResearchJobSummary,
    type ResearchSourcePath,
    type ResearchSourceSummary,
  } from '$lib/research'
  import { Button } from '@entropia/ui'

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

  const GATE_STATUS_LABELS: Record<ResearchGate['status'], I18nKey> = {
    pending: 'investigation.gatePending',
    approved: 'investigation.gateApproved',
    rejected: 'investigation.gateRejected',
  }

  let { jobId, title }: { jobId: string; title: string } = $props()
  let job = $state<ResearchJobSummary | null>(null)
  let events = $state<ResearchEvent[]>([])
  let artifacts = $state<ResearchArtifact[]>([])
  let gates = $state<ResearchGate[]>([])
  let sources = $state<ResearchSourceSummary[]>([])
  let loading = $state(true)
  let detailError = $state<string | null>(null)
  let actionError = $state<string | null>(null)
  let pollTimer: ReturnType<typeof setInterval> | null = null
  let refreshInFlight = false
  let refreshRequestId = 0
  let mounted = false
  let lastLoadedJobId: string | null = null
  let jobActionInFlight = $state<null | 'pause' | 'resume' | 'cancel'>(null)
  let gateActionInFlight = $state<string | null>(null)
  let clarificationDraft = $state<Record<string, string>>({})
  let answeringRound = $state(false)
  let editingArtifactId = $state<string | null>(null)
  let artifactDraft = $state('')
  let savingArtifactId = $state<string | null>(null)
  let artifactError = $state<string | null>(null)
  let sourceLoadingItemId = $state<string | null>(null)
  let expandedSourceIds = $state<string[]>([])
  let sourcePathsByItemId = $state<Record<string, ResearchSourcePath[]>>({})
  let sourceErrorsByItemId = $state<Record<string, string>>({})
  let expandedArtifactIds = $state<string[]>([])

  const statusLabel = (summary: ResearchJobSummary) =>
    summary.status === 'failed' && summary.close_reason === 'blocked'
      ? t('research.status.blocked')
      : t(STATUS_LABELS[summary.status])
  const phaseLabel = (summary: ResearchJobSummary) => t(PHASE_LABELS[summary.phase])
  const gateStatusLabel = (gate: ResearchGate) => t(GATE_STATUS_LABELS[gate.status])

  function translate(key: I18nKey, params?: Record<string, string | number>) {
    return t(key, params)
  }

  function formatBudget(value: number | null): string {
    if (value === null) return '∞'
    return Number.isInteger(value) ? String(value) : value.toFixed(2)
  }

  function formatTimestamp(timestamp: number): string {
    return new Date(timestamp * 1000).toLocaleString($currentLocale)
  }

  function uniqueEvents(list: ResearchEvent[]): ResearchEvent[] {
    const seen = new Set<string>()
    const unique: ResearchEvent[] = []
    for (const event of list) {
      if (seen.has(event.id)) continue
      seen.add(event.id)
      unique.push(event)
    }
    unique.sort((left, right) => left.timestamp - right.timestamp || left.id.localeCompare(right.id))
    return unique
  }

  function renderStructuredContent(content: unknown) {
    if (typeof content === 'string') {
      return { kind: 'markdown' as const, html: renderMarkdown(content) }
    }

    return {
      kind: 'json' as const,
      text: JSON.stringify(content ?? null, null, 2),
    }
  }

  function serializeRevisionDraft(content: unknown): string {
    if (typeof content === 'string') return content
    return JSON.stringify(content ?? null, null, 2)
  }

  function parseRevisionDraft(draft: string): unknown {
    const trimmed = draft.trim()
    if (!trimmed) return ''

    try {
      return JSON.parse(trimmed)
    } catch {
      return draft
    }
  }

  function normalizePath(path: string): string {
    return path.replace(/\\/g, '/').toLowerCase()
  }

  function sourcePathLabel(path: ResearchSourcePath): string {
    return path.page === null ? path.path : `${path.path} · ${translate('investigation.sourcePage', { page: path.page })}`
  }

  function artifactIsExpanded(id: string): boolean {
    return expandedArtifactIds.includes(id)
  }

  function toggleArtifact(id: string) {
    expandedArtifactIds = artifactIsExpanded(id)
      ? expandedArtifactIds.filter((current) => current !== id)
      : [...expandedArtifactIds, id]
  }
  function artifactLabel(artifact: ResearchArtifact): string {
    return `${artifact.kind} · ${translate('investigation.version', { version: artifact.version })}`
  }

  function resetTransientState() {
    detailError = null
    actionError = null
    jobActionInFlight = null
    gateActionInFlight = null
    editingArtifactId = null
    artifactDraft = ''
    savingArtifactId = null
    artifactError = null
    sourceLoadingItemId = null
    expandedSourceIds = []
    sourcePathsByItemId = {}
    sourceErrorsByItemId = {}
    expandedArtifactIds = []
  }

  const openRound = $derived.by(() => {
    const actual = currentClarificationRound(artifacts)
    if (!actual || Array.isArray(actual.round.answers)) return null
    return actual.round
  })

  /**
   * Responder es lo único que cierra la ronda: el motor rechaza aprobarla
   * como gate. Una pregunta puede quedar vacía y se declara en el informe;
   * la ronda entera en blanco, no.
   */
  async function submitClarification() {
    const round = openRound
    if (!job || !round || answeringRound) return

    const answers = round.questions.map(question => ({
      id: question.id,
      text: (clarificationDraft[question.id] ?? '').trim(),
    }))
    if (answers.every(answer => answer.text.length === 0)) {
      actionError = translate('investigation.clarification.empty')
      return
    }

    answeringRound = true
    actionError = null
    try {
      await researchAnswer({ job_id: job.id, answers })
      clarificationDraft = {}
      await refreshDetail({ silent: true })
    } catch (error) {
      actionError = describeBackendError(error, () =>
        translate('investigation.clarification.error')
      )
    } finally {
      answeringRound = false
    }
  }

  async function refreshDetail({ silent = false }: { silent?: boolean } = {}) {
    if (refreshInFlight) return
    refreshInFlight = true
    const requestId = ++refreshRequestId
    if (!silent && !job) loading = true

    try {
      const response = await researchGet(jobId)
      if (!mounted || requestId !== refreshRequestId) return

      job = response.job
      events = uniqueEvents(response.events)
      artifacts = [...response.artifacts].sort((left, right) => left.version - right.version || left.id.localeCompare(right.id))
      gates = response.gates
      sources = response.sources
      detailError = null
    } catch (loadError) {
      if (!mounted || requestId !== refreshRequestId) return
      detailError = describeBackendError(loadError, () => translate('investigation.loadError'))
    } finally {
      refreshInFlight = false
      if (mounted && requestId === refreshRequestId) {
        loading = false
      }
    }
  }

  async function runJobAction(action: 'pause' | 'resume' | 'cancel') {
    if (!job || jobActionInFlight) return

    jobActionInFlight = action
    actionError = null
    try {
      if (action === 'pause') await researchPause(job.id)
      else if (action === 'resume') await researchResume(job.id)
      else await researchCancel(job.id)
      await refreshDetail({ silent: true })
    } catch (error) {
      actionError = describeBackendError(error, () => translate('investigation.actionError'))
    } finally {
      jobActionInFlight = null
    }
  }

  async function resolveGate(gateId: string, approve: boolean) {
    if (!job || gateActionInFlight) return

    gateActionInFlight = gateId
    actionError = null
    try {
      await researchDecision({ job_id: job.id, gate_id: gateId, approve })
      await refreshDetail({ silent: true })
    } catch (error) {
      actionError = describeBackendError(error, () => translate('investigation.gateError'))
    } finally {
      gateActionInFlight = null
    }
  }

  function startArtifactRevision(artifact: ResearchArtifact) {
    if (savingArtifactId) return
    editingArtifactId = artifact.id
    artifactDraft = serializeRevisionDraft(artifact.content)
    artifactError = null
  }

  function cancelArtifactRevision() {
    if (savingArtifactId) return
    editingArtifactId = null
    artifactDraft = ''
    artifactError = null
  }

  async function saveArtifactRevision(artifact: ResearchArtifact) {
    if (!job || savingArtifactId) return
    if (!artifactDraft.trim()) {
      artifactError = translate('investigation.revisionEmpty')
      return
    }

    savingArtifactId = artifact.id
    artifactError = null
    try {
      await researchRevise({
        job_id: job.id,
        artifact_id: artifact.id,
        content: parseRevisionDraft(artifactDraft),
      })
      editingArtifactId = null
      artifactDraft = ''
      await refreshDetail({ silent: true })
    } catch (error) {
      artifactError = describeBackendError(error, () => translate('investigation.revisionError'))
    } finally {
      savingArtifactId = null
    }
  }

  function setSourceExpanded(itemId: string) {
    if (expandedSourceIds.includes(itemId)) return
    expandedSourceIds = [...expandedSourceIds, itemId]
  }

  async function loadSourcePaths(source: ResearchSourceSummary) {
    if (!job || sourceLoadingItemId === source.item_id || sourcePathsByItemId[source.item_id]) {
      setSourceExpanded(source.item_id)
      return
    }

    sourceLoadingItemId = source.item_id
    sourceErrorsByItemId = { ...sourceErrorsByItemId, [source.item_id]: '' }
    try {
      const response = await researchSource(job.id, source.item_id)
      if (!mounted) return
      sourcePathsByItemId = { ...sourcePathsByItemId, [source.item_id]: response.sources }
      setSourceExpanded(source.item_id)
    } catch (error) {
      if (!mounted) return
      sourceErrorsByItemId = {
        ...sourceErrorsByItemId,
        [source.item_id]: describeBackendError(error, () => translate('investigation.sourceError')),
      }
    } finally {
      if (sourceLoadingItemId === source.item_id) {
        sourceLoadingItemId = null
      }
    }
  }

  async function openSourcePath(source: ResearchSourceSummary, path: ResearchSourcePath) {
    try {
      const store = getStore()
      const item = await store.items.findById(source.item_id)
      if (!item) {
        throw new Error(translate('investigation.sourceUnavailable'))
      }

      const collection = await store.collections.findById(item.collectionId)
      if (!collection) {
        throw new Error(translate('investigation.sourceUnavailable'))
      }

      const assets = await store.assets.findByItem(item.id)
      const requestedPath = normalizePath(path.path)
      const requestedLabel = getAssetPathLabel(path.path)
      const asset =
        assets.find((candidate) => normalizePath(candidate.path) === requestedPath) ??
        assets.find((candidate) => getAssetPathLabel(candidate.path) === requestedLabel)

      if (!asset) {
        throw new Error(translate('investigation.sourceUnavailable'))
      }

      navigation.navigate({
        name: 'item',
        collectionId: item.collectionId,
        collectionName: collection.name,
        itemId: item.id,
        itemTitle: item.title,
        assetId: asset.id,
        assetLabel: getAssetPathLabel(asset.path),
      })
    } catch (error) {
      actionError = describeBackendError(error, () => translate('investigation.sourceUnavailable'))
    }
  }

  function jobSummaryLine(summary: ResearchJobSummary | null): string {
    if (!summary) return ''
    return `${statusLabel(summary)} · ${phaseLabel(summary)} · ${translate('investigation.calls', { current: summary.llm_calls, max: summary.max_llm_calls === null ? '∞' : summary.max_llm_calls })} · ${translate('investigation.budget', { current: formatBudget(summary.cost), max: formatBudget(summary.max_cost) })}`
  }

  const visibleJobTitle = $derived(job?.question ?? title)
  const WORKING_COPY: Record<ResearchJobPhase, I18nKey> = {
    coverage: 'investigation.working.coverage',
    design: 'investigation.working.design',
    plan: 'investigation.working.plan',
    execution: 'investigation.working.execution',
    verification: 'investigation.working.verification',
    clarification: 'investigation.working.clarification',
    report: 'investigation.working.report',
  }

  function reportMarkdownFrom(content: unknown): string | null {
    if (!content || typeof content !== 'object') return null
    const root = content as Record<string, unknown>
    const body =
      root.report && typeof root.report === 'object'
        ? (root.report as Record<string, unknown>)
        : root
    const title = typeof body.title === 'string' ? body.title : ''
    const sections = Array.isArray(body.sections) ? body.sections : []
    if (!title && sections.length === 0) return null
    const parts: string[] = []
    if (title) parts.push(`# ${title}`)
    for (const section of sections) {
      if (!section || typeof section !== 'object') continue
      const item = section as { title?: string; text?: string }
      if (item.title) parts.push(`## ${item.title}`)
      if (item.text) parts.push(item.text)
    }
    return parts.join('\n\n')
  }

  const workingCopy = $derived(
    job ? translate(WORKING_COPY[job.phase]) : translate('investigation.working'),
  )
  const reportMarkdown = $derived.by(() => {
    const artifact = artifacts.find((item) => item.kind === 'report' && !item.obsolete)
    return artifact ? reportMarkdownFrom(artifact.content) : null
  })
  const reportHtml = $derived(reportMarkdown ? renderMarkdown(reportMarkdown) : null)
  const canPause = $derived(Boolean(job && job.status === 'running'))
  const canResume = $derived(Boolean(job && job.status === 'paused'))
  const canCancel = $derived(
    Boolean(job && job.status !== 'done' && job.status !== 'failed'),
  )
  const blockedCoverage = $derived(
    Boolean(job && ((job.status === 'failed' && job.close_reason === 'blocked') ||
      (job.status === 'awaiting_human' && gates.some(g => g.kind === 'prospection' && g.status !== 'approved')))),
  )
  const blockedDetail = $derived(
    blockedCoverage
      ? ([...artifacts]
          .reverse()
          .find(
            (artifact) => artifact.kind === 'prospection' && !artifact.obsolete,
          )?.content as { rationale?: string; gaps?: string[] } | null | undefined) ?? null
      : null,
  )
  const blockedGaps = $derived(blockedDetail?.gaps ?? [])
  const lastBackendError = $derived(
    (() => {
      for (let index = events.length - 1; index >= 0; index -= 1) {
        const event: ResearchEvent | undefined = events[index]
        if (event?.kind === 'pause') continue
        if (event?.kind === 'research_error') {
          const message = (event.payload as { message?: string } | null)?.message
          return message?.trim() ? message : null
        }
        return null
      }
      return null
    })(),
  )
  async function continueDespiteCoverage() {
    if (!job || jobActionInFlight) return
    jobActionInFlight = 'resume'
    actionError = null
    try {
      await researchRequest({ op: 'continue_coverage', job_id: job.id })
      await refreshDetail({ silent: true })
    } catch (error) {
      actionError = describeBackendError(error, () => translate('investigation.actionError'))
    } finally {
      jobActionInFlight = null
    }
  }

  let budgetEditing = $state(false)
  let budgetCalls = $state<number | undefined>()
  let budgetCost = $state<number | undefined>()
  async function saveBudget() {
    if (!job || jobActionInFlight || !budgetCalls) return
    jobActionInFlight = 'pause'
    actionError = null
    try {
      await researchRequest({op: 'update_budget', job_id: job.id, max_llm_calls: budgetCalls, max_cost: budgetCost ?? null})
      budgetEditing = false
      await refreshDetail({silent: true})
    } catch (error) {
      actionError = describeBackendError(error, () => translate('research.invalidBudget'))
    } finally {
      jobActionInFlight = null
    }
  }

  $effect(() => {
    if (!mounted || jobId === lastLoadedJobId) return
    lastLoadedJobId = jobId
    resetTransientState()
    job = null
    events = []
    artifacts = []
    gates = []
    sources = []
    expandedArtifactIds = []
    void refreshDetail()
  })

  onMount(() => {
    mounted = true
    lastLoadedJobId = jobId
    void refreshDetail().finally(() => {
      if (!mounted || pollTimer) return
      pollTimer = setInterval(() => {
        void refreshDetail({ silent: true })
      }, 1500)
    })
  })

  onDestroy(() => {
    mounted = false
    if (pollTimer) clearInterval(pollTimer)
  })
</script>

<div class="investigation-view page-shell">
  <section class="page-header investigation-view__header" aria-labelledby="investigation-title">
    <div class="page-header__content">
      <span class="page-header__eyebrow">{$currentLocale && t('investigation.eyebrow')}</span>
      <h1 id="investigation-title">{visibleJobTitle}</h1>
      <p>{$currentLocale && t('investigation.subtitle')}</p>
    </div>

    <div class="page-toolbar investigation-view__toolbar">
      {#if canPause}
        <Button
          variant="secondary"
          size="sm"
          disabled={jobActionInFlight !== null}
          loading={jobActionInFlight === 'pause'}
          onclick={() => void runJobAction('pause')}
        >
          <span>{$currentLocale && t('investigation.pause')}</span>
        </Button>
      {/if}
      {#if canResume}
        <Button
          variant="primary"
          size="sm"
          disabled={jobActionInFlight !== null}
          loading={jobActionInFlight === 'resume'}
          onclick={() => void runJobAction('resume')}
        >
          <span>{$currentLocale && t('investigation.resume')}</span>
        </Button>
      {/if}
      {#if canCancel}
        <Button
          variant="ghost"
          size="sm"
          disabled={jobActionInFlight !== null}
          loading={jobActionInFlight === 'cancel'}
          onclick={() => void runJobAction('cancel')}
        >
          <span>{$currentLocale && t('investigation.cancel')}</span>
        </Button>
      {/if}
    </div>
  </section>

  {#if detailError}
    <p class="surface-message surface-message--error" role="alert">{detailError}</p>
  {/if}

  {#if actionError}
    <p class="surface-message surface-message--error" role="alert">{actionError}</p>
  {/if}

  <div class="investigation-chat">
    <article class="investigation-chat__message investigation-chat__message--user">
      <p>{visibleJobTitle}</p>
    </article>

    {#if job?.status === 'paused'}
      <article class="investigation-chat__message investigation-chat__message--assistant">
        <p>{lastBackendError ?? translate('investigation.pausedHint')}</p>
      </article>
    {:else if !reportHtml}
      <article class="investigation-chat__message investigation-chat__message--assistant">
        <p>{workingCopy}</p>
      </article>
    {/if}

    {#if openRound}
      <article class="investigation-chat__message investigation-chat__message--assistant">
        <h3 class="investigation-round__title">
          {$currentLocale && t('investigation.clarification.title')}
        </h3>
        <p class="investigation-round__intro">
          {$currentLocale && t('investigation.clarification.intro')}
        </p>
        <div class="investigation-round">
          {#each openRound.questions as question (question.id)}
            <label class="investigation-round__field">
              <span class="investigation-round__axis">{question.axis}</span>
              <span class="investigation-round__question">{question.text}</span>
              <textarea
                class="investigation-round__input"
                rows="2"
                bind:value={clarificationDraft[question.id]}
                disabled={answeringRound}
              ></textarea>
            </label>
          {/each}
          <Button
            variant="primary"
            size="sm"
            disabled={answeringRound}
            loading={answeringRound}
            onclick={() => void submitClarification()}
          >
            <span>
              {$currentLocale &&
                t(
                  answeringRound
                    ? 'investigation.clarification.sending'
                    : 'investigation.clarification.submit'
                )}
            </span>
          </Button>
        </div>
      </article>
    {/if}

    {#if reportHtml}
      <article class="investigation-chat__message investigation-chat__message--assistant">
        <div class="investigation-chat__report">
          <!-- markdown: renderMarkdown escapes all HTML before emitting tags -->
          <!-- eslint-disable-next-line svelte/no-at-html-tags -->
          {@html reportHtml}
        </div>
      </article>
    {/if}
  </div>
</div>

<style>
  .investigation-view {
    min-height: 100%;
  }

  .investigation-round__title {
    margin: 0 0 var(--space-1);
    font-size: var(--font-size-md, 1rem);
  }

  .investigation-round__intro {
    margin: 0 0 var(--space-3);
    color: var(--color-text-muted, inherit);
  }

  .investigation-round {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    align-items: start;
  }

  .investigation-round__field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    width: 100%;
  }

  .investigation-round__axis {
    font-size: var(--font-size-xs, 0.75rem);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--color-text-muted, inherit);
  }

  .investigation-round__question {
    font-weight: 600;
  }

  .investigation-round__input {
    width: 100%;
    resize: vertical;
    padding: var(--space-2);
    border: 1px solid var(--color-border, currentColor);
    border-radius: var(--radius-sm, 4px);
    background: var(--color-surface, transparent);
    color: inherit;
    font: inherit;
  }

  .investigation-view__toolbar {
    display: flex;
    flex-wrap: wrap;
    justify-content: end;
    gap: var(--space-2);
  }

  .investigation-chat {
    display: grid;
    gap: var(--space-3);
    max-width: 48rem;
  }

  .investigation-chat__message {
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-lg);
  }

  .investigation-chat__message--user {
    justify-self: end;
    max-width: 80%;
    background: color-mix(in srgb, var(--color-accent) 16%, var(--surface-card));
  }

  .investigation-chat__message--assistant {
    background: var(--surface-card);
    border: 1px solid var(--border-subtle);
  }

  .investigation-chat__report :global(p:last-child) {
    margin-bottom: 0;
  }


  @media (max-width: 820px) {
    .investigation-view__toolbar {
      justify-content: start;
    }
  }
</style>
