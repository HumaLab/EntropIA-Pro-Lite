<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { listen, type UnlistenFn } from '@tauri-apps/api/event'
  import { Button } from '@entropia/ui'
  import {
    checkAllDeps,
    installAllDeps,
    installOneDep,
    getUvStatus,
    resetDeps,
    onDepsProgress,
    onDepsComplete,
    onDepsError,
    DEP_DISPLAY_NAMES,
    DEP_DESCRIPTIONS,
    CRITICAL_DEPS,
    type DepCheckResult,
    type DependencyId,
    type DependencyStatus,
    type UvStatusResult,
  } from '$lib/deps'
  import {
    getRuntimeStatus,
    onRuntimeProgress,
    onRuntimeStatus,
    repairRuntime,
    runtimeBlocksCurrentUse,
    runtimeCanBootstrapAutomatically,
    runtimeNeedsAttention,
    shouldShowRuntimeRepairAction,
    type RuntimeStatus,
    type RuntimeOperation,
  } from '$lib/runtime'
  import {
    llmDownloadModel,
    llmLocalModelInfo,
    type LocalModelInfo,
    type LlmDownloadProgressPayload,
  } from '$lib/llm'
  import {
    embeddingDownloadModel,
    embeddingLocalModelInfo,
    type EmbeddingDownloadProgressPayload,
    type LocalEmbeddingModelInfo,
  } from '$lib/embeddings'
  import { getFocusableElements, getNextFocusTrapTarget } from '$lib/modal-focus'

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let deps = $state<DepCheckResult[]>([])
  let uvStatus = $state<UvStatusResult | null>(null)
  let installing = $state(false)
  let errorBanner = $state<string | null>(null)
  let expandedErrors = $state<Set<DependencyId>>(new Set())
  let runtimeStatus = $state<RuntimeStatus | null>(null)
  let runtimeOperation = $state<RuntimeOperation | null>(null)
  let runtimeRechecking = $state(false)
  let llmModel = $state<LocalModelInfo | null>(null)
  let embeddingModel = $state<LocalEmbeddingModelInfo | null>(null)
  let preparingEntropia = $state(false)
  let prepareStep = $state<string | null>(null)
  let llmDownloading = $state(false)
  let llmDownloadPct = $state(0)
  let embeddingDownloading = $state(false)
  let embeddingDownloadPct = $state(0)
  let embeddingDownloadFile = $state('')
  let resetConfirmationOpen = $state(false)
  let resetConfirmationText = $state('')
  let resetting = $state(false)
  let resetConfirmationEl: HTMLElement | undefined = $state()
  let resetConfirmationTriggerEl: HTMLElement | null = null
  let runtimeOperationInFlight = false

  const RESET_CONFIRMATION_PHRASE = 'resetear entorno'

  // ---------------------------------------------------------------------------
  // Derived
  // ---------------------------------------------------------------------------

  let hasMissingOrFailed = $derived(
    deps.some((d) => d.status.type === 'missing' || d.status.type === 'failed'),
  )

  let allInstalled = $derived(deps.length > 0 && deps.every((d) => d.status.type === 'installed'))
  let runtimeBlocked = $derived(runtimeNeedsAttention(runtimeStatus))
  let runtimeReleaseOnlyIssue = $derived(runtimeStatus?.state === 'fixture')
  let runtimeBlockedOffline = $derived(runtimeStatus?.state === 'blocked_offline')
  let runtimeBlockedSourceUnavailable = $derived(
    runtimeStatus?.state === 'blocked_source_unavailable',
  )
  let runtimeBlockedUnreachable = $derived(
    runtimeBlockedOffline || runtimeBlockedSourceUnavailable,
  )
  let depsReadyButReleaseRuntimePending = $derived(
    allInstalled && runtimeBlocked && runtimeReleaseOnlyIssue,
  )
  let runtimeBlocksInstalledCapabilities = $derived(
    runtimeBlocksCurrentUse(runtimeStatus, allInstalled, uvStatus?.dev_fallback_available === true),
  )
  let canClaimAllReady = $derived(allInstalled && !runtimeBlocksInstalledCapabilities)
  let depsInstalledButRuntimeBlocked = $derived(allInstalled && runtimeBlocksInstalledCapabilities)
  let llmModelNeedsDownload = $derived(
    llmModel != null && !llmModel.available && llmModel.can_auto_download,
  )
  let embeddingModelNeedsDownload = $derived(
    embeddingModel != null && !embeddingModel.available && embeddingModel.can_auto_download,
  )
  let prepareEntropiaNeeded = $derived(
    runtimeBlocksInstalledCapabilities ||
      hasMissingOrFailed ||
      llmModelNeedsDownload ||
      embeddingModelNeedsDownload,
  )

  let overallProgress = $derived(() => {
    if (!installing || deps.length === 0) return 0
    const done = deps.filter(
      (d) => d.status.type === 'installed' || d.status.type === 'failed',
    ).length
    return Math.round((done / deps.length) * 100)
  })

  // ---------------------------------------------------------------------------
  // Event listeners
  // ---------------------------------------------------------------------------

  let unlisteners: Array<() => void> = []

  onMount(async () => {
    errorBanner = null
    runtimeOperation = null
    try {
      await refreshAllState()
    } catch (e) {
      errorBanner = `Error al verificar dependencias: ${String(e)}`
    }

    unlisteners.push(
      await onDepsProgress((event) => {
        deps = deps.map((d) => (d.id === event.id ? { ...d, status: event.status } : d))
      }),
      await onDepsComplete((event) => {
        deps = event.results
        installing = false
        void refreshRuntimeState()
      }),
      await onDepsError((event) => {
        errorBanner = event.error
        installing = false
      }),
      await onRuntimeStatus((status) => {
        runtimeStatus = status
        runtimeOperation = status.activeOperation
        runtimeOperationInFlight = status.activeOperation != null
      }),
      await onRuntimeProgress((operation) => {
        if (
          !runtimeOperationInFlight &&
          runtimeStatus?.state === 'healthy' &&
          operation.stage !== 'checking'
        ) {
          return
        }
        runtimeOperationInFlight = true
        runtimeOperation = operation
      }),
      await listen<LlmDownloadProgressPayload>('llm:download_progress', (event) => {
        llmDownloading = true
        llmDownloadPct = event.payload.pct
      }),
      await listen('llm:download_complete', async () => {
        llmDownloading = false
        llmDownloadPct = 100
        await refreshAiModelState().catch(() => undefined)
      }),
      await listen('llm:download_error', () => {
        llmDownloading = false
        llmDownloadPct = 0
      }),
      await listen<EmbeddingDownloadProgressPayload>('embedding:download_progress', (event) => {
        embeddingDownloading = true
        embeddingDownloadPct = event.payload.pct
        embeddingDownloadFile = event.payload.file
      }),
      await listen('embedding:download_complete', async () => {
        embeddingDownloading = false
        embeddingDownloadPct = 100
        embeddingDownloadFile = ''
        await refreshAiModelState().catch(() => undefined)
      }),
      await listen('embedding:download_error', () => {
        embeddingDownloading = false
        embeddingDownloadPct = 0
        embeddingDownloadFile = ''
      }),
    )
  })

  onDestroy(() => {
    unlisteners.forEach((fn) => fn())
  })

  // ---------------------------------------------------------------------------
  // Handlers
  // ---------------------------------------------------------------------------

  async function handleInstallAll() {
    installing = true
    errorBanner = null
    runtimeOperation = null
    deps = deps.map((dep) =>
      dep.status.type === 'installed'
        ? dep
        : { ...dep, status: { type: 'installing', percent: 0 } },
    )
    try {
      await installAllDeps()
      await refreshAllState()
    } catch (e) {
      errorBanner = String(e)
      await refreshAllState().catch(() => undefined)
    } finally {
      installing = false
    }
  }

  async function handleInstallOne(id: DependencyId) {
    errorBanner = null
    runtimeOperation = null
    deps = deps.map((d) =>
      d.id === id ? { ...d, status: { type: 'installing', percent: 0 } } : d,
    )
    try {
      const result = await installOneDep(id)
      deps = deps.map((d) => (d.id === id ? result : d))
      await refreshAllState()
    } catch (e) {
      deps = deps.map((d) =>
        d.id === id ? { ...d, status: { type: 'failed', message: String(e) } } : d,
      )
      await refreshAllState().catch(() => undefined)
    }
  }

  async function refreshAllState() {
    const [checkResults, uv, runtime, localLlm, localEmbedding] = await Promise.all([
      checkAllDeps(),
      getUvStatus(),
      getRuntimeStatus(),
      llmLocalModelInfo().catch(() => null),
      embeddingLocalModelInfo().catch(() => null),
    ])
    deps = checkResults
    uvStatus = uv
    runtimeStatus = runtime
    runtimeOperation = runtime.activeOperation
    runtimeOperationInFlight = runtime.activeOperation != null
    llmModel = localLlm
    embeddingModel = localEmbedding
  }

  async function refreshRuntimeState() {
    const [uv, runtime] = await Promise.all([getUvStatus(), getRuntimeStatus()])
    uvStatus = uv
    runtimeStatus = runtime
    runtimeOperation = runtime.activeOperation
    runtimeOperationInFlight = runtime.activeOperation != null
  }

  async function refreshAiModelState() {
    const [localLlm, localEmbedding] = await Promise.all([
      llmLocalModelInfo().catch(() => null),
      embeddingLocalModelInfo().catch(() => null),
    ])
    llmModel = localLlm
    embeddingModel = localEmbedding
  }

  async function waitForLlmDownload() {
    let doneUnlisten: UnlistenFn | null = null
    let errorUnlisten: UnlistenFn | null = null
    const cleanup = () => {
      doneUnlisten?.()
      errorUnlisten?.()
    }

    await new Promise<void>((resolve, reject) => {
      void (async () => {
        try {
          doneUnlisten = await listen('llm:download_complete', () => {
            cleanup()
            resolve()
          })
          errorUnlisten = await listen<{ error: string }>('llm:download_error', (event) => {
            cleanup()
            reject(new Error(event.payload.error))
          })
          await llmDownloadModel()
        } catch (e) {
          cleanup()
          reject(e)
        }
      })()
    })
  }

  async function waitForEmbeddingDownload() {
    let doneUnlisten: UnlistenFn | null = null
    let errorUnlisten: UnlistenFn | null = null
    const cleanup = () => {
      doneUnlisten?.()
      errorUnlisten?.()
    }

    await new Promise<void>((resolve, reject) => {
      void (async () => {
        try {
          doneUnlisten = await listen('embedding:download_complete', () => {
            cleanup()
            resolve()
          })
          errorUnlisten = await listen<{ error: string }>('embedding:download_error', (event) => {
            cleanup()
            reject(new Error(event.payload.error))
          })
          await embeddingDownloadModel()
        } catch (e) {
          cleanup()
          reject(e)
        }
      })()
    })
  }

  async function handlePrepareEntropia() {
    if (preparingEntropia) return
    preparingEntropia = true
    errorBanner = null
    try {
      prepareStep = 'Preparando runtime administrado'
      if (
        runtimeCanBootstrapAutomatically(runtimeStatus) ||
        shouldShowRuntimeRepairAction(runtimeStatus)
      ) {
        const status = await repairRuntime()
        runtimeStatus = status
        runtimeOperation = status.activeOperation
      }
      await refreshAllState()

      if (hasMissingOrFailed) {
        prepareStep = 'Instalando dependencias Python'
        if (!canInstallInCurrentDevState()) {
          throw new Error('No hay runtime/fallback listo para instalar dependencias automáticamente.')
        }
        installing = true
        await installAllDeps()
        installing = false
        await refreshAllState()
      }

      prepareStep = 'Descargando Gemma local'
      await refreshAiModelState()
      if (llmModelNeedsDownload) {
        await waitForLlmDownload()
        await refreshAiModelState()
      }

      prepareStep = 'Descargando BGE-M3 local'
      if (embeddingModelNeedsDownload) {
        await waitForEmbeddingDownload()
        await refreshAiModelState()
      }

      prepareStep = 'Verificando preparación'
      await refreshAllState()
    } catch (e) {
      errorBanner = e instanceof Error ? e.message : String(e)
      await refreshAllState().catch(() => undefined)
    } finally {
      installing = false
      preparingEntropia = false
      prepareStep = null
    }
  }

  function openResetConfirmation() {
    resetConfirmationTriggerEl = document.activeElement instanceof HTMLElement ? document.activeElement : null
    resetConfirmationOpen = true
    resetConfirmationText = ''
    errorBanner = null
  }

  function cancelResetConfirmation() {
    if (resetting) return
    resetConfirmationOpen = false
    resetConfirmationText = ''
    resetConfirmationTriggerEl?.focus()
    resetConfirmationTriggerEl = null
  }

  function handleResetConfirmationKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault()
      cancelResetConfirmation()
      return
    }

    if (event.key !== 'Tab') return

    const target = getNextFocusTrapTarget(
      getFocusableElements(resetConfirmationEl ?? null),
      event.target instanceof HTMLElement ? event.target : null,
      event.shiftKey,
      resetConfirmationEl ?? null,
    )

    if (target) {
      event.preventDefault()
      target.focus()
    }
  }

  async function handleReset() {
    if (resetConfirmationText !== RESET_CONFIRMATION_PHRASE || resetting) return
    resetting = true
    errorBanner = null
    runtimeOperation = null
    try {
      await resetDeps()
      await refreshAllState()
      resetConfirmationOpen = false
      resetConfirmationText = ''
      resetConfirmationTriggerEl = null
    } catch (e) {
      errorBanner = String(e)
    } finally {
      resetting = false
    }
  }

  $effect(() => {
    if (!resetConfirmationOpen || !resetConfirmationEl) return

    setTimeout(() => {
      getFocusableElements(resetConfirmationEl ?? null)[0]?.focus()
    }, 0)
  })

  async function handleRuntimeRepair() {
    try {
      errorBanner = null
      runtimeOperation = null
      const status = await repairRuntime()
      runtimeStatus = status
      runtimeOperation = status.activeOperation
      await refreshAllState()
    } catch (e) {
      errorBanner = String(e)
      await refreshAllState().catch(() => undefined)
    }
  }

  async function handleRuntimeRecheck() {
    if (runtimeRechecking) return
    runtimeRechecking = true
    errorBanner = null
    runtimeOperation = null
    try {
      await refreshAllState()
    } catch (e) {
      errorBanner = String(e)
      await refreshAllState().catch(() => undefined)
    } finally {
      runtimeRechecking = false
    }
  }

  function toggleError(id: DependencyId) {
    const next = new Set(expandedErrors)
    if (next.has(id)) next.delete(id)
    else next.add(id)
    expandedErrors = next
  }

  // ---------------------------------------------------------------------------
  // Display helpers
  // ---------------------------------------------------------------------------

  function statusIcon(status: DependencyStatus): string {
    switch (status.type) {
      case 'installed':
        return '✓'
      case 'missing':
        return '✗'
      case 'installing':
      case 'checking':
        return '⏳'
      case 'failed':
        return '⚠'
      default:
        return '?'
    }
  }

  function statusColor(status: DependencyStatus): string {
    switch (status.type) {
      case 'installed':
        return 'var(--color-success, #22c55e)'
      case 'missing':
        return 'var(--color-error, #ef4444)'
      case 'failed':
        return 'var(--color-warning, #f59e0b)'
      default:
        return 'var(--color-text-muted, #6b7280)'
    }
  }

  function isCritical(id: DependencyId): boolean {
    return CRITICAL_DEPS.includes(id)
  }

  function getInstalledVersion(dep: DepCheckResult): string | null {
    if (dep.status.type === 'installed') return dep.status.version ?? dep.version
    return dep.version
  }

  function getInstallingPercent(status: DependencyStatus): number {
    if (status.type === 'installing') return status.percent
    return 0
  }

  function getFailedMessage(status: DependencyStatus): string {
    if (status.type === 'failed') return status.message
    return ''
  }

  function getDepDisplayName(dep: DepCheckResult): string {
    return (DEP_DISPLAY_NAMES as Partial<Record<string, string>>)[dep.id] ?? dep.id
  }

  function getDepDescription(dep: DepCheckResult): string {
    return (
      (DEP_DESCRIPTIONS as Partial<Record<string, string>>)[dep.id] ??
      'Dependencia administrada por EntropIA Pro.'
    )
  }

  function supportsInstallOne(id: DependencyId): boolean {
    return id !== 'Python'
  }

  function isRuntimeFixture(status: RuntimeStatus | null): boolean {
    return status?.state === 'fixture'
  }

  function shouldExplainDevFallback(): boolean {
    return isRuntimeFixture(runtimeStatus) && Boolean(uvStatus?.dev_fallback_available)
  }

  function canInstallInCurrentDevState(): boolean {
    if (runtimeStatus?.state === 'healthy') return true
    if (depsReadyButReleaseRuntimePending) return false
    return Boolean(uvStatus?.dev_fallback_available)
  }

  function bootstrapProgressLabel(): string | null {
    if (!runtimeOperation) return null
    if (runtimeOperation.progressPercent != null) {
      return `${runtimeOperation.progressPercent}% · ${runtimeOperation.summary}`
    }
    if (runtimeOperation.totalBytes != null && runtimeOperation.downloadedBytes != null) {
      return `${runtimeOperation.downloadedBytes}/${runtimeOperation.totalBytes} bytes · ${runtimeOperation.summary}`
    }
    return runtimeOperation.summary
  }
