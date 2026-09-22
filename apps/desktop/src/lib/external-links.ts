import { invoke } from '@tauri-apps/api/core'
import { MICROSOFT_STORE_PRODUCT_URI } from './store-updates'

export function normalizeExternalUrl(rawUrl: string): string {
  // Exact equality, not parsing: the backend accepts this one string as-is.
  if (rawUrl === MICROSOFT_STORE_PRODUCT_URI) return rawUrl
  const url = new URL(rawUrl)
  if (url.protocol !== 'https:' && url.protocol !== 'http:') {
    throw new Error('Only HTTP(S) URLs can be opened externally.')
  }
  return url.href
}

export async function openExternalUrl(rawUrl: string): Promise<void> {
  await invoke('open_external_url', { url: normalizeExternalUrl(rawUrl) })
}

export async function openExternalUrlFromClick(event: MouseEvent, rawUrl: string): Promise<void> {
  event.preventDefault()
  await openExternalUrl(rawUrl)
}
