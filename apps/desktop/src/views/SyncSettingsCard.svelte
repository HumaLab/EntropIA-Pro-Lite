<script lang="ts">
  /**
   * Cloud sync settings card (DESIGN §11). Lives in the SettingsView `api` tab.
   * Binds EXCLUSIVELY to the `sync_*` commands (DESIGN §11): it NEVER calls
   * `settingsGet`/`settingsSet` — sync config lives only in `sync_meta` + the
   * keyring. The device token never crosses this boundary (DESIGN §8).
   *
   * Surface: fixed EntropIA Cloud endpoint, email/password with Registrar/Iniciar
   * sesión/Cerrar sesión, device list with revoke (ConfirmDialog),
   * auto-sync toggle + interval, "Sincronizar ahora", storage usage, compact conflicts
   * summary + logs notification, "Borrar mis datos del servidor" (password ConfirmDialog),
   * "Re-verificar archivos", and a first-sync preflight confirm when pending blob
   * bytes exceed 500 MB.
   */
  import { onMount, onDestroy } from 'svelte'
  import { locale, t } from '$lib/i18n'
  import {
    DEFAULT_SYNC_SERVER_URL,
    describeSyncError,
    syncAckConflict,
    syncDeleteAccount,
    syncGetUsage,
    syncListConflicts,
    syncListDevices,
    syncListPlans,
    syncLogin,
    syncLogout,
    syncNow,
    syncRegisterAccount,
    syncRequestPlanChange,
    syncReverifyBlobs,
    syncRevokeDevice,
    syncSetAuto,
    type PlanCatalogItem,
    type SyncConflict,
    type SyncDevice,
    type SyncStatus,
    type SyncUsage,
  } from '$lib/sync'
  import { syncStore } from '$lib/sync-store'
  import { appendLog } from '$lib/logs'
  import { ActionIcon, Button, Card, ConfirmDialog, Input } from '@entropia/ui'

  // First-sync preflight threshold (DESIGN §11): 500 MB of pending blob bytes.
  const PREFLIGHT_THRESHOLD_BYTES = 500 * 1024 * 1024

  const currentLocale = locale

  // ── Live status (from the module-level store) ──
  let status = $state<SyncStatus>(syncStore.status)
  const unsubscribe = syncStore.subscribe((next) => {
    status = next
  })

  const loggedIn = $derived(status.state !== 'disabled')

  const disabledStatus: SyncStatus = {
    state: 'disabled',
    last_sync_at: null,
    pending: 0,
    blobs_pending: 0,
    pending_blob_bytes: 0,
    conflicts: 0,
    clock_warning: false,
  }

  // ── Session form ──
  let email = $state('')
  let password = $state('')
  let showPassword = $state(false)

  // ── Auto-sync ──
  let autoEnabled = $state(true)
  let autoInterval = $state('5')

  // ── Remote data ──
  let devices = $state<SyncDevice[]>([])
  let usage = $state<SyncUsage | null>(null)
  let conflicts = $state<SyncConflict[]>([])
  let pendingConflictCount = $derived(conflicts.filter((conflict) => !conflict.acknowledged).length)
  const loggedConflictIds = new Set<string>()

  // ── Async / feedback flags ──
  let busy = $state<'register' | 'login' | 'logout' | 'sync' | 'reverify' | 'auto' | null>(null)
  let feedback = $state<{ tone: 'success' | 'error'; text: string } | null>(null)

  // ── Dialog state ──
  let deviceToRevoke = $state<SyncDevice | null>(null)
  let revoking = $state(false)
  let showDeleteAccount = $state(false)
  let deletePassword = $state('')
  let deleting = $state(false)
  let pendingPreflightBytes = $state<number | null>(null)

  // ── Plan change request (NOTIFICATIONS.md §1) ──
  let showPlanModal = $state(false)
  let plans = $state<PlanCatalogItem[]>([])
  let plansLoading = $state(false)
  let plansError = $state<string | null>(null)
  let selectedPlanId = $state('')
  let planNote = $state('')
  let requestingPlan = $state(false)
  let planRequestError = $state<string | null>(null)
  // Locally-tracked pending request (seeds from usage, then updates on submit/409).
  let pendingPlanRequest = $state<string | null>(null)

  // Target plans = catalogue minus the current plan; server already sorts ASC.
  const targetPlans = $derived(plans.filter((p) => !p.is_current))
  const currentPlanName = $derived(
    plans.find((p) => p.is_current)?.name ?? usage?.plan_name ?? '—'
  )

  // ── Validation ──
  const passwordValid = $derived(password.length >= 10)
  const canRegister = $derived(Boolean(email.trim() && passwordValid) && busy === null)
  const canLogin = $derived(Boolean(email.trim() && password) && busy === null)

  onMount(() => {
    void syncStore.initialize()
    if (loggedIn) {
      void refreshAll()
    }
  })

  onDestroy(() => {
    unsubscribe()
  })

  function setError(error: unknown) {
    feedback = { tone: 'error', text: describeSyncError(error) }
  }

  function setSuccess(text: string) {
    feedback = { tone: 'success', text }
  }

  function formatBytes(bytes: number): string {
    if (bytes <= 0) return '0 B'
    const units = ['B', 'KB', 'MB', 'GB', 'TB']
    const exp = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1)
    const value = bytes / 1024 ** exp
    return `${value.toFixed(exp === 0 ? 0 : 1)} ${units[exp]}`
  }

  function formatWhen(ms: number): string {
    if (!ms) return '—'
    return new Date(ms).toLocaleString()
  }

  async function refreshAll() {
    await Promise.all([refreshDevices(), refreshUsage(), refreshConflicts()])
  }

  async function refreshDevices() {
    try {
      devices = collapseDevices(await syncListDevices())
    } catch (error) {
      setError(error)
    }
  }

  /**
   * Collapses historical duplicate device rows down to a single current state per
   * physical device. Repeated logout/login cycles can produce different device ids
   * with the same visible identity (name + platform); the UI should show the best
   * representative instead of the whole session history.
   */
  function collapseDevices(rows: SyncDevice[]): SyncDevice[] {
    const normalize = (value: string | null | undefined): string =>
      (value ?? '').trim().toLowerCase()
    const keyOf = (device: SyncDevice): string => {
      const name = normalize(device.name)
      const platform = normalize(device.platform)
      return name || platform ? `${name}|${platform}` : `id:${device.id}`
    }
    const priority = (device: SyncDevice): number => {
      if (device.current) return 2
      if (!device.revoked) return 1
      return 0
    }
    const newer = (a: SyncDevice, b: SyncDevice): SyncDevice => {
      if (a.last_seen_at !== b.last_seen_at) return a.last_seen_at > b.last_seen_at ? a : b
      return a.created_at >= b.created_at ? a : b
    }

    const groups = new Map<string, SyncDevice>()
    for (const row of rows) {
      const key = keyOf(row)
      const existing = groups.get(key)
      if (!existing) {
        groups.set(key, row)
        continue
      }

      const incomingPriority = priority(row)
      const existingPriority = priority(existing)
      if (incomingPriority > existingPriority) {
        groups.set(key, row)
      } else if (incomingPriority === existingPriority) {
        groups.set(key, newer(row, existing))
      }
    }

    return Array.from(groups.values())
  }

  async function refreshUsage() {
    try {
      usage = await syncGetUsage()
      // Seed the persistent "request under review" banner from usage (§1).
      pendingPlanRequest = usage.pending_plan_request ?? null
    } catch (error) {
      setError(error)
    }
  }

  async function refreshConflicts() {
    try {
      conflicts = await syncListConflicts(50, 0)
      await notifyConflictsInLogs(conflicts)
    } catch (error) {
      setError(error)
    }
  }

  function conflictLogMessage(conflict: SyncConflict): string {
    const header = t('sync.card.conflictLogMessage', {
      reason: conflict.reason,
      table: conflict.table_name,
      row: conflict.row_id,
    })
    const details = [conflict.winner_summary, conflict.loser_payload].filter(Boolean).join('\n\n')
    return details ? `${header}\n${details}` : header
  }

  async function notifyConflictsInLogs(rows: SyncConflict[]) {
    const pending = rows.filter((conflict) => !conflict.acknowledged)
    for (const conflict of pending) {
      if (loggedConflictIds.has(conflict.id)) continue
      loggedConflictIds.add(conflict.id)
      try {
        await appendLog('warn', 'sync/conflicts', conflictLogMessage(conflict))
      } catch {
        // Logging is diagnostic only; conflict refresh must not fail because Logs are unavailable.
      }
    }
  }

  async function handleRegister() {
    if (!canRegister) return
    busy = 'register'
    feedback = null
    try {
      await syncRegisterAccount(DEFAULT_SYNC_SERVER_URL, email.trim(), password)
      setSuccess(t('sync.card.registered'))
    } catch (error) {
      setError(error)
    } finally {
      busy = null
    }
  }

  async function handleLogin() {
    if (!canLogin) return
    busy = 'login'
    feedback = null
    try {
      await syncLogin(DEFAULT_SYNC_SERVER_URL, email.trim(), password)
      password = ''
      syncStore.setStatus({ ...status, state: 'idle' })
      await refreshAll()
      setSuccess(t('sync.card.loggedInAs', { email: email.trim() }))
    } catch (error) {
      setError(error)
    } finally {
      busy = null
    }
  }

  async function handleLogout() {
    busy = 'logout'
    feedback = null
    try {
      await syncLogout()
      devices = []
      usage = null
      conflicts = []
      syncStore.setStatus(disabledStatus)
    } catch (error) {
      setError(error)
    } finally {
      busy = null
    }
  }

  /**
   * Manual sync. On a first sync with a large pending-blob estimate (> 500 MB),
   * pops the preflight confirm BEFORE kicking off the run (DESIGN §11). Once the
   * user confirms (or the estimate is below the threshold) the run proceeds.
   */
  async function handleSyncNow(skipPreflight = false) {
    if (
      !skipPreflight &&
      !status.last_sync_at &&
      status.pending_blob_bytes > PREFLIGHT_THRESHOLD_BYTES
    ) {
      pendingPreflightBytes = status.pending_blob_bytes
      return
    }
    pendingPreflightBytes = null
    busy = 'sync'
    feedback = null
    try {
      const next = await syncNow()
      syncStore.setStatus(next)
    } catch (error) {
      setError(error)
    } finally {
      busy = null
    }
  }

  function cancelPreflight() {
    pendingPreflightBytes = null
  }

  async function confirmPreflight() {
    await handleSyncNow(true)
  }

  async function handleAutoSyncChange() {
    busy = 'auto'
    feedback = null
    const interval = Number(autoInterval.trim() || '5')
    try {
      await syncSetAuto(autoEnabled, Number.isFinite(interval) && interval >= 1 ? interval : 5)
    } catch (error) {
      setError(error)
    } finally {
      busy = null
    }
  }

  function requestRevoke(device: SyncDevice) {
    deviceToRevoke = device
  }

  async function confirmRevoke() {
    if (!deviceToRevoke) return
    revoking = true
    try {
      await syncRevokeDevice(deviceToRevoke.id)
      deviceToRevoke = null
      await refreshDevices()
    } catch (error) {
      setError(error)
    } finally {
      revoking = false
    }
  }

  async function handleAckAllConflicts() {
    try {
      await Promise.all(
        conflicts
          .filter((conflict) => !conflict.acknowledged)
          .map((conflict) => syncAckConflict(conflict.id))
      )
      await refreshConflicts()
      await syncStore.refresh()
    } catch (error) {
      setError(error)
    }
  }

  async function handleReverify() {
    busy = 'reverify'
    feedback = null
    try {
      await syncReverifyBlobs()
      setSuccess(t('sync.card.reverifyDone'))
    } catch (error) {
      setError(error)
    } finally {
      busy = null
    }
  }

  async function confirmDeleteAccount() {
    if (!deletePassword) return
    deleting = true
    try {
      await syncDeleteAccount(deletePassword)
      showDeleteAccount = false
      deletePassword = ''
      devices = []
      usage = null
      conflicts = []
      await syncStore.refresh()
      setSuccess(t('sync.card.deleteAccountDone'))
    } catch (error) {
      setError(error)
    } finally {
      deleting = false
    }
  }

  function cancelDeleteAccount() {
    showDeleteAccount = false
    deletePassword = ''
  }

  // ── Plan change request ──

  type PlanLabel = { name: string; quota: string; bytes: number }

  const FREE_PLAN_LABEL: PlanLabel = { name: 'Free', quota: '100 MB', bytes: 100 * 1024 ** 2 }

  const PLAN_LABELS: PlanLabel[] = [
    FREE_PLAN_LABEL,
    { name: 'Go', quota: '5 GB', bytes: 5 * 1024 ** 3 },
    { name: 'Pro 1', quota: '10 GB', bytes: 10 * 1024 ** 3 },
    { name: 'Pro 2', quota: '20 GB', bytes: 20 * 1024 ** 3 },
    { name: 'Max 1', quota: '50 GB', bytes: 50 * 1024 ** 3 },
    { name: 'Max 2', quota: '100 GB', bytes: 100 * 1024 ** 3 },
  ]

  function canonicalPlanLabel(plan: Pick<PlanCatalogItem, 'name' | 'quota_bytes' | 'price_cents'>) {
    if (plan.price_cents === 0 || plan.name.toLowerCase() === 'free') return FREE_PLAN_LABEL
    const match = PLAN_LABELS.find((label) => {
      const delta = Math.abs(plan.quota_bytes - label.bytes)
      return delta / label.bytes < 0.05
    })
    return match ?? { name: plan.name, quota: formatBytes(plan.quota_bytes), bytes: plan.quota_bytes }
  }

  /** Builds the human label for a target plan option: "Go · 5 GB" / "Pro 1 · 10 GB". */
  function planOptionLabel(plan: PlanCatalogItem): string {
    const label = canonicalPlanLabel(plan)
    return t('sync.upgrade.planOption', { name: label.name, quota: label.quota })
  }

  async function openPlanModal() {
    showPlanModal = true
    planRequestError = null
    plansError = null
    selectedPlanId = ''
    planNote = ''
    plansLoading = true
    try {
      plans = await syncListPlans()
    } catch {
      plansError = t('sync.upgrade.loadPlansError')
    } finally {
      plansLoading = false
    }
  }

  function closePlanModal() {
    showPlanModal = false
  }

  async function submitPlanRequest() {
    if (!selectedPlanId || requestingPlan) return
    requestingPlan = true
    planRequestError = null
    const note = planNote.trim()
    try {
      await syncRequestPlanChange(selectedPlanId, note || undefined)
      pendingPlanRequest = plans.find((p) => p.id === selectedPlanId)?.name ?? selectedPlanId
      showPlanModal = false
    } catch (error) {
      // A 409 means a request is already in review: surface the message AND flip the
      // persistent banner so the button is disabled (the request already exists).
      const message = describeSyncError(error)
      const raw = typeof error === 'string' ? error : error instanceof Error ? error.message : ''
      if (raw.toLowerCase().includes('plan_request_pending') || raw.includes('409')) {
        pendingPlanRequest =
          plans.find((p) => p.id === selectedPlanId)?.name ?? pendingPlanRequest ?? selectedPlanId
        showPlanModal = false
      } else {
        planRequestError = message
      }
    } finally {
      requestingPlan = false
    }
  }
