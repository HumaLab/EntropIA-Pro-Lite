/**
 * Microsoft Store update check for EntropIA Lite.
 *
 * The backend owns every rule (variant, platform, package identity, the
 * six-hour cache and the single query per session); this is only the bridge.
 */

import { invoke } from '@tauri-apps/api/core'

export type StoreUpdateStatus = 'available' | 'up_to_date' | 'skipped' | 'unavailable'

/** The Lite Store listing. Mirrors `STORE_PRODUCT_URI` in `store_updates.rs`. */
export const MICROSOFT_STORE_PRODUCT_URI = 'ms-windows-store://pdp/?ProductId=9N328K9L95JD'

export function checkMicrosoftStoreUpdate(): Promise<StoreUpdateStatus> {
  return invoke<StoreUpdateStatus>('check_microsoft_store_update')
}
