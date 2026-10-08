<script lang="ts">
  import { tooltip } from '@entropia/ui'
  import { onDestroy, onMount, untrack } from 'svelte'
  import { getNavigation, getPaneId } from '$lib/pane-context'
  import { workspace } from '$lib/workspace'
  import { locale, t, type I18nKey } from '$lib/i18n'
  import {
    describeBackendError,
    researchCreate,
    researchDelete,
    researchList,
    takeResearchHandoff,
    type ResearchCollectionSummary,
    type ResearchHandoffDraft,
    type ResearchJobStatus,
    type ResearchJobSummary,
    type ResearchScope,
  } from '$lib/research'
  import {
    bibliographyLibraryStatus,
    type BibliographyLibraryStatus,
  } from '$lib/bibliography-search'
  import { libraryChoiceKey, selectedLibrariesAfterToggle } from '$lib/rag-scope'
  import type { RagLibraryRef } from '$lib/rag'
  import {
    DEFAULT_OPENROUTER_MODEL,
    SETTINGS_KEYS,
    settingsGet,
    testOpenrouterConnection,
    type ModelInfo,
  } from '$lib/settings'

  import { renderMarkdown } from '$lib/markdown'
  import {
    ActionIcon,
    Button,
    Card,
    Checkbox,
    ConfirmDialog,
    IconButton,
    Input,
    TabButton,
    TabList,
    ToolbarMenu,
    type ToolbarMenuItem,
  } from '@entropia/ui'

  const navigation = getNavigation()
  const paneId = getPaneId()

  const currentLocale = locale

  const STATUS_LABELS: Record<ResearchJobStatus, I18nKey> = {
    planned: 'research.status.planned',
    running: 'research.status.running',
    paused: 'research.status.paused',
    awaiting_human: 'research.status.awaitingHuman',
    done: 'research.status.done',
    failed: 'research.status.failed',
  }

  let jobs = $state<ResearchJobSummary[]>([])
  let collections = $state<ResearchCollectionSummary[]>([])
  let loading = $state(true)
  let error = $state<string | null>(null)
  let submitError = $state<string | null>(null)
  let creating = $state(false)
  let refreshRequestId = 0
  let pollingTimer: ReturnType<typeof setInterval> | null = null
  let pendingDeleteId = $state<string | null>(null)
  let deleting = $state(false)
  let mounted = false

  let title = $state('')
  let question = $state('')
  let project = $state('investigación')
  let selectedCollectionIds = $state<string[]>([])
  /**
   * El alcance arranca con las colecciones que tienen material procesado, pero
   * eso es un default de primera carga: en cuanto el investigador lo toca, la
   * lista es suya. Sin esta marca, deseleccionar todo duraba hasta el próximo
   * refresco —1,5 s— que volvía a seleccionarlo.
   */
  let scopeTouched = false
  let maxLlmCalls = $state('80')
  let maxCost = $state('')
  let handoff = $state<ResearchHandoffDraft | null>(null)

  // ── Modelo del trabajo ─────────────────────────────────────────────────
  // El modelo se elige al crear y el motor lo congela en el job: es lo que
  // permite comparar gemma contra luna sin tocar Configuración.
  let researchModel = $state('')
  let availableModels = $state<ModelInfo[]>([])

  /** El modelo efectivo de Investigación: el ajuste del chat RAG manda y el
   *  modelo general de OpenRouter lo cubre — el mismo orden con el que el
   *  backend arma el cliente. Lo que el investigador ya escribió no se pisa. */
  async function loadResearchModel() {
    const [ragModel, openrouterModel] = await Promise.all([
      settingsGet(SETTINGS_KEYS.RAG_MODEL).catch(() => null),
      settingsGet(SETTINGS_KEYS.OPENROUTER_MODEL).catch(() => null),
    ])
    if (!mounted || researchModel.trim()) return
    researchModel = ragModel?.trim() || openrouterModel?.trim() || ''
  }

  /** Las sugerencias del catálogo de OpenRouter, con la credencial guardada
   *  (el mismo patrón que Configuración). Sin credencial o sin red no hay
   *  lista: el campo sigue editable a mano. */
  async function loadSuggestedModels() {
    try {
      const models = await testOpenrouterConnection('')
      if (!mounted) return
      availableModels = Array.isArray(models) ? models : []
    } catch {
      availableModels = []
    }
  }

  function chooseModel(id: string) {
    researchModel = id
    submitError = null
  }

  // ── Alcance de la evidencia: Corpus / Biblioteca / Ambos ─────────────────
  // Los mismos controles, tokens y vocabulario que el chat de Recuperar:
  // TabList para el alcance, ToolbarMenu para las bibliotecas.
  const EVIDENCE_SCOPES: Array<{
    id: ResearchScope
    label: 'ragChat.scopeCorpus' | 'ragChat.scopeBiblioteca' | 'ragChat.scopeBoth'
  }> = [
    { id: 'corpus', label: 'ragChat.scopeCorpus' },
    { id: 'biblioteca', label: 'ragChat.scopeBiblioteca' },
    { id: 'ambos', label: 'ragChat.scopeBoth' },
  ]
  let evidenceScope = $state<ResearchScope>('corpus')
  const scopeIncludesCorpus = $derived(evidenceScope !== 'biblioteca')
  const scopeIncludesBibliography = $derived(evidenceScope !== 'corpus')
  let libraryStatus = $state<BibliographyLibraryStatus | null>(null)
  let libraryStatusLoading = $state(false)
  let libraryStatusFailed = $state(false)
  let libraryMenuOpen = $state(false)
  let libraryStatusRequest = 0
  /** `null` = todas las sincronizadas, como en el chat. */
  let selectedLibraries = $state<RagLibraryRef[] | null>(null)

  async function loadLibraryStatus() {
    const request = ++libraryStatusRequest
    libraryStatusLoading = true
    libraryStatusFailed = false
    try {
      const status = await bibliographyLibraryStatus()
      if (request !== libraryStatusRequest) return
      libraryStatus = status
    } catch {
      if (request !== libraryStatusRequest) return
      libraryStatus = null
      libraryStatusFailed = true
    } finally {
      if (request === libraryStatusRequest) libraryStatusLoading = false
    }
  }

  // El catálogo se consulta solo cuando el alcance deja de ser Corpus: la
  // investigación de corpus nunca lo toca.
  $effect(() => {
    if (evidenceScope !== 'corpus') untrack(() => void loadLibraryStatus())
  })

  const libraryRefs = $derived<RagLibraryRef[]>(
    (libraryStatus?.libraries ?? []).map((library) => ({
      libraryType: library.libraryType,
      libraryId: library.libraryId,
    }))
  )
  const chosenLibraries = $derived(selectedLibraries ?? libraryRefs)
  const checkedLibraryKeys = $derived(new Set(chosenLibraries.map((ref) => libraryChoiceKey(ref))))
  const librariesAllChecked = $derived(
    libraryRefs.length > 0 &&
      libraryRefs.every((ref) => checkedLibraryKeys.has(libraryChoiceKey(ref)))
  )
  const libraryTriggerLabel = $derived(
    $currentLocale &&
      (librariesAllChecked
        ? t('ragChat.librariesAll')
        : t('ragChat.librariesSome', {
            count: libraryRefs.filter((ref) => checkedLibraryKeys.has(libraryChoiceKey(ref)))
              .length,
            total: libraryRefs.length,
          }))
  )
  const libraryItems = $derived<ToolbarMenuItem[]>([
    {
      kind: 'radio',
      id: 'all',
      label: $currentLocale && t('ragChat.librariesAll'),
      checked: librariesAllChecked,
      onselect: () => keepLibraryMenuOpen(() => (selectedLibraries = null)),
    },
    ...(libraryStatus?.libraries ?? []).map((library) => {
      const ref = { libraryType: library.libraryType, libraryId: library.libraryId }
      return {
        kind: 'checkbox' as const,
        id: libraryChoiceKey(ref),
        label:
          $currentLocale &&
          `${library.name} · ${t('ragChat.libraryCounts', {
            works: library.works,
            passages: library.passages,
          })}`,
        checked: checkedLibraryKeys.has(libraryChoiceKey(ref)),
        onselect: () =>
          keepLibraryMenuOpen(() => {
            selectedLibraries = selectedLibrariesAfterToggle(selectedLibraries, libraryRefs, ref)
          }),
      }
    }),
  ])

  // Elegir bibliotecas son varios clics: el menú se reabre solo hasta que el
  // investigador lo cierra.
  function keepLibraryMenuOpen(apply: () => void) {
    apply()
    libraryMenuOpen = true
  }

  const libraryStatusMessage = $derived.by(() => {
    if (!scopeIncludesBibliography) return null
    if (libraryStatusLoading && !libraryStatus) return t('ragChat.libraryStatusLoading')
    if (libraryStatusFailed) return t('ragChat.libraryStatusError')
    if (!libraryStatus) return null
    if (libraryStatus.libraries.length === 0) return t('ragChat.libraryStatusNone')
    if (!libraryStatus.vectorReady) return t('ragChat.libraryStatusNoEmbeddings')
    return null
  })

  function chooseEvidenceScope(next: ResearchScope) {
    evidenceScope = next
    submitError = null
  }

  const collectionCountLabel = $derived(
    collections.length === 1
      ? t('research.collectionsOne', { count: collections.length })
      : t('research.collectionsMany', { count: collections.length })
  )

  const selectedCollectionCountLabel = $derived(
    selectedCollectionIds.length === 1
      ? t('research.selectedCollectionsOne', { count: selectedCollectionIds.length })
      : t('research.selectedCollectionsMany', { count: selectedCollectionIds.length })
  )

  const contextSummaryLabel = $derived.by(() => {
    if (!handoff?.context?.length) return ''
    const messageCount = handoff.context.length
    const sourceCount = handoff.context.reduce(
      (total, message) => total + (message.sources?.length ?? 0),
      0
    )
    return t('research.contextSummary', { messages: messageCount, sources: sourceCount })
  })

  const canSubmit = $derived.by(() => {
    const trimmedQuestion = question.trim()
    const parsedCalls = Number.parseInt(maxLlmCalls, 10)
    return (
      !creating &&
      trimmedQuestion.length > 0 &&
      // Cada alcance exige lo que realmente usa: las colecciones alimentan la
      // pierna del corpus y las bibliotecas, la de la Biblioteca.
      (!scopeIncludesCorpus || selectedCollectionIds.length > 0) &&
      (!scopeIncludesBibliography || chosenLibraries.length > 0) &&
      Number.isInteger(parsedCalls) &&
      parsedCalls > 0
    )
  })

  function translate(key: I18nKey, params?: Record<string, string | number>) {
    return t(key, params)
  }

  function normalizeSelection(ids: string[]) {
    scopeTouched = true
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

  /** Borrar es destructivo y sin vuelta: siempre pasa por confirmación. */
  async function confirmDelete() {
    const id = pendingDeleteId
    if (!id || deleting) return
    deleting = true
    error = null
    try {
      await researchDelete(id)
      // Only once the delete actually landed: a rejected delete leaves the
      // job (and any history entry pointing at it) in place.
      workspace.forgetResearch(id)
      pendingDeleteId = null
      await refreshJobs({ silent: true })
    } catch (deleteError) {
      error = describeBackendError(deleteError, () => translate('research.deleteError'))
    } finally {
      deleting = false
    }
  }

  function clearCollections() {
    scopeTouched = true
    selectedCollectionIds = []
  }

  const allCollectionsSelected = $derived(
    collections.length > 0 && selectedCollectionIds.length === collections.length
  )

  /** Un solo control para las dos acciones: el icono dice en qué estado está. */
  function toggleAllCollections() {
    if (allCollectionsSelected) clearCollections()
    else selectAllCollections()
  }

  function statusLabel(job: ResearchJobSummary): string {
    return translate(
      job.status === 'failed' && job.close_reason === 'blocked'
        ? 'research.status.blocked'
        : STATUS_LABELS[job.status]
    )
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
      // Solo mientras el investigador no haya elegido: un alcance vacío que él
      // eligió es una decisión, no un estado a corregir.
      if (!scopeTouched && selectedCollectionIds.length === 0) {
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
    // La validación de las bibliotecas es propia: el alcance bibliográfico sin
    // ninguna biblioteca sincronizada elegida no tiene qué buscar.
    if (scopeIncludesBibliography && chosenLibraries.length === 0) {
      submitError = translate('research.needLibrary')
      return
    }
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
        title: title.trim(),
        question: question.trim(),
        project: project.trim() || 'investigación',
        // El motor siempre pide un recorte de colecciones, incluso cuando el
        // alcance no correrá la pierna del corpus: con el selector oculto se
        // manda el recorte que la vista ya tenía preparado.
        collection_ids:
          selectedCollectionIds.length > 0
            ? [...selectedCollectionIds]
            : collections.map((collection) => collection.id),
        max_llm_calls: parsedCalls,
        max_cost: parsedCost,
        context: handoff?.context ?? null,
        alcance: evidenceScope,
        // Las referencias viajan en el único formato que hablan los dos lados:
        // «user:123» / «group:456».
        bibliotecas: chosenLibraries.map(libraryChoiceKey),
        // El modelo elegido viaja tal cual: el motor lo valida y lo congela
        // en el job. Vacío no viaja: el motor usa el modelo configurado.
        modelo: researchModel.trim() || undefined,
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
    void loadResearchModel()
    void loadSuggestedModels()
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
  <section class="page-header research-view__header" aria-labelledby="research-title-{paneId}">
    <div class="page-header__content">
      <span class="page-header__eyebrow">{$currentLocale && t('research.eyebrow')}</span>
      <h1 id="research-title-{paneId}">{$currentLocale && t('research.title')}</h1>
      <span class="page-header__meta">{collectionCountLabel}</span>
    </div>
  </section>

  {#if error}
    <p class="surface-message surface-message--error" role="alert">{error}</p>
  {/if}

  <div class="research-view__layout">
    <section class="research-view__jobs" aria-labelledby="research-jobs-title-{paneId}">
      <div class="research-view__section-header">
        <h2 id="research-jobs-title-{paneId}">{$currentLocale && t('research.jobsTitle')}</h2>
        <span class="research-view__section-meta">{jobs.length}</span>
      </div>

      {#if loading && jobs.length === 0}
        <p class="surface-message surface-message--center">
          {$currentLocale && t('research.loading')}
        </p>
      {:else if jobs.length === 0}
        <div class="surface-message surface-message--center">
          <p>{$currentLocale && t('research.empty')}</p>
        </div>
      {:else}
        <div class="research-view__job-list">
          {#each jobs as job (job.id)}
            <div class="research-job-row">
              <button
                type="button"
                class="research-job-card"
                onclick={() =>
                  navigation.navigate({
                    name: 'investigation',
                    jobId: job.id,
                    title: job.title,
                  })}
              >
                <span class="research-job-card__question" use:tooltip={job.title}>{job.title}</span>
                <span class="research-job-card__meta">{statusLabel(job)}</span>
              </button>
              <IconButton
                class="research-job-card__discard"
                variant="ghost"
                size="sm"
                label={$currentLocale && t('research.deleteTitle')}
                title={$currentLocale && t('research.deleteTitle')}
                disabled={deleting}
                onclick={() => {
                  pendingDeleteId = job.id
                }}
              >
                <ActionIcon name="delete" size={14} />
              </IconButton>
            </div>
          {/each}
        </div>
      {/if}
    </section>

    <section class="research-view__form-column" aria-labelledby="research-form-title-{paneId}">
      <Card>
        <form
          class="research-form"
          onsubmit={(event) => {
            event.preventDefault()
            void handleSubmit()
          }}
        >
          <div class="research-form__copy">
            <h2 id="research-form-title-{paneId}">{$currentLocale && t('research.formTitle')}</h2>
          </div>
          <label class="research-form__field">
            <Input
              type="text"
              placeholder={$currentLocale && t('research.titlePlaceholder')}
              bind:value={title}
            />
          </label>
          <label class="research-form__field">
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

          <!-- El modelo del trabajo: el mismo campo y la misma lista de
               sugerencias que Configuración («Modelos sugeridos desde
               OpenRouter»). El valor viaja en el create y el motor lo congela
               en el job. -->
          <div class="research-form__field">
            <Input
              label={$currentLocale && t('research.modelLabel')}
              type="text"
              bind:value={researchModel}
              placeholder={DEFAULT_OPENROUTER_MODEL}
            />
            {#if availableModels.length > 0}
              <div class="settings__model-list">
                <p class="settings__model-list-title">
                  {$currentLocale && t('settings.suggestedModels')}
                </p>
                {#each availableModels
                  .filter((m) => m.id.includes('gemma') || m.id.includes('llama') || m.id.includes('mistral') || m.id.includes('qwen') || m.id.includes('claude') || m.id.includes('gpt'))
                  .slice(0, 15) as m (m.id)}
                  <button
                    class="settings__model-option"
                    type="button"
                    class:selected={researchModel === m.id}
                    onclick={() => chooseModel(m.id)}
                  >
                    <span class="settings__model-id">{m.id}</span>
                    <span class="settings__model-ctx"
                      >{Math.round(m.context_length / 1024)}k ctx</span
                    >
                  </button>
                {/each}
              </div>
            {/if}
          </div>

          <div class="research-form__evidence">
            <span class="research-form__evidence-label"
              >{$currentLocale && t('ragChat.scopeLabel')}</span
            >
            <TabList aria-label={$currentLocale && t('ragChat.scopeLabel')}>
              {#each EVIDENCE_SCOPES as entry (entry.id)}
                <TabButton
                  active={evidenceScope === entry.id}
                  onclick={() => chooseEvidenceScope(entry.id)}
                >
                  {$currentLocale && t(entry.label)}
                </TabButton>
              {/each}
            </TabList>
            {#if scopeIncludesBibliography && libraryStatus && libraryStatus.libraries.length > 0}
              <ToolbarMenu
                label={$currentLocale && t('ragChat.librariesMenu')}
                items={libraryItems}
                bind:open={libraryMenuOpen}
              >
                {#snippet trigger(props, { open })}
                  <button
                    type="button"
                    class="research-form__library-trigger"
                    class:research-form__library-trigger--open={open}
                    {...props}
                  >
                    <span>{libraryTriggerLabel}</span>
                    <ActionIcon name="chevron-down" size={12} />
                  </button>
                {/snippet}
              </ToolbarMenu>
            {/if}
          </div>
          {#if libraryStatusMessage}
            <p class="research-form__hint" role="status">
              {$currentLocale && libraryStatusMessage}
            </p>
          {/if}

          <!-- Las colecciones alimentan la pierna del corpus: solo se eligen
               cuando el alcance la incluye. -->
          {#if scopeIncludesCorpus}
            <details class="research-form__advanced">
              <summary>{$currentLocale && t('research.collectionsLabel')}</summary>
              <fieldset class="research-form__scope">
                <div class="research-form__scope-header">
                  <span class="research-form__scope-count">{selectedCollectionCountLabel}</span>
                  <div class="research-form__scope-actions">
                    <IconButton
                      size="sm"
                      label={$currentLocale &&
                        t(allCollectionsSelected ? 'research.deselectAll' : 'research.selectAll')}
                      title={$currentLocale &&
                        t(allCollectionsSelected ? 'research.deselectAll' : 'research.selectAll')}
                      disabled={collections.length === 0}
                      onclick={toggleAllCollections}
                    >
                      <ActionIcon
                        name={allCollectionsSelected ? 'circle-x' : 'check-check'}
                        size={14}
                      />
                    </IconButton>
                  </div>
                </div>
                {#if collections.length === 0}
                  <p class="research-form__hint research-form__hint--empty">
                    {$currentLocale && t('research.noCollections')}
                  </p>
                {:else}
                  <div class="research-form__scope-list">
                    {#each collections as collection (collection.id)}
                      <Checkbox
                        class="research-form__scope-card"
                        checked={selectedCollectionIds.includes(collection.id)}
                        onchange={(checked) => {
                          toggleCollection(collection.id, checked)
                          submitError = null
                        }}
                      >
                        <span class="research-form__scope-row">
                          <span class="research-form__scope-name" use:tooltip={collection.name}
                            >{collection.name}</span
                          >
                          <span class="research-form__scope-count">{collection.items}</span>
                        </span>
                      </Checkbox>
                    {/each}
                  </div>
                {/if}
              </fieldset>
            </details>
          {/if}

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
                              {$currentLocale &&
                                t('research.handoffSources', { count: message.sources.length })}
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
              <span
                >{creating
                  ? $currentLocale && t('research.starting')
                  : $currentLocale && t('research.start')}</span
              >
            </Button>
          </div>
        </form>
      </Card>
    </section>

    {#if pendingDeleteId}
      <ConfirmDialog
        title={$currentLocale && t('research.deleteTitle')}
        titleId="research-delete-title"
        message={$currentLocale && t('research.deleteMessage')}
        cancelLabel={$currentLocale && t('collections.cancel')}
        confirmLabel={$currentLocale && t('research.confirmDelete')}
        variant="destructive"
        oncancel={() => {
          pendingDeleteId = null
        }}
        onconfirm={() => void confirmDelete()}
      />
    {/if}
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

  /* Anteriores is the Colecciones grid at the width the left column actually
     has. A 280px floor steps 3 / 2 / 2 / 1 cards per row as the window narrows;
     320px reads better on a maximised window but collapses to one card at the
     width the left column has on a 1100px window, which is common enough to
     decide it.

     `auto-fill` rather than `auto-fit`, as in Escritura and Colecciones: with a
     single past investigation, auto-fit would stretch its card across the whole
     column. The two behave identically once there are more jobs than columns. */
  .research-view__job-list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
    gap: var(--space-3);
    align-content: start;
  }

  /* Same shape as an Escritura card: the whole box is the control that opens
     the investigation, so there is no separate open icon to aim at. The delete
     control stays a SIBLING of the card — a button nested in a button is
     invalid, and the browser would hand the click to the outer one anyway — so
     the row is the positioning context and the control is laid over the corner
     the card reserves for it. */
  .research-job-row {
    position: relative;
    display: flex;
    min-width: 0;
  }

  .research-job-row :global(.research-job-card__discard) {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
  }

  .research-job-card {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    justify-content: space-between;
    gap: var(--space-1);
    flex: 1;
    min-width: 0;
    min-height: 44px;
    /* The right inset is the delete control's seat. Without it a long title
       runs under the button instead of ellipsing before it. sm is a 28px
       container, plus a gap on each side. */
    padding: var(--space-3) calc(var(--space-2) + 28px + var(--space-2)) var(--space-3)
      var(--space-3);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition:
      background var(--transition-base),
      border-color var(--transition-base);
  }

  .research-job-card:hover {
    background: var(--color-surface-elevated);
    border-color: var(--color-border-strong);
  }

  .research-job-card:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  /* One step down from the full-width row this used to be: at a third of the
     column, --font-size-lg wrapped the question onto three lines.

     `align-items: stretch` on the card is what lets this ellipse: a column flex
     item that shrink-wraps its text has no width to overflow. */
  .research-job-card__question {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
    line-height: var(--line-height-tight);
  }

  .research-job-card__meta {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .research-form {
    display: grid;
    gap: var(--space-4);
  }

  .research-form__copy {
    display: grid;
    gap: var(--space-1);
  }

  .research-form__field,
  /* El recuadro claro de esquinas rectas que se veía acá no era nuestro: es
     el borde por defecto que el navegador le da a <fieldset>. */
  .research-form__scope {
    display: grid;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    border: 0;
    min-inline-size: 0;
  }

  .research-form__hint {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .research-form__hint--empty {
    font-style: italic;
  }

  /* La lista de modelos sugeridos es la MISMA que Configuración
     (`settings__model-*` en SettingsView): mismas clases y mismos tokens para
     que las dos listas sean visualmente la misma. */
  .settings__model-list {
    max-height: 240px;
    overflow-y: auto;
    border: 1px solid color-mix(in srgb, var(--color-hairline) 78%, transparent);
    border-radius: var(--radius-surface);
    background: color-mix(in srgb, var(--color-surface-glass) 72%, transparent);
  }

  .settings__model-list-title {
    padding: var(--space-2) var(--space-3);
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    border-bottom: 1px solid color-mix(in srgb, var(--color-hairline) 72%, transparent);
  }
  .settings__model-option {
    display: flex;
    justify-content: space-between;
    align-items: center;
    width: 100%;
    padding: var(--space-2) var(--space-3);
    border: none;
    background: transparent;
    cursor: pointer;
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    text-align: left;
    transition: background-color var(--transition-smooth);
  }
  .settings__model-option:hover {
    background: color-mix(in srgb, var(--color-surface-glass) 82%, transparent);
  }

  .settings__model-option.selected {
    background: color-mix(in srgb, var(--color-accent) 10%, var(--color-surface-glass));
    font-weight: var(--font-weight-medium);
  }

  .settings__model-option + .settings__model-option {
    border-top: 1px solid var(--color-border-subtle);
  }

  .settings__model-id {
    color: var(--color-text-primary);
  }

  .settings__model-ctx {
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  /* El alcance de la evidencia y la elección de bibliotecas son los MISMOS
     controles y tokens que el chat de Recuperar: TabList para el alcance y un
     ToolbarMenu pintado con los tokens de la app (nunca un <select> nativo). */
  .research-form__evidence {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .research-form__evidence-label {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.075em;
    text-transform: uppercase;
  }

  .research-form__library-trigger {
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

  .research-form__library-trigger:hover,
  .research-form__library-trigger--open {
    background: var(--surface-toolbar);
    border-color: var(--border-panel);
  }

  .research-form__library-trigger:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .research-form__library-trigger :global(svg) {
    color: var(--color-text-muted);
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

  /* El listado es el contenedor: una superficie con el mismo borde y radio
     que los demás paneles, no una caja aparte dentro del panel. */
  /* The densest of the four grids in the app, because it lives inside the form
     column rather than across a page. A 180px floor steps 3 / 2 / 2 / 1 cards
     per row as that column narrows, and lands on 1 once the column hits its own
     360px floor — which is the point: two columns crammed into 312px of usable
     width would be narrower than the names they hold.

     The header above is a sibling of this box, so the count stays put while the
     cards scroll. */
  .research-form__scope-list {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: var(--space-2);
    align-content: start;
    max-height: 220px;
    overflow-y: auto;
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    background: var(--surface-input);
  }

  /* The card IS the Checkbox, exactly as in the Lotes picker. Its <label>
     already makes the whole surface a click target, already toggles on Space,
     and already draws one focus ring — so there is no wrapper handler to fire
     twice. What is added is the lift off the sunken list, and a checked border
     firmer than the resting one: the component's own checked rule lands on
     --border-subtle, which is this card's RESTING border. */
  .research-form__scope-list :global(.research-form__scope-card) {
    gap: var(--space-2);
    padding: var(--space-2);
    min-width: 0;
    background: var(--surface-panel);
    border-color: var(--border-subtle);
  }

  .research-form__scope-list :global(.research-form__scope-card:hover) {
    background: var(--surface-toolbar);
  }

  .research-form__scope-list :global(.research-form__scope-card:has(input:checked)) {
    background: var(--surface-toolbar);
    border-color: var(--border-panel);
  }

  .research-form__scope-row {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    min-width: 0;
  }

  .research-form__scope-name {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: var(--font-weight-medium);
  }

  .research-form__scope-count {
    flex: none;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    font-variant-numeric: tabular-nums;
  }

  .research-form__textarea {
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

  .research-form__textarea:focus {
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

  /* Keyed to this pane's own width (the `pane` container WorkPane.svelte
     establishes), not the window: the right column's own 360px floor
     (grid-template-columns above) never yielded in a narrow split pane
     because a plain `@media` query only ever sees the window, which is
     rarely the pane's width (columns-adapt-to-pane-width fix). Same 980px
     breakpoint as before, so a single pane behaves exactly as it did. */
  @container pane (max-width: 980px) {
    .research-view__layout {
      grid-template-columns: 1fr;
    }
  }

  @media (max-width: 640px) {
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
