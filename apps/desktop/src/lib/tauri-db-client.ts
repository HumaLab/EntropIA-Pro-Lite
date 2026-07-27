import { invoke } from '@tauri-apps/api/core'
import type { DbClient } from '@entropia/store'

/**
 * Low-level database adapter for the desktop app.
 *
 * This is intentionally kept outside `@entropia/store` so the store package
 * stays transport-agnostic and depends only on its DbClient port.
 */
export const createTauriDbClient = (): DbClient => ({
  async execute(sql, params = []) {
    console.log('[db] execute:', sql.slice(0, 50), '...')
    const result = await invoke<{ rowsAffected: number }>('db_execute', { sql, params })
    console.log('[db] execute done, rowsAffected:', result.rowsAffected)
    return result
  },

  async executeBatch(sql: string) {
    console.log('[db] executeBatch:', sql.slice(0, 100), '...')
    await invoke('db_execute_batch', { sql })
    console.log('[db] executeBatch done')
  },

  async executeTransaction(statements) {
    await invoke('db_execute_transaction', { statements })
  },

  async select<T = Record<string, unknown>>(sql: string, params: unknown[] = []) {
    console.log('[db] select:', sql.slice(0, 50), '...')
    const result = await invoke<T[]>('db_select', { sql, params })
    console.log('[db] select done, rows:', result.length)
    return result
  },

  async selectRows(sql: string, params: unknown[] = []) {
    return await invoke<unknown[][]>('db_select_rows', { sql, params })
  },
})