</script>

<Card>
  <section class="settings-card-section settings-card-section--vertical sync-card">
    <div class="settings-card-section__copy">
      <h2>{$currentLocale ? t('sync.card.title') : 'Sincronización en la nube'}</h2>
      <p>{t('sync.card.description')}</p>
    </div>

    {#if feedback}
      <p
        class="surface-message settings__feedback"
        class:surface-message--error={feedback.tone === 'error'}
        class:surface-message--success={feedback.tone === 'success'}
        role={feedback.tone === 'error' ? 'alert' : 'status'}
      >
        {feedback.text}
      </p>
    {/if}

    {#if !loggedIn}
      <!-- ── Session form (logged out) ── -->
      <div class="sync-card__form">
        <Input
          label={t('sync.card.emailLabel')}
          type="email"
          bind:value={email}
          placeholder={t('sync.card.emailPlaceholder')}
        />

        <div class="sync-card__password">
          <Input
            label={t('sync.card.passwordLabel')}
            type={showPassword ? 'text' : 'password'}
            bind:value={password}
            placeholder={t('sync.card.passwordPlaceholder')}
          />
          <button
            class="sync-card__password-toggle"
            type="button"
            onclick={() => (showPassword = !showPassword)}
            aria-label={showPassword ? t('settings.hideApiKey') : t('settings.showApiKey')}
            title={showPassword ? t('settings.hideApiKey') : t('settings.showApiKey')}
          >
            <ActionIcon name={showPassword ? 'eye-off' : 'eye'} size={15} />
          </button>
        </div>
      </div>

      <div class="settings__button-row sync-card__actions">
        <Button variant="primary" onclick={handleLogin} disabled={!canLogin}>
          {busy === 'login' ? t('sync.card.loggingIn') : t('sync.card.login')}
        </Button>
        <Button variant="secondary" onclick={handleRegister} disabled={!canRegister}>
          {busy === 'register' ? t('sync.card.registering') : t('sync.card.register')}
        </Button>
      </div>
    {:else}
      <div class="sync-card__grid">
      <!-- ── Session block (logged in) ── -->
      <div class="sync-card__block">
        <h3>{t('sync.card.sessionTitle')}</h3>
        <div class="settings__button-row sync-card__actions">
          <Button variant="primary" onclick={() => handleSyncNow()} disabled={busy !== null}>
            {busy === 'sync' || status.state === 'syncing'
              ? t('sync.card.syncing')
              : t('sync.card.syncNow')}
          </Button>
          <Button variant="secondary" onclick={handleLogout} disabled={busy !== null}>
            {busy === 'logout' ? t('sync.card.loggingOut') : t('sync.card.logout')}
          </Button>
        </div>
      </div>

      <!-- ── Auto-sync ── -->
      <div class="sync-card__block">
        <h3>{t('sync.card.autoSyncTitle')}</h3>
        <label class="sync-card__check">
          <input type="checkbox" bind:checked={autoEnabled} onchange={handleAutoSyncChange} />
          <span>{t('sync.card.autoSyncToggle')}</span>
        </label>
        <div class="sync-card__interval">
          <label class="sync-card__label" for="sync-auto-interval">
            {t('sync.card.autoSyncInterval')}
          </label>
          <input
            id="sync-auto-interval"
            type="number"
            min="1"
            class="sync-card__number-input"
            bind:value={autoInterval}
            onchange={handleAutoSyncChange}
          />
        </div>
      </div>

      <!-- ── Devices ── -->
      <div class="sync-card__block">
        <div class="sync-card__block-head">
          <h3>{t('sync.card.devicesTitle')}</h3>
          <Button variant="secondary" size="sm" onclick={refreshDevices}>
            {t('sync.card.refreshDevices')}
          </Button>
        </div>
        {#if devices.length === 0}
          <p class="settings__hint">{t('sync.card.devicesEmpty')}</p>
        {:else}
          <ul class="sync-card__list">
            {#each devices as device (device.id)}
              <li class="sync-card__device">
                <div class="sync-card__device-info">
                  <span class="sync-card__device-name">
                    {device.name || device.platform}
                    {#if device.current}
                      <span class="sync-card__tag">{t('sync.card.deviceCurrent')}</span>
                    {/if}
                    {#if device.revoked}
                      <span class="sync-card__tag sync-card__tag--muted">
                        {t('sync.card.deviceRevoked')}
                      </span>
                    {/if}
                  </span>
                  <span class="settings__hint">
                    {t('sync.card.deviceLastSeen', { when: formatWhen(device.last_seen_at) })}
                  </span>
                </div>
                {#if !device.current && !device.revoked}
                  <Button variant="secondary" size="sm" onclick={() => requestRevoke(device)}>
                    {t('sync.card.deviceRevoke')}
                  </Button>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      </div>

      <!-- ── Storage usage ── -->
      <div class="sync-card__block">
        <div class="sync-card__block-head">
          <h3>{t('sync.card.usageTitle')}</h3>
          <Button variant="secondary" size="sm" onclick={refreshUsage}>
            {t('sync.card.refreshUsage')}
          </Button>
        </div>
        {#if usage}
          <ul class="sync-card__usage">
            {#if usage.plan_name}
              <li>{t('sync.card.usagePlan', { plan: usage.plan_name })}</li>
            {/if}
            <li>{t('sync.card.usageRows', { count: usage.rows })}</li>
            <li>
              {t('sync.card.usageBlobs', {
                count: usage.blobs_count,
                size: formatBytes(usage.blobs_bytes),
              })}
            </li>
            <li>
              {usage.quota_bytes > 0
                ? t('sync.card.usageQuota', {
                    used: formatBytes(usage.blobs_bytes),
                    total: formatBytes(usage.quota_bytes),
                  })
                : t('sync.card.usageUnlimited', { used: formatBytes(usage.blobs_bytes) })}
            </li>
          </ul>
        {/if}

        <!-- Plan change: a REQUEST (reviewed by an operator), not a checkout (§1). -->
        <div class="sync-card__plan-action">
          {#if pendingPlanRequest}
            <p class="surface-message sync-card__plan-pending" role="status">
              {t('sync.upgrade.pendingPlan', { plan: pendingPlanRequest })}
            </p>
            <Button variant="secondary" size="sm" disabled>
              {t('sync.upgrade.pending')}
            </Button>
          {:else}
            <Button variant="secondary" size="sm" onclick={openPlanModal} disabled={busy !== null}>
              {t('sync.upgrade.button')}
            </Button>
          {/if}
        </div>
      </div>

      <!-- ── Conflicts: compact summary, details are emitted to Logs ── -->
      <div class="sync-card__conflict-summary" aria-live="polite">
        <div>
          <strong>{t('sync.card.conflictsTitle')}</strong>
          <p class="settings__hint">
            {pendingConflictCount === 0
              ? t('sync.card.conflictsEmpty')
              : t('sync.card.conflictsSummary', {
                  count: pendingConflictCount,
                })}
          </p>
        </div>
        <div class="sync-card__conflict-actions">
          <Button variant="secondary" size="sm" onclick={refreshConflicts}>
            {t('sync.card.refreshConflicts')}
          </Button>
          {#if pendingConflictCount > 0}
            <Button variant="secondary" size="sm" onclick={handleAckAllConflicts}>
              {t('sync.card.conflictAckAll')}
            </Button>
          {/if}
        </div>
      </div>
      </div>

      <!-- ── Danger zone ── -->
      <div class="sync-card__block sync-card__block--danger">
        <h3>{t('sync.card.dangerTitle')}</h3>
        <div class="sync-card__danger-row">
          <div class="settings__field--stacked">
            <Button variant="secondary" onclick={handleReverify} disabled={busy !== null}>
              {t('sync.card.reverifyBlobs')}
            </Button>
            <p class="settings__hint">{t('sync.card.reverifyBlobsHint')}</p>
          </div>
          <div class="settings__field--stacked">
            <Button
              variant="secondary"
              onclick={() => (showDeleteAccount = true)}
              disabled={busy !== null}
            >
              {t('sync.card.deleteAccount')}
            </Button>
            <p class="settings__hint">{t('sync.card.deleteAccountHint')}</p>
          </div>
        </div>
      </div>
    {/if}
  </section>
</Card>

{#if deviceToRevoke}
  <ConfirmDialog
    title={t('sync.card.revokeDeviceTitle')}
    titleId="sync-revoke-device-title"
    message={t('sync.card.revokeDeviceMessage', {
      name: deviceToRevoke.name || deviceToRevoke.platform,
    })}
    cancelLabel={t('sync.card.revokeDeviceCancel')}
    confirmLabel={t('sync.card.revokeDeviceConfirm')}
    variant="destructive"
    confirming={revoking}
    oncancel={() => (deviceToRevoke = null)}
    onconfirm={confirmRevoke}
  />
{/if}

{#if showDeleteAccount}
  <ConfirmDialog
    title={t('sync.card.deleteAccountTitle')}
    titleId="sync-delete-account-title"
    message={t('sync.card.deleteAccountMessage')}
    cancelLabel={t('sync.card.deleteAccountCancel')}
    confirmLabel={t('sync.card.deleteAccountConfirm')}
    variant="destructive"
    confirming={deleting}
    confirmDisabled={!deletePassword}
    oncancel={cancelDeleteAccount}
    onconfirm={confirmDeleteAccount}
  >
    <Input
      label={t('sync.card.passwordLabel')}
      type="password"
      bind:value={deletePassword}
      placeholder={t('sync.card.passwordPlaceholder')}
    />
  </ConfirmDialog>
{/if}

{#if pendingPreflightBytes !== null}
  <ConfirmDialog
    title={t('sync.preflight.title')}
    titleId="sync-preflight-title"
    message={t('sync.preflight.message', { size: formatBytes(pendingPreflightBytes) })}
    cancelLabel={t('sync.preflight.cancel')}
    confirmLabel={t('sync.preflight.confirm')}
    oncancel={cancelPreflight}
    onconfirm={confirmPreflight}
  />
{/if}

{#if showPlanModal}
  <ConfirmDialog
    title={t('sync.upgrade.title')}
    titleId="sync-plan-change-title"
    message={t('sync.upgrade.currentPlanLabel') + ': ' + currentPlanName}
    cancelLabel={t('sync.upgrade.cancel')}
    confirmLabel={requestingPlan ? t('sync.upgrade.submitting') : t('sync.upgrade.submit')}
    confirmDisabled={!selectedPlanId || plansLoading}
    confirming={requestingPlan}
    error={planRequestError ?? plansError}
    oncancel={closePlanModal}
    onconfirm={submitPlanRequest}
  >
    <div class="sync-plan-modal">
      <div class="sync-plan-modal__field">
        <label class="sync-card__label" for="sync-plan-target">
          {t('sync.upgrade.targetPlanLabel')}
        </label>
        <select
          id="sync-plan-target"
          class="sync-card__number-input sync-plan-modal__select"
          bind:value={selectedPlanId}
          disabled={plansLoading || targetPlans.length === 0}
        >
          <option value="" disabled>{t('sync.upgrade.targetPlanPlaceholder')}</option>
          {#each targetPlans as plan (plan.id)}
            <option value={plan.id}>{planOptionLabel(plan)}</option>
          {/each}
        </select>
      </div>

      <div class="sync-plan-modal__field">
        <label class="sync-card__label" for="sync-plan-note">
          {t('sync.upgrade.noteLabel')}
        </label>
        <textarea
          id="sync-plan-note"
          class="sync-card__number-input sync-plan-modal__textarea"
          bind:value={planNote}
          placeholder={t('sync.upgrade.notePlaceholder')}
          rows="3"
        ></textarea>
      </div>

      <p class="sync-plan-modal__disclaimer">{t('sync.upgrade.disclaimer')}</p>
    </div>
  </ConfirmDialog>
{/if}

<style>
  /* Self-sufficient layout: SyncSettingsCard is a separate component, so the
     `settings-*` classes scoped to SettingsView do NOT reach it. Re-declare the
     layout/typography it relies on here, using the shared design tokens, so the
     card matches the rest of the Settings surface regardless of where it mounts.
     Form fields use the @entropia/ui <Input>, which also inherits SettingsView's
     `:global(.input-field__input)` theming when rendered inside the api tab. */
  .sync-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  .settings-card-section__copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .settings-card-section__copy h2 {
    margin: 0;
    font-size: var(--font-size-base);
    font-weight: var(--font-weight-semibold);
    letter-spacing: -0.01em;
  }

  .settings-card-section__copy p {
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    line-height: 1.6;
  }

  .settings__feedback {
    margin: 0;
  }

  .settings__hint {
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    line-height: 1.6;
  }

  .settings__button-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .settings__field--stacked {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .sync-card__form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  /* Password field: the shared <Input> has no trailing slot, so overlay the
     show/hide toggle and reserve room for it on the right of the input box. */
  .sync-card__password {
    position: relative;
  }

  .sync-card__password :global(.input-field__input) {
    padding-right: calc(var(--control-height-md) + var(--space-2));
  }

  .sync-card__password-toggle {
    position: absolute;
    right: 0;
    bottom: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    width: var(--control-height-md);
    height: var(--control-height-md);
    border: none;
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
    border-radius: var(--radius-control);
    transition: color var(--transition-base);
  }

  .sync-card__password-toggle:hover {
    color: var(--color-text-primary);
  }

  .sync-card__password-toggle:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  /* Numeric interval field: <Input> doesn't forward onchange, so keep a raw
     input but mirror the component's token-based styling exactly. */
  .sync-card__label {
    display: block;
    margin-bottom: var(--space-2);
    font-family: var(--font-sans);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
  }

  .sync-card__number-input {
    width: 100%;
    min-height: var(--control-height-md);
    padding: 0 var(--space-3);
    font-family: var(--font-sans);
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
    background-color: color-mix(in srgb, var(--color-surface-glass) 78%, transparent);
    border: 1px solid color-mix(in srgb, var(--color-hairline) 78%, transparent);
    border-radius: var(--radius-input);
    outline: none;
    box-sizing: border-box;
    transition:
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth),
      background-color var(--transition-smooth);
  }

  .sync-card__number-input:focus,
  .sync-card__number-input:focus-visible {
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
    background-color: color-mix(in srgb, var(--color-surface-glass) 88%, transparent);
  }

  .sync-card__grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: var(--space-4) var(--space-5);
    align-items: start;
  }

  .sync-card__block {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding-top: var(--space-3);
    border-top: 1px solid color-mix(in srgb, var(--color-hairline) 60%, transparent);
  }

  .sync-card__block h3 {
    margin: 0;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
  }

  .sync-card__block-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .sync-card__actions {
    margin-top: 0;
  }

  .sync-card__check {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
  }

  .sync-card__interval {
    max-width: 240px;
  }

  .sync-card__list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .sync-card__device,
  .sync-card__conflict-summary {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .sync-card__device-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .sync-card__device-name {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
  }

  .sync-card__tag {
    padding: 0 var(--space-2);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--color-accent) 14%, transparent);
    color: var(--color-accent);
    font-size: var(--font-size-2xs);
  }

  .sync-card__tag--muted {
    background: color-mix(in srgb, var(--color-text-muted) 14%, transparent);
    color: var(--color-text-muted);
  }

  .sync-card__usage {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding-left: var(--space-4);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
  }

  .sync-card__conflict-summary {
    padding: var(--space-3) 0;
    border-top: 1px solid color-mix(in srgb, var(--color-hairline) 58%, transparent);
    border-bottom: 1px solid color-mix(in srgb, var(--color-hairline) 58%, transparent);
  }

  .sync-card__conflict-summary strong,
  .sync-card__conflict-summary p {
    margin: 0;
  }

  .sync-card__conflict-actions {
    display: flex;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  .sync-card__plan-action {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
    margin-top: var(--space-2);
  }

  .sync-card__plan-pending {
    margin: 0;
  }

  .sync-plan-modal {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .sync-plan-modal__field {
    display: flex;
    flex-direction: column;
  }

  .sync-plan-modal__select {
    appearance: auto;
  }

  .sync-plan-modal__textarea {
    min-height: calc(var(--control-height-md) * 2);
    padding-top: var(--space-2);
    padding-bottom: var(--space-2);
    line-height: 1.5;
    resize: vertical;
  }

  .sync-plan-modal__disclaimer {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--color-accent) 8%, transparent);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    line-height: 1.5;
  }

  .sync-card__block--danger {
    border-top-color: color-mix(in srgb, var(--color-danger) 30%, transparent);
  }

  .sync-card__danger-row {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: var(--space-4);
  }

  @media (max-width: 860px) {
    .sync-card__grid {
      grid-template-columns: 1fr;
    }

    .sync-card__conflict-summary {
      align-items: flex-start;
      flex-direction: column;
    }
  }
</style>
