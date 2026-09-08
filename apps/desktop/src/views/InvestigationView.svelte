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
    type ResearchCitation,
    type ResearchReportContent,
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
    // El motor arma el informe completo —cobertura, fragmentos citados con su
    // `[n]`, «Fuentes citadas», perfil y encuadre— y lo deja en el artefacto.
    // Rearmarlo acá desde `sections[].text` pierde todo eso: el frontend no
    // reinterpreta la presentación ni duplica el motor.
    if (typeof root.markdown === 'string' && root.markdown.trim().length > 0) {
      return root.markdown
    }
    // Respaldo para artefactos anteriores, que no lo traen.
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

  /**
   * El artefacto entero, para pintarlo por partes.
   *
   * El motor decide qué se cita —numeración, pasajes y referencias vienen
   * armados—; acá solo se decide cómo se ve. `markdown` sigue siendo el
   * documento canónico y es lo que se copia o exporta.
   */
  const reportContent = $derived.by(() => {
    const artifact = artifacts.find((item) => item.kind === 'report' && !item.obsolete)
    const content = artifact?.content
    if (!content || typeof content !== 'object') return null
    return content as ResearchReportContent
  })

  const reportSections = $derived(reportContent?.report?.sections ?? [])
  const reportReferences = $derived(reportContent?.report?.references ?? [])
  const reportCoverage = $derived(reportContent?.coverage?.collections ?? [])
  const coverageWarning = $derived(
    reportContent?.coverage_warning?.sufficient === false
      ? reportContent.coverage_warning
      : null,
  )
  const reportLimitations = $derived.by(() => {
    const limitaciones = (reportContent?.archive_limitations ?? [])
      .map((l) => (l.reason ? `${l.text} (${l.reason})` : l.text))
      .filter((t): t is string => Boolean(t && t.trim()))
    const degradaciones = (reportContent?.role_warnings ?? [])
      .filter((w) => w.error?.trim())
      .map((w) => {
        const veces = (w.times ?? 1) > 1 ? `, ${w.times} veces` : ''
        return `Degradación del pipeline: ${w.error} (${w.role ?? 'rol desconocido'}${veces}).`
      })
    // Las limitaciones llegan por lote y se repiten: el investigador no
    // necesita leer tres veces la misma ausencia.
    return [...new Set([...limitaciones, ...degradaciones])]
  })

  const totalCoverage = $derived.by(() => {
    const items = reportCoverage.reduce((total, c) => total + (c.items ?? 0), 0)
    const conChunks = reportCoverage.reduce((total, c) => total + (c.items_with_chunks ?? 0), 0)
    return { items, conChunks, sinProcesar: items - conChunks }
  })

  /** Referencia legible de una cita: colección · título · fecha. */
  function citationLabel(cita: ResearchCitation): string {
    return [cita.collection, cita.title, cita.date].filter(Boolean).join(' · ')
  }

  function citationRange(cita: ResearchCitation): string {
    return `chars ${cita.start}–${cita.end}`
  }

  /** Abre el asset de una cita reusando la máquina de fuentes ya existente. */
  function openCitation(cita: ResearchCitation) {
    const fuente = sources.find((s) => s.title === cita.title)
    if (fuente) void loadSourcePaths(fuente)
  }
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

    {#if reportContent && reportSections.length > 0}
      <article class="investigation-chat__message investigation-chat__message--assistant">
        <div class="investigation-chat__report">
          {#if reportContent.report?.title}
            <h2 class="report__title">{reportContent.report.title}</h2>
          {/if}

          {#if reportCoverage.length > 0}
            <section class="report__coverage">
              <h3 class="report__label">{$currentLocale && t('investigation.report.coverage')}</h3>
              <table class="report__table">
                <thead>
                  <tr>
                    <th scope="col">{$currentLocale && t('investigation.report.collection')}</th>
                    <th scope="col">{$currentLocale && t('investigation.report.items')}</th>
                    <th scope="col">{$currentLocale && t('investigation.report.withChunks')}</th>
                    <th scope="col">{$currentLocale && t('investigation.report.unprocessed')}</th>
                  </tr>
                </thead>
                <tbody>
                  {#each reportCoverage as coleccion (coleccion.id)}
                    <tr>
                      <th scope="row">{coleccion.name}</th>
                      <td>{coleccion.items}</td>
                      <td>{coleccion.items_with_chunks}</td>
                      <td>{coleccion.items - coleccion.items_with_chunks}</td>
                    </tr>
                  {/each}
                  <tr class="report__table-total">
                    <th scope="row">{$currentLocale && t('investigation.report.total')}</th>
                    <td>{totalCoverage.items}</td>
                    <td>{totalCoverage.conChunks}</td>
                    <td>{totalCoverage.sinProcesar}</td>
                  </tr>
                </tbody>
              </table>
              {#if reportContent.profile?.bias}
                <p class="report__bias">
                  <strong>{reportContent.profile.name}</strong> — {reportContent.profile.bias}
                </p>
              {/if}
            </section>
          {/if}

          {#if coverageWarning}
            <aside class="report__warning" role="note">
              <p>{coverageWarning.rationale}</p>
              {#if coverageWarning.gaps?.length}
                <ul>
                  {#each coverageWarning.gaps as gap (gap)}<li>{gap}</li>{/each}
                </ul>
              {/if}
            </aside>
          {/if}

          {#if reportContent.clarification?.questions?.length}
            <section class="report__framing">
              <h3 class="report__label">{$currentLocale && t('investigation.report.framing')}</h3>
              <dl class="report__framing-list">
                {#each reportContent.clarification.questions as pregunta (pregunta.id)}
                  {@const respuesta = reportContent.clarification?.answers?.find(
                    (a) => a.id === pregunta.id,
                  )?.text}
                  <dt>{pregunta.text}</dt>
                  <dd class:report__framing-empty={!respuesta?.trim()}>
                    {respuesta?.trim() || translate('investigation.report.unanswered')}
                  </dd>
                {/each}
              </dl>
            </section>
          {/if}

          {#each reportSections as seccion, index (`${seccion.title}-${index}`)}
            <section class="report__section">
              {#if seccion.title}<h3>{seccion.title}</h3>{/if}
              <!-- markdown: renderMarkdown escapes all HTML before emitting tags -->
              <!-- eslint-disable-next-line svelte/no-at-html-tags -->
              {@html renderMarkdown(seccion.text)}
              {#if seccion.quotes?.length}
                <ul class="report__quotes">
                  {#each seccion.quotes as cita (`${cita.n}-${cita.start}`)}
                    <li>
                      <button
                        type="button"
                        class="report__quote"
                        onclick={() => openCitation(cita)}
                        title={$currentLocale && t('investigation.report.openSource')}
                      >
                        <span class="report__quote-text"
                          >{cita.text}{cita.truncated ? ' […]' : ''}</span
                        >
                        <span class="report__quote-meta">
                          <span class="report__quote-ref">[{cita.n}]</span>
                          <span class="report__quote-source">{citationLabel(cita)}</span>
                          <span class="report__quote-range">{citationRange(cita)}</span>
                        </span>
                      </button>
                    </li>
                  {/each}
                </ul>
              {/if}
            </section>
          {/each}

          {#if reportLimitations.length > 0}
            <section class="report__section">
              <h3>{$currentLocale && t('investigation.report.limitations')}</h3>
              <ul>
                {#each reportLimitations as limitacion (limitacion)}<li>{limitacion}</li>{/each}
              </ul>
            </section>
          {/if}

          {#if reportReferences.length > 0}
            <section class="report__sources">
              <h3 class="report__label">{$currentLocale && t('investigation.report.cited')}</h3>
              <ul class="report__sources-list">
                {#each reportReferences as referencia (referencia.n)}
                  <li>
                    <button
                      type="button"
                      class="report__source"
                      onclick={() => openCitation(referencia)}
                      title={$currentLocale && t('investigation.report.openSource')}
                    >
                      <span class="report__source-heading">
                        <span class="report__quote-ref">[{referencia.n}]</span>
                        <span class="report__source-name">{citationLabel(referencia)}</span>
                      </span>
                      <span class="report__quote-range">{citationRange(referencia)}</span>
                    </button>
                  </li>
                {/each}
              </ul>
            </section>
          {/if}
        </div>
      </article>
    {:else if reportHtml}
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

  /* El markdown de las secciones entra por {@html}: sin reglas propias, los
     enlaces y el código caen a los colores por defecto del navegador —azul y
     violeta— que no pertenecen a la paleta del tema. */
  .investigation-chat__report :global(a) {
    color: var(--color-text-primary);
    text-decoration: underline;
    text-underline-offset: 2px;
    text-decoration-color: var(--border-panel);
  }

  .investigation-chat__report :global(a:hover) {
    text-decoration-color: var(--color-text-primary);
  }

  .investigation-chat__report :global(a:focus-visible) {
    outline: none;
    border-radius: var(--radius-xs);
    box-shadow: var(--focus-ring);
  }

  .investigation-chat__report :global(code) {
    padding: 0.1em 0.35em;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-xs);
    background: var(--surface-toolbar);
    font-family: var(--font-mono);
    font-size: 0.9em;
  }

  .investigation-chat__report :global(strong) {
    color: var(--color-text-primary);
    font-weight: 600;
  }

  /* La advertencia usa el ámbar del sistema, pero como acento fino, no como
     bloque de color: el tema es monocromático y el aviso no es una alarma. */
  .report__warning {
    color: var(--color-text-secondary);
  }

  .report__title {
    margin: 0 0 var(--space-4);
    font-size: var(--font-size-lg, 1.25rem);
    line-height: 1.3;
  }

  .report__label {
    margin: 0 0 var(--space-2);
    font-size: var(--font-size-xs, 0.75rem);
    font-weight: 500;
    letter-spacing: 0.075em;
    text-transform: uppercase;
    color: var(--color-text-muted, inherit);
  }

  .report__coverage,
  .report__framing,
  .report__sources {
    margin-bottom: var(--space-5, 1.5rem);
  }

  .report__table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--font-size-sm, 0.875rem);
  }

  .report__table th,
  .report__table td {
    padding: var(--space-1) var(--space-2);
    border-bottom: 1px solid var(--border-subtle, currentColor);
    text-align: right;
  }

  .report__table th[scope='row'],
  .report__table th[scope='col']:first-child {
    text-align: left;
    font-weight: 500;
  }

  .report__table-total th,
  .report__table-total td {
    font-weight: 600;
    border-bottom: none;
  }

  .report__bias {
    margin: var(--space-2) 0 0;
    font-size: var(--font-size-xs, 0.75rem);
    color: var(--color-text-muted, inherit);
  }

  /* La advertencia de cobertura no es decorativa: dice qué no se pudo leer. */
  .report__warning {
    margin-bottom: var(--space-5, 1.5rem);
    padding: var(--space-3);
    border-left: 3px solid var(--color-warning, currentColor);
    border-radius: var(--radius-sm, 4px);
    background: var(--surface-toolbar, transparent);
    font-size: var(--font-size-sm, 0.875rem);
  }

  .report__warning p,
  .report__warning ul {
    margin: 0;
  }

  .report__warning ul {
    margin-top: var(--space-1);
    padding-left: var(--space-4);
  }

  .report__framing-list {
    margin: 0;
    font-size: var(--font-size-sm, 0.875rem);
  }

  .report__framing-list dt {
    font-weight: 600;
    margin-top: var(--space-2);
  }

  .report__framing-list dd {
    margin: var(--space-1) 0 0;
    padding-left: var(--space-3);
    border-left: 2px solid var(--border-subtle, currentColor);
    color: var(--color-text-muted, inherit);
    white-space: pre-line;
  }

  .report__framing-empty {
    font-style: italic;
  }

  .report__section {
    margin-bottom: var(--space-5, 1.5rem);
  }

  .report__quotes,
  .report__sources-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: var(--space-3) 0 0;
    padding: 0;
    list-style: none;
  }

  .report__quote,
  .report__source {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    width: 100%;
    min-width: 0;
    box-sizing: border-box;
    padding: var(--space-2) var(--space-3);
    border: 1px solid transparent;
    border-left: 3px solid var(--border-subtle, currentColor);
    border-radius: var(--radius-sm, 4px);
    background: none;
    cursor: pointer;
    text-align: left;
    font: inherit;
    color: inherit;
    transition:
      background-color 120ms ease,
      border-color 120ms ease;
  }

  .report__quote:hover,
  .report__source:hover {
    background: var(--surface-toolbar, transparent);
    border-color: var(--border-subtle, currentColor);
  }

  .report__quote:focus-visible,
  .report__source:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring, 0 0 0 2px currentColor);
  }

  /* El fragmento es literal: se muestra tal cual, con sus saltos de línea. */
  .report__quote-text {
    white-space: pre-line;
    font-size: var(--font-size-sm, 0.875rem);
  }

  .report__quote-meta,
  .report__source-heading {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-2);
    font-size: var(--font-size-xs, 0.75rem);
    color: var(--color-text-muted, inherit);
  }

  .report__quote-ref {
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }

  .report__quote-range {
    font-family: var(--font-mono, monospace);
    opacity: 0.75;
  }

  .report__source-name {
    min-width: 0;
    overflow-wrap: anywhere;
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
