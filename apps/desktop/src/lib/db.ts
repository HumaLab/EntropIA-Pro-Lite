import { initStore, type StoreApi } from '@entropia/store'
import { createTauriDbClient } from './tauri-db-client'
import { ensureSyncCapture } from '$lib/sync'

let _store: StoreApi | null = null

export async function initDb(): Promise<void> {
  _store = await initStore(createTauriDbClient())
  // Sync capture bootstrap (DESIGN §6.1): now that every migration has run and
  // all synced tables exist, ensure the sync schema + capture triggers. The
  // backend already ran this at setup for tables that existed then; this covers
  // tables created by the JS migrations. Best-effort — never block app init.
  try {
    await ensureSyncCapture()
  } catch (error) {
    console.error('[sync] ensureSyncCapture failed:', error)
  }
}

export function getStore(): StoreApi {
  if (!_store) throw new Error('Store not initialized. Call initDb() first.')
  return _store
}
