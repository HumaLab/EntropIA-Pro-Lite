import { invoke } from '@tauri-apps/api/core'
import type { AssetProcessEvent } from '@entropia/store'
import { getStore } from './db'

export type { AssetProcessEvent } from '@entropia/store'

/** Processes the renderer performs and reports; the backend refuses others. */
export type RendererProcess =
  | 'image_crop'
  | 'image_erase'
  | 'image_rotate'
  | 'image_rotate_fine'
  | 'pdf_crop'
  | 'pdf_erase'
  | 'pdf_rotate'
  | 'image_undo'
  | 'image_redo'
  | 'image_use_version'
  | 'duplicate'

export interface RendererProcessReport {
  assetId: string
  process: RendererProcess
  params?: Record<string, unknown>
  /** The asset's file before the process, hashed as its input. */
  previousPath?: string
}

/**
 * Adds a renderer-side process to the asset's history. Best effort: the edit
 * already happened, so a history failure is logged, never thrown.
 */
export async function recordAssetProcess(report: RendererProcessReport): Promise<void> {
  try {
    await invoke('asset_history_record', { report })
  } catch (error) {
    console.warn('[asset-history] Could not record', report.process, error)
  }
}

export async function getAssetProcessHistory(assetId: string): Promise<AssetProcessEvent[]> {
  return getStore().assetProcessEvents.findByAsset(assetId)
}

/** The JSON document the "Exportar" button saves: self-describing and stable. */
export function serializeAssetProcessHistory(
  asset: { id: string; path: string; type: string },
  events: AssetProcessEvent[],
  exportedAt = new Date()
) {
  return JSON.stringify(
    {
      format: 'entropia-asset-process-history',
      version: 1,
      exportedAt: exportedAt.toISOString(),
      asset: { id: asset.id, path: asset.path, type: asset.type },
      events: events.map((event) => ({
        process: event.process,
        tool: event.tool,
        toolVersion: event.toolVersion,
        model: event.model,
        modelVersion: event.modelVersion,
        appVersion: event.appVersion,
        params: event.params,
        inputSha256: event.inputSha256,
        outputSha256: event.outputSha256,
        at: new Date(event.createdAt).toISOString(),
      })),
    },
    null,
    2
  )
}
