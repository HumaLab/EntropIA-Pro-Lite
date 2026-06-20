import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { locale } from './i18n'
import {
  describeSyncError,
  ensureSyncCapture,
  SyncEventManager,
  syncAckConflict,
  syncDeleteAccount,
  syncDeleteNotification,
  syncGetUsage,
  syncListConflicts,
  syncListDevices,
  syncListNotifications,
  syncListPlans,
  syncLogin,
  syncLogout,
  syncMarkNotificationRead,
  syncNow,
  syncRegisterAccount,
  syncRequestPlanChange,
  syncReverifyBlobs,
  syncRevokeDevice,
  syncSetAuto,
  syncStatus,
  SYNC_STATUS_EVENT,
  type NotificationItem,
  type PlanCatalogItem,
  type SyncStatus,
} from './sync'

const mockInvoke = vi.mocked(invoke)

function status(overrides: Partial<SyncStatus> = {}): SyncStatus {
  return {
    state: 'idle',
    last_sync_at: null,
    pending: 0,
    blobs_pending: 0,
    pending_blob_bytes: 0,
    conflicts: 0,
    clock_warning: false,
    ...overrides,
  }
}

describe('sync.ts invoke wrappers', () => {
  beforeEach(() => {
    mockInvoke.mockReset().mockResolvedValue(undefined)
    locale.set('es')
  })

  it('maps camelCase args to the snake_case commands', async () => {
    mockInvoke.mockResolvedValue('acc-1')
    await ensureSyncCapture()
    expect(mockInvoke).toHaveBeenCalledWith('sync_ensure_capture')

    await syncRegisterAccount('https://sync.x', 'ana@x.com', 'supersecret1')
    expect(mockInvoke).toHaveBeenCalledWith('sync_register_account', {
      serverUrl: 'https://sync.x',
      email: 'ana@x.com',
      password: 'supersecret1',
    })

    await syncLogin('https://sync.x', 'ana@x.com', 'supersecret1')
    expect(mockInvoke).toHaveBeenCalledWith('sync_login', {
      serverUrl: 'https://sync.x',
      email: 'ana@x.com',
      password: 'supersecret1',
    })

    await syncLogout()
    expect(mockInvoke).toHaveBeenCalledWith('sync_logout')

    await syncStatus()
    expect(mockInvoke).toHaveBeenCalledWith('sync_status')

    await syncNow()
    expect(mockInvoke).toHaveBeenCalledWith('sync_now')

    await syncSetAuto(true, 7)
    expect(mockInvoke).toHaveBeenCalledWith('sync_set_auto', { enabled: true, intervalMin: 7 })

    await syncListDevices()
    expect(mockInvoke).toHaveBeenCalledWith('sync_list_devices')

    await syncRevokeDevice('dev-2')
    expect(mockInvoke).toHaveBeenCalledWith('sync_revoke_device', { deviceId: 'dev-2' })

    await syncListConflicts(25, 50)
    expect(mockInvoke).toHaveBeenCalledWith('sync_list_conflicts', { limit: 25, offset: 50 })

    await syncAckConflict('cf-1')
    expect(mockInvoke).toHaveBeenCalledWith('sync_ack_conflict', { conflictId: 'cf-1' })

    await syncGetUsage()
    expect(mockInvoke).toHaveBeenCalledWith('sync_get_usage')

    await syncDeleteAccount('supersecret1')
    expect(mockInvoke).toHaveBeenCalledWith('sync_delete_account', { password: 'supersecret1' })

    await syncReverifyBlobs()
    expect(mockInvoke).toHaveBeenCalledWith('sync_reverify_blobs')
  })

  it('maps the plan + notification commands (S2 contract)', async () => {
    mockInvoke.mockResolvedValue([])
    await syncListPlans()
    expect(mockInvoke).toHaveBeenCalledWith('sync_list_plans')

    mockInvoke.mockResolvedValue({
      id: 'req-1',
      current_plan_id: 'free',
      requested_plan_id: 'gb5',
      note: 'need space',
      status: 'pending',
      created_at: 1,
    })
    await syncRequestPlanChange('gb5', 'need space')
    // The argument crosses the wire as camelCase `requestedPlanId`.
    expect(mockInvoke).toHaveBeenCalledWith('sync_request_plan_change', {
      requestedPlanId: 'gb5',
      note: 'need space',
    })

    mockInvoke.mockResolvedValue([])
    await syncListNotifications('cursor-9', 50)
    expect(mockInvoke).toHaveBeenCalledWith('sync_list_notifications', {
      since: 'cursor-9',
      limit: 50,
    })

    mockInvoke.mockResolvedValue(undefined)
    await syncMarkNotificationRead('ntf-7')
    expect(mockInvoke).toHaveBeenCalledWith('sync_mark_notification_read', { id: 'ntf-7' })

    mockInvoke.mockResolvedValue(undefined)
    await syncDeleteNotification('ntf-7')
    expect(mockInvoke).toHaveBeenCalledWith('sync_delete_notification', { id: 'ntf-7' })
  })

  it('coerces non-array list returns to []', async () => {
    mockInvoke.mockResolvedValue(undefined)
    await expect(syncListPlans()).resolves.toEqual([])
    await expect(syncListNotifications()).resolves.toEqual([])
  })

  it('returns the plan catalogue and notification list verbatim', async () => {
    const plans: PlanCatalogItem[] = [
      {
        id: 'free',
        name: 'Free',
        quota_bytes: 0,
        price_cents: 0,
        currency: 'ARS',
        period: 'month',
        description: null,
        is_current: true,
      },
    ]
    mockInvoke.mockResolvedValue(plans)
    await expect(syncListPlans()).resolves.toEqual(plans)

    const notifs: NotificationItem[] = [
      {
        id: 'ntf-1',
        kind: 'plan',
        category: '',
        severity: 'info',
        title: 'Plan actualizado',
        body: 'Tu plan cambió.',
        created_at: 1_700_000_000,
        read_at: null,
      },
    ]
    mockInvoke.mockResolvedValue(notifs)
    await expect(syncListNotifications()).resolves.toEqual(notifs)
  })

  it('returns the account id from registration', async () => {
    mockInvoke.mockResolvedValue('acc-42')
    await expect(syncRegisterAccount('https://x', 'a@x', 'passwordpass')).resolves.toBe('acc-42')
  })

  it('returns the status snapshot from sync_now', async () => {
    const snapshot = status({ state: 'syncing', pending: 3 })
    mockInvoke.mockResolvedValue(snapshot)
    await expect(syncNow()).resolves.toEqual(snapshot)
  })
})