</script>

<div class="deps-tab">
  {#if prepareEntropiaNeeded}
    <div class="deps-prepare-panel" role="status">
      <div class="deps-prepare-panel__copy">
        <strong>Prepará EntropIA Pro para uso local</strong>
        <span>
          Esto hidrata el runtime, dependencias y modelos dentro de la app. Después queda listo
          para usar sin pedirle al usuario instalaciones manuales.
        </span>
        {#if prepareStep}
          <p class="deps-prepare-panel__progress">{prepareStep}</p>
        {/if}
        {#if llmDownloading}
          <p class="deps-prepare-panel__progress">Gemma: {llmDownloadPct}%</p>
        {/if}
        {#if embeddingDownloading}
          <p class="deps-prepare-panel__progress">
            BGE-M3: {embeddingDownloadPct}%{embeddingDownloadFile ? ` · ${embeddingDownloadFile}` : ''}
          </p>
        {/if}
      </div>
      <Button
        variant="primary"
        onclick={handlePrepareEntropia}
        loading={preparingEntropia}
        disabled={preparingEntropia}
      >
        Preparar EntropIA Pro
      </Button>
    </div>
  {/if}

  {#if runtimeBlocksInstalledCapabilities}
    <div
      class="deps-runtime-panel"
      role="status"
    >
      <div class="deps-runtime-panel__copy">
        <strong>{runtimeStatus?.summary}</strong>
        {#if isRuntimeFixture(runtimeStatus)}
          <span>
            El runtime-pack de release SIGUE sin estar listo. Eso no cambia.
            {#if shouldExplainDevFallback()}
              {uvStatus?.dev_fallback_reason ?? 'Hay un fallback de desarrollo disponible para instalar dependencias localmente sin validar el runtime de release.'}
            {:else}
              Sin payloads reales ni fallback local usable, las capacidades bloqueadas no van a funcionar.
            {/if}
          </span>
        {/if}
        {#if runtimeStatus?.blockedCapabilities?.length}
          <span>Capacidades afectadas: {(runtimeStatus?.blockedCapabilities ?? []).join(', ')}</span>
        {/if}
        {#if runtimeStatus?.details?.length}
          <ul>
            {#each runtimeStatus?.details ?? [] as detail, i (i)}
              <li>{detail}</li>
            {/each}
          </ul>
        {/if}
        {#if runtimeStatus?.guidance?.length}
          <ul class="deps-runtime-panel__guidance">
            {#each runtimeStatus?.guidance ?? [] as guidanceItem, i (i)}
              <li>{guidanceItem}</li>
            {/each}
          </ul>
        {/if}
        {#if runtimeBlockedUnreachable}
          <div class="deps-runtime-panel__blocked" role="group" aria-label="Runtime no disponible">
            {#if runtimeBlockedOffline}
              <strong>No pudimos llegar a la fuente del runtime: parece que estás sin conexión.</strong>
              <span>
                EntropIA Pro necesita descargar y verificar el runtime administrado la primera vez.
                Conectate a internet y volvé a verificar. Mientras tanto, las capacidades locales
                (OCR, transcripción y NLP) quedan en pausa.
              </span>
            {:else}
              <strong>Todavía no hay una fuente de descarga confiable para el runtime.</strong>
              <span>
                El manifiesto firmado del runtime aún no está publicado, así que no se puede
                descargar ni verificar de forma segura. No es un problema de tu equipo. Volvé a
                verificar más tarde; cuando EntropIA publique una fuente firmada, la app la va a tomar
                automáticamente.
              </span>
            {/if}
          </div>
        {/if}
        {#if runtimeOperation}
          <p class="deps-runtime-panel__progress">{bootstrapProgressLabel()}</p>
        {/if}
        {#if runtimeCanBootstrapAutomatically(runtimeStatus)}
          <span>EntropIA Pro va a intentar preparar el runtime automáticamente cuando una fuente válida esté disponible.</span>
        {/if}
      </div>
      {#if shouldShowRuntimeRepairAction(runtimeStatus)}
        <Button variant="secondary" onclick={handleRuntimeRepair}>Reparar runtime</Button>
      {:else if runtimeBlockedUnreachable}
        <Button
          variant="secondary"
          onclick={handleRuntimeRecheck}
          loading={runtimeRechecking}
          disabled={runtimeRechecking}
        >
          {runtimeBlockedOffline ? 'Reintentar conexión' : 'Volver a verificar'}
        </Button>
      {/if}
    </div>
  {/if}

  <!-- Error banner -->
  {#if errorBanner}
    <div class="deps-banner deps-banner--error">
      <span class="deps-banner__message">{errorBanner}</span>
      <button
        class="deps-banner__dismiss"
        type="button"
        onclick={() => (errorBanner = null)}
        aria-label="Cerrar error"
      >
        ✕
      </button>
    </div>
  {/if}

  <!-- UV status row -->
  <div class="deps-uv-status">
    {#if uvStatus}
      {#if uvStatus.release_runtime_ready}
        <span class="deps-uv-status__text">
          uv {uvStatus.uv_version ?? ''} · {uvStatus.uv_path ?? ''}
          {#if uvStatus.venv_exists}
            · entorno virtual en {uvStatus.venv_path ?? ''}
          {:else if uvStatus.uv_source === 'managed-runtime'}
            · runtime embebido · sin venv administrado
          {:else}
            · sin entorno virtual
          {/if}
        </span>
      {:else if uvStatus.dev_fallback_available}
        <span class="deps-uv-status__text deps-uv-status__text--info">
          Fallback dev disponible · uv {uvStatus.uv_version ?? ''} · {uvStatus.uv_path ?? ''}
          {#if uvStatus.venv_exists}
            · entorno virtual local en {uvStatus.venv_path ?? ''}
          {:else}
            · sin entorno virtual local
          {/if}
        </span>
      {:else if hasMissingOrFailed}
        <span class="deps-uv-status__text deps-uv-status__text--warn">
          No hay fallback usable: faltan prerequisitos locales para gestionar dependencias automáticamente
        </span>
      {:else if runtimeBlocked}
        <span class="deps-uv-status__text deps-uv-status__text--warn">
          Gestión automática pausada hasta resolver el runtime de EntropIA Pro.
        </span>
      {:else}
        <span class="deps-uv-status__text deps-uv-status__text--info">
          Dependencias Python detectadas; uv administrado no requiere acciones.
        </span>
      {/if}
      {#if !uvStatus.release_runtime_ready && runtimeStatus?.state === 'fixture' && !depsReadyButReleaseRuntimePending}
        <p class="deps-uv-warning">
          Runtime de release no listo ({runtimeStatus.summary}). La gestión local en dev NO hidrata ni valida payloads de release.
        </p>
      {/if}
      {#if uvStatus.dev_fallback_reason && !depsReadyButReleaseRuntimePending}
        <p class="deps-uv-warning">{uvStatus.dev_fallback_reason}</p>
      {/if}
      {#if uvStatus.uv_warning && !depsReadyButReleaseRuntimePending}
        <p class="deps-uv-warning">{uvStatus.uv_warning}</p>
      {/if}
      {#if !uvStatus.uv_ready && uvStatus.uv_version && uvStatus.uv_path}
        <p class="deps-uv-warning">
          Detectado: uv {uvStatus.uv_version} en {uvStatus.uv_path}
        </p>
      {/if}
    {:else}
      <span class="deps-uv-status__text">Verificando uv...</span>
    {/if}
  </div>

  <!-- Install all button -->
  {#if hasMissingOrFailed && !installing}
    <div class="deps-actions">
      <Button variant="primary" onclick={handleInstallAll} disabled={installing || !canInstallInCurrentDevState()}>
        Instalar todo
      </Button>
      {#if !canInstallInCurrentDevState()}
        <p class="deps-actions__hint">
          Necesitas runtime release hidratado/compatible o un fallback de desarrollo disponible para esta plataforma.
        </p>
      {/if}
    </div>
  {/if}

  <!-- Progress bar -->
  {#if installing}
    <div class="deps-progress">
      <div class="deps-progress__bar">
        <div
          class="deps-progress__fill"
          style="width: {overallProgress()}%"
        ></div>
      </div>
      <span class="deps-progress__label">{overallProgress()}% instalado</span>
    </div>
  {/if}

  <!-- All installed banner -->
  {#if canClaimAllReady && !installing}
    <div class="deps-banner deps-banner--success">
      <span class="deps-banner__message">
        Todas las dependencias están instaladas y listas para usar.
      </span>
    </div>
  {:else if depsInstalledButRuntimeBlocked && !installing}
    <div class="deps-banner deps-banner--warning">
      <span class="deps-banner__message">
        Las dependencias Python están instaladas, pero el runtime de EntropIA Pro necesita atención antes de habilitar OCR, transcripción y NLP.
      </span>
    </div>
  {/if}

  <!-- Dependency list -->
  <div class="deps-list">
    {#each deps as dep (dep.id)}
      <div class="deps-row" class:deps-row--failed={dep.status.type === 'failed'}>
        <!-- Status icon -->
        <span class="deps-row__icon" style="color: {statusColor(dep.status)}">
          {statusIcon(dep.status)}
        </span>

        <!-- Name + description -->
        <div class="deps-row__info">
          <div class="deps-row__name-line">
            <strong class="deps-row__name">{getDepDisplayName(dep)}</strong>
            {#if isCritical(dep.id)}
              <span class="deps-badge deps-badge--required">Requerido</span>
            {/if}
            {#if dep.status.type === 'installed'}
              {@const version = getInstalledVersion(dep)}
              {#if version}
                <span class="deps-badge deps-badge--version">{version}</span>
              {/if}
            {/if}
          </div>
          <p class="deps-row__desc">{getDepDescription(dep)}</p>

          <!-- Installing progress per-item -->
          {#if dep.status.type === 'installing'}
            <div class="deps-row__progress">
              <div class="deps-progress__bar deps-progress__bar--sm">
                <div
                  class="deps-progress__fill"
                  style="width: {getInstallingPercent(dep.status)}%"
                ></div>
              </div>
              <span class="deps-row__progress-pct">{getInstallingPercent(dep.status)}%</span>
            </div>
          {/if}

          <!-- Error detail (expandable) -->
          {#if dep.status.type === 'failed'}
            <button
              class="deps-row__error-toggle"
              type="button"
              onclick={() => toggleError(dep.id)}
            >
              {expandedErrors.has(dep.id) ? 'Ocultar detalle' : 'Ver detalle del error'}
            </button>
            {#if expandedErrors.has(dep.id)}
              <pre class="deps-row__error-detail">{getFailedMessage(dep.status)}</pre>
            {/if}
          {/if}
        </div>

        <!-- Action button -->
        <div class="deps-row__action">
          {#if supportsInstallOne(dep.id) && dep.status.type === 'missing'}
            <Button
              variant="secondary"
              size="sm"
              onclick={() => handleInstallOne(dep.id)}
              disabled={installing}
            >
              Instalar
            </Button>
          {:else if supportsInstallOne(dep.id) && dep.status.type === 'failed'}
            <Button
              variant="secondary"
              size="sm"
              onclick={() => handleInstallOne(dep.id)}
              disabled={installing}
            >
              Reintentar
            </Button>
          {/if}
        </div>
      </div>
    {/each}

    {#if deps.length === 0 && !errorBanner}
      <p class="deps-empty">Verificando dependencias...</p>
    {/if}
  </div>

  <!-- Disk space estimate -->
  <p class="deps-disk-estimate">
    Espacio estimado en disco: ~2.5 GB (incluye modelos de IA y entorno virtual Python)
  </p>

  <!-- Reset button -->
  <div class="deps-danger-zone">
    <Button variant="danger" onclick={openResetConfirmation} disabled={installing || resetting}>
      Resetear entorno
    </Button>
    <p class="deps-danger-zone__hint">
      Elimina el entorno administrado y limpia la configuración de dependencias. Requiere reinstalación o rehidratación.
    </p>
  </div>

  {#if resetConfirmationOpen}
    <div
      bind:this={resetConfirmationEl}
      class="deps-reset-confirmation"
      role="alertdialog"
      aria-modal="true"
      aria-labelledby="deps-reset-title"
      aria-describedby="deps-reset-description"
      tabindex="-1"
      onkeydown={handleResetConfirmationKeydown}
    >
      <div class="deps-reset-confirmation__copy">
        <strong id="deps-reset-title">Confirmar reseteo del entorno</strong>
        <p id="deps-reset-description">
          Esta acción elimina el entorno administrado de IA y limpia las rutas Python usadas por OCR, transcripción y NLP.
          Para confirmar, escribí <code>{RESET_CONFIRMATION_PHRASE}</code>.
        </p>
      </div>
      <label class="deps-reset-confirmation__label" for="deps-reset-confirmation-input">
        Confirmación requerida
      </label>
      <input
        id="deps-reset-confirmation-input"
        class="deps-reset-confirmation__input"
        type="text"
        bind:value={resetConfirmationText}
        placeholder={RESET_CONFIRMATION_PHRASE}
        disabled={resetting}
        autocomplete="off"
      />
      <div class="deps-reset-confirmation__actions">
        <Button variant="ghost" onclick={cancelResetConfirmation} disabled={resetting}>
          Cancelar
        </Button>
        <Button
          variant="danger"
          onclick={handleReset}
          disabled={resetConfirmationText !== RESET_CONFIRMATION_PHRASE || resetting}
          loading={resetting}
        >
          Confirmar reseteo
        </Button>
      </div>
    </div>
  {/if}
</div>

<style>
  .deps-tab {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .deps-runtime-panel {
    display: flex;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    border: 1px solid rgba(245, 158, 11, 0.35);
    border-radius: var(--radius-md);
    background: rgba(245, 158, 11, 0.08);
    color: var(--color-warning);
  }

  .deps-prepare-panel {
    display: flex;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-4);
    border: 1px solid rgba(99, 102, 241, 0.35);
    border-radius: var(--radius-md);
    background: rgba(99, 102, 241, 0.08);
  }

  .deps-prepare-panel__copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--font-size-sm);
  }

  .deps-prepare-panel__progress {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-accent, #4f46e5);
    font-family: var(--font-mono, monospace);
  }

  .deps-runtime-panel__copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    font-size: var(--font-size-sm);
  }

  .deps-runtime-panel__copy ul {
    margin: 0;
    padding-left: var(--space-4);
  }

  .deps-runtime-panel__guidance {
    margin-top: var(--space-1);
    color: var(--color-warning);
  }

  .deps-runtime-panel__blocked {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin-top: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm, 4px);
    background: rgba(245, 158, 11, 0.12);
  }

  .deps-runtime-panel__blocked strong {
    color: var(--color-warning);
  }

  .deps-runtime-panel__progress {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-warning);
    font-family: var(--font-mono, monospace);
  }

  /* Banner */
  .deps-banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-md);
    font-size: var(--font-size-sm);
  }

  .deps-banner--error {
    background: var(--color-danger-soft);
    border: 1px solid rgba(239, 68, 68, 0.3);
    color: var(--color-danger);
  }

  .deps-banner--success {
    background: var(--color-success-soft);
    border: 1px solid rgba(34, 197, 94, 0.3);
    color: var(--color-success);
  }

  .deps-banner--warning {
    background: var(--color-warning-soft);
    border: 1px solid rgba(245, 158, 11, 0.3);
    color: var(--color-warning);
  }

  .deps-banner__message {
    flex: 1;
  }

  .deps-banner__dismiss {
    background: none;
    border: none;
    cursor: pointer;
    font-size: var(--font-size-sm);
    color: inherit;
    padding: 0 var(--space-1);
    opacity: 0.7;
  }

  .deps-banner__dismiss:hover {
    opacity: 1;
  }

  /* UV status */
  .deps-uv-status {
    padding: var(--space-2) 0;
  }

  .deps-uv-status__text {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted, #6b7280);
    font-family: var(--font-mono, monospace);
  }

  .deps-uv-status__text--warn {
    color: var(--color-warning, #f59e0b);
  }

  .deps-uv-status__text--info {
    color: var(--color-accent, #4f46e5);
  }

  .deps-uv-warning {
    margin: var(--space-1) 0 0;
    font-size: var(--font-size-xs);
    color: var(--color-warning);
  }

  /* Actions */
  .deps-actions {
    display: flex;
    gap: var(--space-3);
    align-items: center;
    flex-wrap: wrap;
  }

  .deps-actions__hint {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-muted, #6b7280);
  }

  /* Progress bar */
  .deps-progress {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .deps-progress__bar {
    flex: 1;
    height: 6px;
    background: var(--color-border-subtle, #e5e7eb);
    border-radius: var(--radius-full, 9999px);
    overflow: hidden;
  }

  .deps-progress__bar--sm {
    flex: none;
    width: 120px;
    height: 4px;
  }

  .deps-progress__fill {
    height: 100%;
    background: var(--color-accent, #6366f1);
    border-radius: var(--radius-full, 9999px);
    transition: width 0.3s ease;
  }

  .deps-progress__label {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted, #6b7280);
    white-space: nowrap;
  }

  /* Dep list */
  .deps-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .deps-row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--color-border-subtle, #e5e7eb);
    border-radius: var(--radius-md);
    background: var(--color-surface);
  }

  .deps-row--failed {
    border-color: rgba(245, 158, 11, 0.4);
    background: rgba(245, 158, 11, 0.04);
  }

  .deps-row__icon {
    font-size: 16px;
    line-height: 1.5;
    flex: 0 0 auto;
    margin-top: 2px;
  }

  .deps-row__info {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .deps-row__name-line {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .deps-row__name {
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
  }

  .deps-row__desc {
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary, #6b7280);
    margin: 0;
  }

  .deps-row__progress {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-1);
  }

  .deps-row__progress-pct {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted, #6b7280);
  }

  .deps-row__error-toggle {
    background: none;
    border: none;
    cursor: pointer;
    font-size: var(--font-size-xs);
    color: var(--color-warning, #f59e0b);
    padding: 0;
    text-decoration: underline;
    text-align: left;
  }

  .deps-row__error-detail {
    font-size: 11px;
    font-family: var(--font-mono, monospace);
    background: var(--color-surface-sunken, #f3f4f6);
    border: 1px solid var(--color-border, #d1d5db);
    border-radius: var(--radius-sm, 4px);
    padding: var(--space-2) var(--space-3);
    white-space: pre-wrap;
    word-break: break-all;
    max-height: 160px;
    overflow-y: auto;
    margin: 0;
    color: var(--color-text-primary);
  }

  .deps-row__action {
    flex: 0 0 auto;
  }

  /* Badges */
  .deps-badge {
    display: inline-block;
    padding: 2px 7px;
    border-radius: var(--radius-full, 9999px);
    font-size: 10px;
    font-weight: var(--font-weight-medium, 500);
    vertical-align: middle;
  }

  .deps-badge--required {
    background: rgba(99, 102, 241, 0.12);
    color: var(--color-accent);
  }

  .deps-badge--version {
    background: rgba(34, 197, 94, 0.12);
    color: var(--color-success);
    font-family: var(--font-mono, monospace);
  }

  /* Empty state */
  .deps-empty {
    font-size: var(--font-size-sm);
    color: var(--color-text-muted, #6b7280);
    text-align: center;
    padding: var(--space-6) 0;
  }

  /* Disk estimate */
  .deps-disk-estimate {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted, #6b7280);
    margin: 0;
    padding: var(--space-2) 0;
    border-top: 1px solid var(--color-border-subtle, #e5e7eb);
  }

  /* Danger zone */
  .deps-danger-zone {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    border: 1px solid rgba(239, 68, 68, 0.2);
    border-radius: var(--radius-md);
    background: rgba(239, 68, 68, 0.04);
  }

  .deps-danger-zone__hint {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted, #6b7280);
    margin: 0;
    flex: 1;
  }

  .deps-reset-confirmation {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-4);
    border: 1px solid rgba(239, 68, 68, 0.35);
    border-radius: var(--radius-md);
    background: rgba(127, 29, 29, 0.12);
  }

  .deps-reset-confirmation__copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .deps-reset-confirmation__copy strong {
    color: var(--color-text-primary);
  }

  .deps-reset-confirmation__copy p {
    margin: 0;
    color: var(--color-text-secondary, #6b7280);
    font-size: var(--font-size-sm);
  }

  .deps-reset-confirmation__copy code {
    padding: 2px 6px;
    border-radius: var(--radius-sm, 4px);
    background: rgba(239, 68, 68, 0.12);
    color: var(--color-danger);
    font-family: var(--font-mono, monospace);
  }

  .deps-reset-confirmation__label {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium, 500);
    color: var(--color-text-primary);
  }

  .deps-reset-confirmation__input {
    min-height: var(--control-height-md);
    padding: 0 var(--space-3);
    border: 1px solid var(--color-border-subtle, #e5e7eb);
    border-radius: var(--radius-md);
    background: var(--color-surface);
    color: var(--color-text-primary);
    font-family: var(--font-mono, monospace);
  }

  .deps-reset-confirmation__input:focus {
    outline: none;
    box-shadow: var(--focus-ring);
    border-color: var(--color-danger);
  }

  .deps-reset-confirmation__actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
</style>
