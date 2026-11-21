<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { listen } from '@tauri-apps/api/event'
  import {
    ActionIcon,
    Button,
    Dialog,
    IconButton,
    Input,
    ProgressBar,
    StatusBadge,
  } from '@entropia/ui'
  import { dependencyStatusIcon, dependencyStatusIconClass } from '$lib/deps-status'
  import { registerListeners } from '$lib/register-listeners'
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
    DOWNLOAD_SCOPES,
    localSetupInstall,
    localSetupStatus,
    pendingComponents,
    type LocalComponent,
    type LocalComponentId,
    type LocalSetupStatus,
  } from '$lib/local-setup'
  import { t } from '$lib/i18n'
  import {
    clearInflight,
    downloadKey,
    isInflight,
    markInflight,
    registerModelDownloads,
    trackInflight,
    whenInflightSettles,
  } from '$lib/inflight.svelte'

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  let deps = $state<DepCheckResult[]>([])
  let uvStatus = $state<UvStatusResult | null>(null)
  // Installs, the preparation and model downloads keep running while the user
  // is on another screen; these flags start from the app-wide record.
  const INSTALL_KEY = 'deps:install'
  const PREPARE_KEY = 'deps:prepare'
  const SCOPE_BY_COMPONENT = Object.fromEntries(
    Object.entries(DOWNLOAD_SCOPES).map(([scope, id]) => [id, scope])
  ) as Partial<Record<LocalComponentId, string>>
  registerModelDownloads(Object.keys(DOWNLOAD_SCOPES))
  let installing = $state(isInflight(INSTALL_KEY))
  let errorBanner = $state<string | null>(null)
  let expandedErrors = $state<Set<DependencyId>>(new Set())
  let runtimeStatus = $state<RuntimeStatus | null>(null)
  let runtimeOperation = $state<RuntimeOperation | null>(null)
  let runtimeRechecking = $state(false)
  // Models and servers (Gemma, BGE-M3, reranker, Parakeet, llama-server,
  // GLM-OCR, Qwen3-VL), as the backend inventory reports them.
  let setup = $state<LocalSetupStatus | null>(null)
  let setupProgress = $state<Partial<Record<LocalComponentId, number>>>({})
  let setupErrors = $state<Partial<Record<LocalComponentId, string>>>({})
  let preparingEntropia = $state(isInflight(PREPARE_KEY))
  let prepareStep = $state<string | null>(null)
  let resetConfirmationOpen = $state(false)
  let resetConfirmationText = $state('')
  let resetting = $state(false)
  let runtimeOperationInFlight = false

  const RESET_CONFIRMATION_PHRASE = 'resetear entorno'

  // ---------------------------------------------------------------------------
  // Derived
  // ---------------------------------------------------------------------------

  let hasMissingOrFailed = $derived(
    deps.some((d) => d.status.type === 'missing' || d.status.type === 'failed')
  )

  let allInstalled = $derived(deps.length > 0 && deps.every((d) => d.status.type === 'installed'))
  let runtimeBlocked = $derived(runtimeNeedsAttention(runtimeStatus))
  let runtimeReleaseOnlyIssue = $derived(runtimeStatus?.state === 'fixture')
  let runtimeBlockedOffline = $derived(runtimeStatus?.state === 'blocked_offline')
  let runtimeBlockedSourceUnavailable = $derived(
    runtimeStatus?.state === 'blocked_source_unavailable'
  )
  let runtimeBlockedUnreachable = $derived(runtimeBlockedOffline || runtimeBlockedSourceUnavailable)
  let depsReadyButReleaseRuntimePending = $derived(
    allInstalled && runtimeBlocked && runtimeReleaseOnlyIssue
  )
  let runtimeBlocksInstalledCapabilities = $derived(
    runtimeBlocksCurrentUse(runtimeStatus, allInstalled, uvStatus?.dev_fallback_available === true)
  )
  let canClaimAllReady = $derived(allInstalled && !runtimeBlocksInstalledCapabilities)
  let depsInstalledButRuntimeBlocked = $derived(allInstalled && runtimeBlocksInstalledCapabilities)
  // The runtime has its own panel; the rest are models and servers.
  let modelComponents = $derived(
    (setup?.components ?? []).filter((component) => component.id !== 'runtime')
  )
  let modelsPending = $derived(
    pendingComponents(setup).filter((component) => component.id !== 'runtime')
  )
  let prepareEntropiaNeeded = $derived(
    runtimeBlocksInstalledCapabilities || hasMissingOrFailed || modelsPending.length > 0
  )

  let overallProgress = $derived(() => {
    if (!installing || deps.length === 0) return 0
    const done = deps.filter(
      (d) => d.status.type === 'installed' || d.status.type === 'failed'
    ).length
    return Math.round((done / deps.length) * 100)
  })

  // ---------------------------------------------------------------------------
  // Event listeners
  // ---------------------------------------------------------------------------

  let unlisteners: Array<() => void> = []
  // The listeners register after the initial refresh, one at a time through
  // the shared helper. A tab destroyed before that refresh settles registers
  // nothing; one destroyed during registration releases what lands
  // afterwards; and every handler ignores what Tauri still delivers in
  // between (drop-dup fix).
  let destroyed = false

  onMount(async () => {
    errorBanner = null
    runtimeOperation = null
    // Started on an earlier visit: refresh once it ends.
    for (const key of [INSTALL_KEY, PREPARE_KEY]) {
      void whenInflightSettles(key)?.then(() => {
        if (destroyed) return
        if (key === INSTALL_KEY) installing = false
        else preparingEntropia = false
        void refreshAllState().catch(() => undefined)
      })
    }
    try {
      await refreshAllState()
    } catch (e) {
      errorBanner = t('deps.checkFailed', { error: String(e) })
    }
    if (destroyed) return
    restoreDownloadProgress()

    try {
      const registered = await registerListeners(
        [
          () =>
            onDepsProgress((event) => {
              if (destroyed) return
              deps = deps.map((d) => (d.id === event.id ? { ...d, status: event.status } : d))
            }),
          () =>
            onDepsComplete((event) => {
              if (destroyed) return
              deps = event.results
              installing = false
              void refreshRuntimeState()
            }),
          () =>
            onDepsError((event) => {
              if (destroyed) return
              errorBanner = event.error
              installing = false
            }),
          () =>
            onRuntimeStatus((status) => {
              if (destroyed) return
              // uv's status reads the runtime too (`release_runtime_ready`,
              // the release fallback note): a new runtime state makes it stale.
              if (status.state !== runtimeStatus?.state) {
                void getUvStatus()
                  .then((uv) => {
                    if (!destroyed) uvStatus = uv
                  })
                  .catch(() => undefined)
              }
              runtimeStatus = status
              runtimeOperation = status.activeOperation
              runtimeOperationInFlight = status.activeOperation != null
            }),
          () =>
            onRuntimeProgress((operation) => {
              if (destroyed) return
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
          ...Object.entries(DOWNLOAD_SCOPES).flatMap(([scope, id]) => [
            () =>
              listen<{ pct: number }>(`${scope}:download_progress`, (event) => {
                if (destroyed) return
                setupProgress = { ...setupProgress, [id]: event.payload.pct }
                setupErrors = { ...setupErrors, [id]: undefined }
              }),
            () =>
              listen(`${scope}:download_complete`, () => {
                if (destroyed) return
                // GLM-OCR and Qwen download two files, one `complete` each:
                // the inventory says when the component is whole.
                void refreshSetupState()
              }),
            () =>
              listen<{ error: string }>(`${scope}:download_error`, (event) => {
                if (destroyed) return
                setupProgress = { ...setupProgress, [id]: undefined }
                setupErrors = { ...setupErrors, [id]: event.payload.error }
              }),
          ]),
        ],
        () => destroyed
      )
      unlisteners.push(...registered)
    } catch (e) {
      // Best-effort wiring: the helper already released what had landed, so
      // the tab stays usable and only surfaces the failure.
      if (!destroyed) errorBanner = t('deps.listenersFailed', { error: String(e) })
    }
  })

  onDestroy(() => {
    destroyed = true
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
      dep.status.type === 'installed' ? dep : { ...dep, status: { type: 'installing', percent: 0 } }
    )
    try {
      await trackInflight(INSTALL_KEY, installAllDeps)
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
    deps = deps.map((d) => (d.id === id ? { ...d, status: { type: 'installing', percent: 0 } } : d))
    try {
      const result = await installOneDep(id)
      deps = deps.map((d) => (d.id === id ? result : d))
      await refreshAllState()
    } catch (e) {
      deps = deps.map((d) =>
        d.id === id ? { ...d, status: { type: 'failed', message: String(e) } } : d
      )
      await refreshAllState().catch(() => undefined)
    }
  }

  async function refreshAllState() {
    const [checkResults, uv, runtime, localSetup] = await Promise.all([
      checkAllDeps(),
      getUvStatus(),
      getRuntimeStatus(),
      localSetupStatus().catch(() => null),
    ])
    deps = checkResults
    uvStatus = uv
    runtimeStatus = runtime
    runtimeOperation = runtime.activeOperation
    runtimeOperationInFlight = runtime.activeOperation != null
    setup = localSetup
  }

  async function refreshSetupState() {
    const next = await localSetupStatus().catch(() => null)
    if (destroyed || !next) return
    setup = next
    // A finished component no longer shows a progress bar.
    for (const component of next.components) {
      if (!component.ready) continue
      setupProgress = { ...setupProgress, [component.id]: undefined }
      const scope = SCOPE_BY_COMPONENT[component.id]
      if (scope) clearInflight(downloadKey(scope))
    }
  }

  function markDownload(id: LocalComponentId) {
    const scope = SCOPE_BY_COMPONENT[id]
    if (scope) markInflight(downloadKey(scope))
  }

  /** Shows again the model downloads that kept running while the user was away. */
  function restoreDownloadProgress() {
    for (const [scope, id] of Object.entries(DOWNLOAD_SCOPES)) {
      if (!isInflight(downloadKey(scope))) continue
      if (setup?.components.find((component) => component.id === id)?.ready) {
        clearInflight(downloadKey(scope))
      } else if (setupProgress[id] === undefined) {
        setupProgress = { ...setupProgress, [id]: 0 }
      }
    }
  }

  async function refreshRuntimeState() {
    const [uv, runtime] = await Promise.all([getUvStatus(), getRuntimeStatus()])
    uvStatus = uv
    runtimeStatus = runtime
    runtimeOperation = runtime.activeOperation
    runtimeOperationInFlight = runtime.activeOperation != null
  }

  /** Start one model or server, the same request Preparar EntropIA Pro makes. */
  async function handleInstallComponent(component: LocalComponent) {
    setupErrors = { ...setupErrors, [component.id]: undefined }
    setupProgress = { ...setupProgress, [component.id]: 0 }
    try {
      const [outcome] = await localSetupInstall([component.id])
      if (outcome?.started) markDownload(component.id)
      if (outcome && !outcome.started) {
        setupProgress = { ...setupProgress, [component.id]: undefined }
        setupErrors = { ...setupErrors, [component.id]: outcome.message }
      }
    } catch (e) {
      setupProgress = { ...setupProgress, [component.id]: undefined }
      setupErrors = { ...setupErrors, [component.id]: e instanceof Error ? e.message : String(e) }
    }
  }

  async function handlePrepareEntropia() {
    if (preparingEntropia) return
    preparingEntropia = true
    errorBanner = null
    await trackInflight(PREPARE_KEY, runPrepareEntropia)
  }

  async function runPrepareEntropia() {
    try {
      prepareStep = t('deps.prepare.stepRuntime')
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
        prepareStep = t('deps.prepare.stepPython')
        if (!canInstallInCurrentDevState()) {
          throw new Error(t('deps.prepare.noRuntimeForPython'))
        }
        installing = true
        await trackInflight(INSTALL_KEY, installAllDeps)
        installing = false
        await refreshAllState()
      }

      // Every model and server this machine can use, at once: the downloads
      // run in the background and each row shows its own progress.
      prepareStep = t('deps.prepare.stepModels')
      const pending = modelsPending.map((component) => component.id)
      if (pending.length > 0) {
        for (const id of pending) setupProgress = { ...setupProgress, [id]: 0 }
        const outcomes = await localSetupInstall(pending)
        for (const outcome of outcomes) {
          if (outcome.started) {
            markDownload(outcome.id)
            continue
          }
          setupProgress = { ...setupProgress, [outcome.id]: undefined }
          setupErrors = { ...setupErrors, [outcome.id]: outcome.message }
        }
      }
      await refreshSetupState()
    } catch (e) {
      errorBanner = e instanceof Error ? e.message : String(e)
      await refreshAllState().catch(() => undefined)
    } finally {
      installing = false
      preparingEntropia = false
      prepareStep = null
    }
  }

  // Dialog owns focus: it moves it to the typed-phrase input on open, traps
  // Tab, answers Escape and gives focus back to the trigger once it closes.
  function openResetConfirmation() {
    resetConfirmationOpen = true
    resetConfirmationText = ''
    errorBanner = null
  }

  function cancelResetConfirmation() {
    if (resetting) return
    resetConfirmationOpen = false
    resetConfirmationText = ''
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
    } catch (e) {
      errorBanner = String(e)
    } finally {
      resetting = false
    }
  }

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
    return (DEP_DESCRIPTIONS as Partial<Record<string, string>>)[dep.id] ?? t('deps.managedByPro')
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
        <strong>{t('deps.prepare.title')}</strong>
        <span>{t('deps.prepare.description')}</span>
        {#if prepareStep}
          <p class="deps-prepare-panel__progress">{prepareStep}</p>
        {/if}
      </div>
      <Button
        variant="primary"
        onclick={handlePrepareEntropia}
        loading={preparingEntropia}
        disabled={preparingEntropia}
      >
        {t('deps.prepare.action')}
      </Button>
    </div>
  {/if}

  {#if runtimeBlocksInstalledCapabilities}
    <div class="deps-runtime-panel" role="status">
      <div class="deps-runtime-panel__copy">
        <strong>{runtimeStatus?.summary}</strong>
        {#if isRuntimeFixture(runtimeStatus) && runtimeStatus?.bootstrapEligible}
          <span>{t('deps.runtime.fixtureDownloading')}</span>
        {:else if isRuntimeFixture(runtimeStatus)}
          <span>
            {t('deps.runtime.fixture')}
            {#if shouldExplainDevFallback()}
              {uvStatus?.dev_fallback_reason ?? t('deps.runtime.devFallbackAvailable')}
            {:else}
              {t('deps.runtime.noFallback')}
            {/if}
          </span>
        {/if}
        {#if runtimeStatus?.blockedCapabilities?.length}
          <span>
            {t('deps.runtime.affected', {
              list: (runtimeStatus?.blockedCapabilities ?? []).join(', '),
            })}
          </span>
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
          <div
            class="deps-runtime-panel__blocked"
            role="group"
            aria-label={t('deps.runtime.unavailableAria')}
          >
            {#if runtimeBlockedOffline}
              <strong>{t('deps.runtime.offlineTitle')}</strong>
              <span>{t('deps.runtime.offlineBody')}</span>
            {:else if runtimeStatus?.blockedReason === 'no_matching_release'}
              <strong>{t('deps.runtime.noReleaseTitle')}</strong>
              <span>{t('deps.runtime.noReleaseBody')}</span>
            {:else}
              <strong>{t('deps.runtime.noSourceTitle')}</strong>
              <span>{t('deps.runtime.noSourceBody')}</span>
            {/if}
          </div>
        {/if}
        {#if runtimeOperation}
          <p class="deps-runtime-panel__progress">{bootstrapProgressLabel()}</p>
        {/if}
        {#if runtimeCanBootstrapAutomatically(runtimeStatus)}
          <span>{t('deps.runtime.willBootstrap')}</span>
        {/if}
      </div>
      {#if shouldShowRuntimeRepairAction(runtimeStatus)}
        <Button variant="secondary" onclick={handleRuntimeRepair}>
          {t('deps.runtime.repair')}
        </Button>
      {:else if runtimeBlockedUnreachable}
        <Button
          variant="secondary"
          onclick={handleRuntimeRecheck}
          loading={runtimeRechecking}
          disabled={runtimeRechecking}
        >
          {runtimeBlockedOffline ? t('deps.runtime.retryConnection') : t('deps.runtime.recheck')}
        </Button>
      {/if}
    </div>
  {/if}

  <!-- Error banner -->
  {#if errorBanner}
    <div class="deps-banner deps-banner--error">
      <span class="deps-banner__message">{errorBanner}</span>
      <IconButton size="xs" label={t('deps.closeError')} onclick={() => (errorBanner = null)}>
        <ActionIcon name="close" size={12} />
      </IconButton>
    </div>
  {/if}

  <!-- UV status row -->
  <div class="deps-uv-status">
    {#if uvStatus}
      {#if uvStatus.release_runtime_ready}
        <span class="deps-uv-status__text">
          uv {uvStatus.uv_version ?? ''} · {uvStatus.uv_path ?? ''}
          {#if uvStatus.venv_exists}
            · {t('deps.uv.venvAt', { path: uvStatus.venv_path ?? '' })}
          {:else if uvStatus.uv_source === 'managed-runtime'}
            · {t('deps.uv.embeddedRuntime')}
          {:else}
            · {t('deps.uv.noVenv')}
          {/if}
        </span>
      {:else if uvStatus.dev_fallback_available}
        <span class="deps-uv-status__text deps-uv-status__text--info">
          {t('deps.uv.devFallback')} · uv {uvStatus.uv_version ?? ''} · {uvStatus.uv_path ?? ''}
          {#if uvStatus.venv_exists}
            · {t('deps.uv.localVenvAt', { path: uvStatus.venv_path ?? '' })}
          {:else}
            · {t('deps.uv.noLocalVenv')}
          {/if}
        </span>
      {:else if hasMissingOrFailed}
        <span class="deps-uv-status__text deps-uv-status__text--warn">
          {t('deps.uv.noFallback')}
        </span>
      {:else if runtimeBlocked}
        <span class="deps-uv-status__text deps-uv-status__text--warn">
          {t('deps.uv.paused')}
        </span>
      {:else}
        <span class="deps-uv-status__text deps-uv-status__text--info">
          {t('deps.uv.allDetected')}
        </span>
      {/if}
      {#if !uvStatus.release_runtime_ready && runtimeStatus?.state === 'fixture' && !depsReadyButReleaseRuntimePending}
        <p class="deps-uv-warning">
          {t('deps.uv.devNotRelease', { summary: runtimeStatus.summary })}
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
          {t('deps.uv.detected', { version: uvStatus.uv_version, path: uvStatus.uv_path })}
        </p>
      {/if}
    {:else}
      <span class="deps-uv-status__text">{t('deps.uv.checking')}</span>
    {/if}
  </div>

  <!-- Install all button -->
  {#if hasMissingOrFailed && !installing}
    <div class="deps-actions">
      <Button
        variant="primary"
        onclick={handleInstallAll}
        disabled={installing || !canInstallInCurrentDevState()}
      >
        {t('deps.installAll')}
      </Button>
      {#if !canInstallInCurrentDevState()}
        <p class="deps-actions__hint">{t('deps.installAllNeedsRuntime')}</p>
      {/if}
    </div>
  {/if}

  <!-- Progress bar -->
  {#if installing}
    <div class="deps-progress">
      <div class="deps-progress__track">
        <ProgressBar value={overallProgress()} aria-labelledby="deps-progress-label" />
      </div>
      <span id="deps-progress-label" class="deps-progress__label">
        {t('deps.installedPct', { pct: overallProgress() })}
      </span>
    </div>
  {/if}

  <!-- All installed banner -->
  {#if canClaimAllReady && !installing}
    <div class="deps-banner deps-banner--success">
      <span class="deps-banner__message">{t('deps.allInstalled')}</span>
    </div>
  {:else if depsInstalledButRuntimeBlocked && !installing}
    <div class="deps-banner deps-banner--warning">
      <span class="deps-banner__message">{t('deps.installedButRuntime')}</span>
    </div>
  {/if}

  <!-- Dependency list -->
  <div class="deps-list">
    {#each deps as dep (dep.id)}
      <div class="deps-row" class:deps-row--failed={dep.status.type === 'failed'}>
        <!-- Status icon -->
        <span class="deps-row__icon {dependencyStatusIconClass(dep.status)}">
          <ActionIcon name={dependencyStatusIcon(dep.status)} size={16} />
        </span>

        <!-- Name + description -->
        <div class="deps-row__info">
          <div class="deps-row__name-line">
            <strong class="deps-row__name">{getDepDisplayName(dep)}</strong>
            {#if isCritical(dep.id)}
              <StatusBadge variant="info" size="xs">{t('deps.required')}</StatusBadge>
            {/if}
            {#if dep.status.type === 'installed'}
              {@const version = getInstalledVersion(dep)}
              {#if version}
                <StatusBadge variant="success" size="xs">
                  <span class="deps-row__version">{version}</span>
                </StatusBadge>
              {/if}
            {/if}
          </div>
          <p class="deps-row__desc">{getDepDescription(dep)}</p>

          <!-- Installing progress per-item -->
          {#if dep.status.type === 'installing'}
            <div class="deps-row__progress">
              <div class="deps-row__progress-track">
                <ProgressBar
                  size="sm"
                  value={getInstallingPercent(dep.status)}
                  label={t('deps.installing', { name: getDepDisplayName(dep) })}
                />
              </div>
              <span class="deps-row__progress-pct">{getInstallingPercent(dep.status)}%</span>
            </div>
          {/if}

          <!-- Error detail (expandable) -->
          {#if dep.status.type === 'failed'}
            <span class="deps-row__error-toggle">
              <Button variant="link" onclick={() => toggleError(dep.id)}>
                {expandedErrors.has(dep.id) ? t('deps.hideDetail') : t('deps.showDetail')}
              </Button>
            </span>
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
              {t('deps.install')}
            </Button>
          {:else if supportsInstallOne(dep.id) && dep.status.type === 'failed'}
            <Button
              variant="secondary"
              size="sm"
              onclick={() => handleInstallOne(dep.id)}
              disabled={installing}
            >
              {t('deps.retry')}
            </Button>
          {/if}
        </div>
      </div>
    {/each}

    {#if deps.length === 0 && !errorBanner}
      <p class="deps-empty">{t('deps.checking')}</p>
    {/if}
  </div>

  <!-- Models and servers: the same inventory the assistant reads (models_setup). -->
  <h3 class="deps-section-title">{t('deps.models.title')}</h3>
  <p class="deps-section-hint">{t('deps.models.hint')}</p>
  <div class="deps-list">
    {#each modelComponents as component (component.id)}
      {@const progress = setupProgress[component.id]}
      {@const error = setupErrors[component.id]}
      <div class="deps-row" class:deps-row--failed={Boolean(error)}>
        <span
          class="deps-row__icon"
          class:deps-row__icon--installed={component.ready}
          class:deps-row__icon--missing={!component.ready && !component.unavailableReason}
          class:deps-row__icon--neutral={!component.ready && Boolean(component.unavailableReason)}
        >
          <!-- Shape carries the state: ready, missing, or not for this machine. -->
          <ActionIcon
            name={component.ready
              ? 'circle-check'
              : component.unavailableReason
                ? 'circle-help'
                : 'circle-x'}
            size={16}
          />
        </span>
        <div class="deps-row__info">
          <div class="deps-row__name-line">
            <strong class="deps-row__name">{component.label}</strong>
            {#if component.ready}
              <StatusBadge variant="success" size="xs">{t('deps.models.ready')}</StatusBadge>
            {:else if component.unavailableReason}
              <StatusBadge variant="neutral" size="xs">
                {t('deps.models.notForThisMachine')}
              </StatusBadge>
            {:else}
              <StatusBadge variant="warning" size="xs">{t('deps.models.missing')}</StatusBadge>
            {/if}
          </div>
          <p class="deps-row__desc">{component.enables}</p>
          <p class="deps-row__desc">{component.unavailableReason ?? component.detail}</p>
          {#if progress != null && !component.ready}
            <div class="deps-row__progress">
              <div class="deps-row__progress-track">
                <ProgressBar
                  size="sm"
                  value={progress}
                  label={t('deps.models.downloading', { name: component.label })}
                />
              </div>
              <span class="deps-row__progress-pct">{progress}%</span>
            </div>
          {/if}
          {#if error}
            <p class="deps-row__error-detail">{error}</p>
          {/if}
        </div>
        <div class="deps-row__action">
          {#if !component.ready && !component.unavailableReason && progress == null}
            <Button variant="secondary" size="sm" onclick={() => handleInstallComponent(component)}>
              {error ? t('deps.models.retry') : t('deps.models.install')}
            </Button>
          {/if}
        </div>
      </div>
    {/each}
    {#if setup === null && !errorBanner}
      <p class="deps-empty">{t('deps.models.checking')}</p>
    {/if}
  </div>

  <!-- Reset button -->
  <div class="deps-danger-zone">
    <Button variant="danger" onclick={openResetConfirmation} disabled={installing || resetting}>
      {t('deps.reset.action')}
    </Button>
    <p class="deps-danger-zone__hint">{t('deps.reset.hint')}</p>
  </div>

  <Dialog
    bind:open={() => resetConfirmationOpen, (next) => !next && cancelResetConfirmation()}
    title={t('deps.reset.title')}
    titleId="deps-reset-title"
    description={t('deps.reset.description')}
    dismissible={!resetting}
    dismissOnBackdrop={false}
  >
    <p class="deps-reset-confirmation__phrase">
      {t('deps.reset.typeToConfirm')} <code>{RESET_CONFIRMATION_PHRASE}</code>.
    </p>
    <Input
      id="deps-reset-confirmation-input"
      label={t('deps.reset.confirmationLabel')}
      font="mono"
      bind:value={resetConfirmationText}
      placeholder={RESET_CONFIRMATION_PHRASE}
      disabled={resetting}
      autocomplete="off"
      data-dialog-initial-focus
    />
    {#snippet footer()}
      <Button variant="secondary" onclick={cancelResetConfirmation} disabled={resetting}>
        {t('collections.cancel')}
      </Button>
      <Button
        variant="danger"
        onclick={handleReset}
        disabled={resetConfirmationText !== RESET_CONFIRMATION_PHRASE || resetting}
        loading={resetting}
      >
        {t('deps.reset.confirm')}
      </Button>
    {/snippet}
  </Dialog>
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
    border: 1px solid color-mix(in srgb, var(--color-warning) 30%, transparent);
    border-radius: var(--radius-md);
    background: var(--color-warning-soft);
    color: var(--color-warning);
  }

  .deps-prepare-panel {
    display: flex;
    justify-content: space-between;
    gap: var(--space-4);
    padding: var(--space-4);
    border: 1px solid color-mix(in srgb, var(--color-accent) 30%, transparent);
    border-radius: var(--radius-md);
    background: var(--color-accent-soft);
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
    color: var(--color-accent);
    font-family: var(--font-mono);
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
    border-radius: var(--radius-sm);
    background: var(--color-warning-soft);
  }

  .deps-runtime-panel__blocked strong {
    color: var(--color-warning);
  }

  .deps-runtime-panel__progress {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-warning);
    font-family: var(--font-mono);
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
    border: 1px solid color-mix(in srgb, var(--color-danger) 30%, transparent);
    color: var(--color-danger);
  }

  .deps-banner--success {
    background: var(--color-success-soft);
    border: 1px solid color-mix(in srgb, var(--color-success) 30%, transparent);
    color: var(--color-success);
  }

  .deps-banner--warning {
    background: var(--color-warning-soft);
    border: 1px solid color-mix(in srgb, var(--color-warning) 30%, transparent);
    color: var(--color-warning);
  }

  .deps-banner__message {
    flex: 1;
  }

  /* UV status */
  .deps-uv-status {
    padding: var(--space-2) 0;
  }

  .deps-uv-status__text {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    font-family: var(--font-mono);
  }

  .deps-uv-status__text--warn {
    color: var(--color-warning);
  }

  .deps-uv-status__text--info {
    color: var(--color-accent);
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
    color: var(--color-text-muted);
  }

  /* Progress bar */
  .deps-progress {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .deps-progress__track {
    flex: 1;
    min-width: 0;
  }

  .deps-progress__label {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
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
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    background: var(--color-surface);
  }

  .deps-row--failed {
    border-color: color-mix(in srgb, var(--color-warning) 30%, transparent);
    background: var(--color-warning-soft);
  }

  /* Holds a 16px ActionIcon: font-size no longer aligns anything, so the box
     centres the glyph and pins itself to the first line of the name. */
  .deps-row__icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 24px;
    flex: 0 0 auto;
  }

  .deps-row__icon--installed {
    color: var(--color-success);
  }

  .deps-row__icon--missing {
    color: var(--color-danger);
  }

  .deps-row__icon--failed {
    color: var(--color-warning);
  }

  .deps-row__icon--neutral {
    color: var(--color-text-muted);
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
    color: var(--color-text-secondary);
    margin: 0;
  }

  .deps-row__progress {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-1);
  }

  .deps-row__progress-track {
    flex: none;
    width: 120px;
  }

  .deps-row__progress-pct {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  /* Sizes the link-style Button, which takes its text size from here. */
  .deps-row__error-toggle {
    align-self: flex-start;
    font-size: var(--font-size-xs);
  }

  .deps-row__error-detail {
    font-size: var(--font-size-2xs);
    font-family: var(--font-mono);
    background: var(--color-surface-sunken);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-sm);
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

  /* A version is machine text: mono inside the badge. */
  .deps-row__version {
    font-family: var(--font-mono);
  }

  /* Empty state */
  .deps-empty {
    font-size: var(--font-size-sm);
    color: var(--color-text-muted);
    text-align: center;
    padding: var(--space-6) 0;
  }

  /* Models and servers section */
  .deps-section-title {
    margin: var(--space-2) 0 0;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
  }

  .deps-section-hint {
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--color-text-muted);
  }

  /* Danger zone */
  .deps-danger-zone {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
    border: 1px solid color-mix(in srgb, var(--color-danger) 30%, transparent);
    border-radius: var(--radius-md);
    background: var(--color-danger-soft);
  }

  .deps-danger-zone__hint {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    margin: 0;
    flex: 1;
  }

  .deps-reset-confirmation__phrase {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .deps-reset-confirmation__phrase code {
    padding: var(--space-0-5) var(--space-1);
    border-radius: var(--radius-sm);
    background: var(--color-danger-soft);
    color: var(--color-danger);
    font-family: var(--font-mono);
  }
</style>
