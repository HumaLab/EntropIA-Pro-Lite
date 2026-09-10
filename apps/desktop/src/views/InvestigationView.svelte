<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import { navigation } from '$lib/navigation'
  import { getStore } from '$lib/db'
  import { locale, t, type I18nKey } from '$lib/i18n'
  import { renderMarkdown } from '$lib/markdown'
  import { getAssetPathLabel } from '$lib/item-metadata'
  import { classifyFileType, getAssetUrl } from '$lib/file-import'
  import {
    researchRequest,
    researchAnswer,
    researchCancel,
    currentClarificationRound,
    type ResearchCitation,
    type ResearchReportContent,
    researchGet,
    researchPause,
    researchResume,
    describeBackendError,
    researchSource,
    type ResearchArtifact,
    type ResearchEvent,
    type ResearchJobPhase,
    type ResearchJobSummary,
    type ResearchSourcePath,
    type ResearchSourceSummary,
  } from '$lib/research'
  import { Button } from '@entropia/ui'

  const currentLocale = locale

  let { jobId, title }: { jobId: string; title: string } = $props()
  let job = $state<ResearchJobSummary | null>(null)
  let events = $state<ResearchEvent[]>([])
  let artifacts = $state<ResearchArtifact[]>([])
  let sources = $state<ResearchSourceSummary[]>([])
  let detailError = $state<string | null>(null)
  let actionError = $state<string | null>(null)
  let pollTimer: ReturnType<typeof setInterval> | null = null
  let refreshInFlight = false
  let refreshRequestId = 0
  let mounted = false
  let lastLoadedJobId: string | null = null
  let jobActionInFlight = $state<null | 'pause' | 'resume' | 'cancel' | 'budget'>(null)
  let clarificationDraft = $state<Record<string, string>>({})
  let answeringRound = $state(false)
  let sourceLoadingItemId = $state<string | null>(null)
  let expandedSourceIds = $state<string[]>([])
  let sourcePathsByItemId = $state<Record<string, ResearchSourcePath[]>>({})
  let sourceErrorsByItemId = $state<Record<string, string>>({})
  /** Cita abierta en el panel de la derecha. */
  let selectedCitation = $state<ResearchCitation | null>(null)
  /** Vista previa del documento citado, cuando el asset se puede mostrar. */
  let preview = $state<{ url: string; kind: 'image' | 'pdf'; label: string } | null>(null)
  let previewFailed = $state(false)

  function translate(key: I18nKey, params?: Record<string, string | number>) {
    return t(key, params)
  }

  function formatBudget(value: number | null): string {
    if (value === null) return '∞'
    return Number.isInteger(value) ? String(value) : value.toFixed(2)
  }

  function uniqueEvents(list: ResearchEvent[]): ResearchEvent[] {
    const seen = new Set<string>()
    const unique: ResearchEvent[] = []
    for (const event of list) {
      if (seen.has(event.id)) continue
      seen.add(event.id)
      unique.push(event)
    }
    unique.sort(
      (left, right) => left.timestamp - right.timestamp || left.id.localeCompare(right.id)
    )
    return unique
  }

  function normalizePath(path: string): string {
    return path.replace(/\\/g, '/').toLowerCase()
  }

  function resetTransientState() {
    detailError = null
    actionError = null
    jobActionInFlight = null
    sourceLoadingItemId = null
    expandedSourceIds = []
    sourcePathsByItemId = {}
    sourceErrorsByItemId = {}
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

    const answers = round.questions.map((question) => ({
      id: question.id,
      text: (clarificationDraft[question.id] ?? '').trim(),
    }))
    if (answers.every((answer) => answer.text.length === 0)) {
      actionError = translate('investigation.clarification.empty')
      return
    }

    answeringRound = true
    actionError = null
    try {
      await researchAnswer({ job_id: job.id, answers })
      clarificationDraft = {}
      await refreshDetail()
    } catch (error) {
      actionError = describeBackendError(error, () =>
        translate('investigation.clarification.error')
      )
    } finally {
      answeringRound = false
    }
  }

  async function refreshDetail() {
    if (refreshInFlight) return
    refreshInFlight = true
    const requestId = ++refreshRequestId

    try {
      const response = await researchGet(jobId)
      if (!mounted || requestId !== refreshRequestId) return

      job = response.job
      events = uniqueEvents(response.events)
      artifacts = [...response.artifacts].sort(
        (left, right) => left.version - right.version || left.id.localeCompare(right.id)
      )
      sources = response.sources
      detailError = null
    } catch (loadError) {
      if (!mounted || requestId !== refreshRequestId) return
      detailError = describeBackendError(loadError, () => translate('investigation.loadError'))
    } finally {
      refreshInFlight = false
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
      await refreshDetail()
    } catch (error) {
      actionError = describeBackendError(error, () => translate('investigation.actionError'))
    } finally {
      jobActionInFlight = null
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
      const primera = response.sources[0]
      if (primera && selectedCitation) {
        void loadPreview(source.item_id, primera, selectedCitation.title)
      }
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
      // Both sides read the same column of the same database for the same item,
      // so the exact match holds by construction. A filename fallback could only
      // substitute the wrong asset when a name repeats across pages or versions.
      const requestedPath = normalizePath(path.path)
      const asset = assets.find((candidate) => normalizePath(candidate.path) === requestedPath)

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
    job ? translate(WORKING_COPY[job.phase]) : translate('investigation.working')
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
    reportContent?.coverage_warning?.sufficient === false ? reportContent.coverage_warning : null
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

  /**
   * Abre la cita en el panel de la derecha y pide las rutas de su asset.
   *
   * La cita trae su `item_id`, así que la fuente se resuelve por identidad y
   * no por coincidencia de título.
   */
  /**
   * Item de la cita.
   *
   * Los informes anteriores a que la cita llevara su `item_id` no lo traen: se
   * resuelve por título contra las fuentes del job. Un informe viejo no tiene
   * por qué quedar sin fuentes navegables.
   */
  function itemIdDe(cita: ResearchCitation): string {
    if (cita.item_id) return cita.item_id
    return sources.find((source) => source.title === cita.title)?.item_id ?? ''
  }

  function openCitation(cita: ResearchCitation) {
    selectedCitation = cita
    actionError = null
    preview = null
    previewFailed = false
    const itemId = itemIdDe(cita)
    if (!itemId) {
      sourceErrorsByItemId = {
        ...sourceErrorsByItemId,
        [cita.evidence_id]: translate('investigation.source.unresolved'),
      }
      return
    }
    void loadSourcePaths({ item_id: itemId, title: cita.title })
  }

  /**
   * Resuelve el asset de una ruta devuelta por el motor.
   *
   * El motor lee `assets.path` de la misma base y para el mismo ítem, así que
   * la comparación por ruta normalizada coincide por construcción. Tomar el
   * primer asset del ítem era adivinar —un ítem puede tener varios— y por eso
   * la vista previa salía rota.
   */
  async function resolverAsset(itemId: string, path: ResearchSourcePath) {
    const store = getStore()
    const assets = await store.assets.findByItem(itemId)
    const buscada = normalizePath(path.path)
    return assets.find((candidato) => normalizePath(candidato.path) === buscada)
  }

  /**
   * Trae el documento citado para verlo acá.
   *
   * El fragmento está citado en el informe: el investigador tiene que poder
   * mirar la página de la que salió sin abandonar la investigación.
   */
  async function loadPreview(itemId: string, path: ResearchSourcePath, titulo: string) {
    try {
      const asset = await resolverAsset(itemId, path)
      if (!asset || !mounted || selectedCitation?.title !== titulo) return
      const kind = classifyFileType(asset.path)
      if (kind !== 'image' && kind !== 'pdf') return
      previewFailed = false
      preview = { url: getAssetUrl(asset.path), kind, label: getAssetPathLabel(asset.path) }
    } catch {
      // Sin vista previa el panel sigue sirviendo: la ruta queda accionable.
    }
  }

  const selectedItemId = $derived(selectedCitation ? itemIdDe(selectedCitation) : '')
  const selectedPaths = $derived(selectedItemId ? (sourcePathsByItemId[selectedItemId] ?? []) : [])
  const selectedSourceError = $derived(
    selectedCitation
      ? (sourceErrorsByItemId[selectedItemId] ??
          sourceErrorsByItemId[selectedCitation.evidence_id] ??
          '')
      : ''
  )
  const canPause = $derived(Boolean(job && job.status === 'running'))
  const canResume = $derived(Boolean(job && job.status === 'paused'))
  const canCancel = $derived(Boolean(job && job.status !== 'done' && job.status !== 'failed'))
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
    })()
  )
  // El editor del presupuesto existe solo con el trabajo pausado: es el
  // estado en que el motor deja una investigación que agotó su techo, y el
  // único en que acepta cambiarlo. Con el trabajo corriendo, ofrecerlo sería
  // ofrecer un error.
  const canAdjustBudget = $derived(Boolean(job && job.status === 'paused'))
  let budgetEditing = $state(false)
  let budgetCalls = $state<number | null>(null)
  let budgetCost = $state<number | null>(null)
  // Lo ya consumido es el piso del techo nuevo: el motor rechaza un límite por
  // debajo de lo gastado. Decirlo antes evita descubrirlo por un error.
  const budgetFloor = $derived(
    $currentLocale && job
      ? translate('investigation.budgetFloor', {
          calls: job.llm_calls,
          cost: formatBudget(job.cost),
        })
      : ''
  )
  function openBudgetEditor() {
    if (!job) return
    // Precargado con el techo vigente: se ajusta, no se escribe de cero.
    budgetCalls = job.max_llm_calls ?? job.llm_calls
    budgetCost = job.max_cost
    budgetEditing = true
  }
  async function saveBudget() {
    if (!job || jobActionInFlight || !budgetCalls) return
    jobActionInFlight = 'budget'
    actionError = null
    try {
      await researchRequest({
        op: 'update_budget',
        job_id: job.id,
        max_llm_calls: budgetCalls,
        max_cost: budgetCost ?? null,
      })
      budgetEditing = false
      await refreshDetail()
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
    sources = []
    void refreshDetail()
  })

  onMount(() => {
    mounted = true
    lastLoadedJobId = jobId
    void refreshDetail().finally(() => {
      if (!mounted || pollTimer) return
      pollTimer = setInterval(() => {
        void refreshDetail()
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
      {#if canAdjustBudget && !budgetEditing}
        <Button
          variant="ghost"
          size="sm"
          disabled={jobActionInFlight !== null}
          onclick={openBudgetEditor}
        >
          <span>{$currentLocale && t('investigation.adjustBudget')}</span>
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

  {#if budgetEditing && canAdjustBudget}
    <section
      class="investigation-budget"
      aria-label={$currentLocale && t('investigation.adjustBudget')}
    >
      <label class="investigation-round__field">
        <span class="investigation-round__axis">{$currentLocale && t('research.callsLabel')}</span>
        <input
          class="investigation-round__input"
          type="number"
          min="1"
          step="1"
          bind:value={budgetCalls}
          disabled={jobActionInFlight !== null}
        />
      </label>
      <label class="investigation-round__field">
        <span class="investigation-round__axis">{$currentLocale && t('research.maxCostLabel')}</span
        >
        <input
          class="investigation-round__input"
          type="number"
          min="0"
          step="0.01"
          placeholder={$currentLocale && t('research.maxCostPlaceholder')}
          bind:value={budgetCost}
          disabled={jobActionInFlight !== null}
        />
      </label>
      <p class="investigation-budget__floor">{budgetFloor}</p>
      <div class="investigation-budget__actions">
        <Button
          variant="primary"
          size="sm"
          disabled={jobActionInFlight !== null}
          loading={jobActionInFlight === 'budget'}
          onclick={() => void saveBudget()}
        >
          <span>{$currentLocale && t('investigation.saveBudget')}</span>
        </Button>
        <Button
          variant="ghost"
          size="sm"
          disabled={jobActionInFlight !== null}
          onclick={() => (budgetEditing = false)}
        >
          <span>{$currentLocale && t('investigation.cancelBudget')}</span>
        </Button>
      </div>
    </section>
  {/if}

  <div class="investigation-view__body">
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
                <h3 class="report__label">
                  {$currentLocale && t('investigation.report.coverage')}
                </h3>
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
                      (a) => a.id === pregunta.id
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

    <!-- El informe cita; acá se lee la fuente sin salir de la investigación. -->
    <aside
      class="investigation-source"
      aria-label={$currentLocale && t('investigation.source.title')}
    >
      {#if selectedCitation}
        <h2 class="report__label">{$currentLocale && t('investigation.source.title')}</h2>
        <p class="investigation-source__heading">
          <span class="report__quote-ref">[{selectedCitation.n}]</span>
          <span>{citationLabel(selectedCitation)}</span>
        </p>
        <p class="report__quote-range">
          {selectedCitation.chunk_id} · {citationRange(selectedCitation)}
        </p>

        {#if selectedCitation.text}
          <h3 class="report__label">{$currentLocale && t('investigation.source.passage')}</h3>
          <blockquote class="investigation-source__passage">
            {selectedCitation.text}{selectedCitation.truncated ? ' […]' : ''}
          </blockquote>
        {/if}

        {#if preview && !previewFailed}
          <figure class="investigation-source__preview">
            {#if preview.kind === 'image'}
              <img
                src={preview.url}
                alt=""
                loading="lazy"
                onerror={() => {
                  previewFailed = true
                }}
              />
            {:else}
              <embed src={preview.url} type="application/pdf" title={preview.label} />
            {/if}
          </figure>
        {:else if previewFailed}
          <p class="investigation-source__preview-failed">
            {$currentLocale && t('investigation.source.previewFailed')}
          </p>
        {/if}

        {#if sourceLoadingItemId === selectedItemId}
          <p class="report__quote-range">{$currentLocale && t('investigation.source.loading')}</p>
        {:else if selectedSourceError}
          <p class="surface-message surface-message--error" role="alert">{selectedSourceError}</p>
        {:else if selectedPaths.length > 0}
          <ul class="investigation-source__paths">
            {#each selectedPaths as ruta (`${ruta.path}-${ruta.page ?? 0}`)}
              <li>
                <Button
                  variant="secondary"
                  size="sm"
                  onclick={() =>
                    void openSourcePath(
                      { item_id: selectedItemId, title: selectedCitation!.title },
                      ruta
                    )}
                >
                  <span>
                    {$currentLocale && t('investigation.source.openDocument')}{ruta.page
                      ? ` · p. ${ruta.page}`
                      : ''}
                  </span>
                </Button>
              </li>
            {/each}
          </ul>
        {/if}
      {:else}
        <p class="investigation-source__empty">
          {$currentLocale && t('investigation.source.empty')}
        </p>
      {/if}
    </aside>
  </div>
</div>

<style>
  .investigation-view {
    min-height: 100%;
  }

  /* El detalle usaba una sola columna y dejaba media pantalla vacía: la
     fuente citada entra ahí, al lado del informe que la cita. */
  .investigation-view__body {
    display: grid;
    /* El panel crece con la pantalla en vez de quedarse en una columna
       angosta que parte los identificadores en pedazos. */
    grid-template-columns: minmax(0, 1fr) minmax(0, clamp(22rem, 38vw, 46rem));
    gap: var(--space-4);
    align-items: start;
    min-width: 0;
  }

  @media (max-width: 60rem) {
    .investigation-view__body {
      grid-template-columns: minmax(0, 1fr);
    }
  }

  .investigation-source {
    position: sticky;
    top: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    /* Scroll propio: si el panel creciera más que la ventana, su encabezado
       terminaría montado sobre el contenido de la página. */
    max-height: calc(100vh - 10rem);
    overflow-y: auto;
    overflow-x: hidden;
    min-width: 0;
    padding: var(--space-3);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    background: var(--surface-panel);
  }

  /* Nada sale de la caja: ni un hash de 64 caracteres ni un nombre de archivo
     sin espacios. */
  .investigation-source > * {
    min-width: 0;
    max-width: 100%;
    overflow-wrap: anywhere;
  }

  .investigation-source__empty {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .investigation-source__heading {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-2);
    margin: 0;
    font-size: var(--font-size-sm);
    overflow-wrap: anywhere;
  }

  .investigation-source__passage {
    margin: 0;
    padding-left: var(--space-3);
    border-left: 2px solid var(--border-subtle);
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
    white-space: pre-line;
  }

  .investigation-source__preview {
    margin: 0;
    overflow: hidden;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    background: var(--surface-app);
  }

  .investigation-source__preview img {
    display: block;
    width: 100%;
    max-width: 100%;
    height: auto;
    max-height: 26rem;
    object-fit: contain;
  }

  .investigation-source__preview-failed {
    margin: 0;
    padding: var(--space-3);
    border: 1px dashed var(--border-subtle);
    border-radius: var(--radius-sm);
    color: var(--color-text-muted);
    font-size: var(--font-size-sm);
  }

  .investigation-source__preview embed {
    display: block;
    width: 100%;
    height: 22rem;
  }

  .investigation-source__paths {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
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
    min-width: 0;
    max-width: 100%;
    font-family: var(--font-mono, monospace);
    overflow-wrap: anywhere;
    word-break: break-word;
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
  .investigation-budget {
    display: flex;
    flex-wrap: wrap;
    align-items: end;
    gap: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--color-border, currentColor);
    border-radius: var(--radius-sm, 4px);
  }

  .investigation-budget .investigation-round__field {
    width: auto;
    min-width: 10rem;
  }

  .investigation-budget__floor {
    flex-basis: 100%;
    margin: 0;
    opacity: 0.8;
  }

  .investigation-budget__actions {
    display: flex;
    gap: var(--space-2);
  }
</style>