describe('describeSyncError', () => {
  beforeEach(() => {
    locale.set('es')
  })

  it('maps the stable error codes/statuses to human messages', () => {
    expect(describeSyncError('api error 426 (schema_upgrade_required): old')).toBe(
      'Actualizá la app: el servidor pide un esquema más nuevo.'
    )
    expect(describeSyncError('api error 507 (insufficient_storage): full')).toBe(
      'Almacenamiento del servidor lleno.'
    )
    expect(describeSyncError('api error 400 (clock_skew): drift')).toBe(
      'Revisá el reloj del dispositivo.'
    )
    expect(describeSyncError('api error 403 (registration_closed): closed')).toBe(
      'El registro está cerrado en este servidor.'
    )
    expect(describeSyncError('api error 409 (email_taken): taken')).toBe(
      'Ya existe una cuenta con ese email.'
    )
    expect(describeSyncError('api error 409 (plan_request_pending): in review')).toBe(
      'Ya tenés una solicitud de cambio de plan en revisión.'
    )
    expect(describeSyncError('api error 401 (unauthorized): nope')).toBe(
      'Credenciales inválidas o sesión revocada.'
    )
    expect(describeSyncError('api error 403 (account_suspended): blocked')).toBe(
      'Tu cuenta está suspendida. Contactá al administrador para reactivarla.'
    )
    expect(describeSyncError('api error 403 (subscription_expired): lapsed')).toBe(
      'Tu suscripción venció. Podés seguir descargando, pero no vas a poder subir cambios hasta renovarla.'
    )
  })

  it('maps in English too', () => {
    locale.set('en')
    expect(describeSyncError('api error 426 (schema_upgrade_required): old')).toBe(
      'Update the app: the server requires a newer schema.'
    )
    expect(describeSyncError('api error 403 (account_suspended): blocked')).toBe(
      'Your account is suspended. Contact the administrator to reactivate it.'
    )
    expect(describeSyncError('api error 403 (subscription_expired): lapsed')).toBe(
      'Your subscription has expired. You can still download, but you cannot upload changes until you renew.'
    )
  })

  it('falls back to the raw message, then a generic message', () => {
    expect(describeSyncError('network error: connection refused')).toBe(
      'network error: connection refused'
    )
    expect(describeSyncError('')).toBe('No se pudo completar la operación de sincronización.')
    expect(describeSyncError(new Error('boom'))).toBe('boom')
  })
})

describe('SyncEventManager', () => {
  it('forwards sync:status payloads and cleans up on stop', async () => {
    const received: SyncStatus[] = []
    const manager = new SyncEventManager((s) => received.push(s))

    const unlisten = vi.fn()
    let captured: ((e: { payload: unknown }) => void) | null = null
    const listen = vi.fn(async (event: string, cb: (e: { payload: unknown }) => void) => {
      expect(event).toBe(SYNC_STATUS_EVENT)
      captured = cb
      return unlisten
    })

    await manager.startListening(listen)
    expect(listen).toHaveBeenCalledTimes(1)

    captured!({ payload: status({ state: 'syncing' }) })
    expect(received).toHaveLength(1)
    expect(received.at(0)?.state).toBe('syncing')

    manager.stopListening()
    expect(unlisten).toHaveBeenCalledTimes(1)
  })

  it('unlistens a late registration if stopListening ran first (generation guard)', async () => {
    const manager = new SyncEventManager(() => {})
    const unlisten = vi.fn()

    // listen resolves only after we have already called stopListening.
    let resolveListen!: (fn: () => void) => void
    const listen = vi.fn(
      () =>
        new Promise<() => void>((resolve) => {
          resolveListen = resolve
        })
    )

    const startPromise = manager.startListening(listen)
    // Stop BEFORE the listen() promise resolves: the generation advances.
    manager.stopListening()
    resolveListen(unlisten)
    await startPromise

    // The late registration is unlistened immediately, not leaked.
    expect(unlisten).toHaveBeenCalledTimes(1)
  })
})
