/**
 * One-time Pro offer to switch the cloud modes an archive inherited from Lite
 * to the local engines (R6). The backend decides what to offer
 * (`local_modes_offer.rs`); the switch goes through `settingsSet`, the same
 * path the Settings view uses.
 */

import { invoke } from '@tauri-apps/api/core'
import { settingsSet } from './settings'

const OFF = import.meta.env.VITE_LOCAL_ML !== '1'

/** Recorded once the offer is answered, so it never shows again. */
export const LOCAL_MODES_OFFERED_KEY = 'pro_local_modes_offered'

export type OfferedModeKey = 'stt_mode' | 'ocrh_mode' | 'llm_mode'

export interface ModeOffer {
  key: OfferedModeKey
  current: string
  local: string
}

/** The modes to offer, empty when there is nothing to ask. */
export function localModesOffer(): Promise<ModeOffer[]> {
  if (OFF) return Promise.resolve([])
  return invoke<ModeOffer[]>('local_modes_offer')
}

/** Switch every offered mode to local, then close the offer. */
export async function acceptLocalModes(offer: ModeOffer[]): Promise<void> {
  for (const mode of offer) {
    await settingsSet(mode.key, mode.local)
  }
  await settingsSet(LOCAL_MODES_OFFERED_KEY, '1')
}

/** Keep the current modes and close the offer. */
export function dismissLocalModes(): Promise<void> {
  return settingsSet(LOCAL_MODES_OFFERED_KEY, '1')
}
