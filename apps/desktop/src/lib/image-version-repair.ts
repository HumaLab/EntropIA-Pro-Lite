import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

/**
 * Maintenance "review image versions": images whose asset shows an older
 * version than the one their layout was computed on.
 *
 * The backend decides everything (`image_version_repair.rs`): the scan only
 * reads, and the apply judges every confirmed proposal again before writing
 * all of them in one transaction. Nothing here sends a path: a proposal is
 * confirmed with its asset id and the token the scan gave it.
 */

/** One file of a proposal, as the person is shown it. */
export interface RepairFile {
  fileName: string
  /** `N` of `<stem>_vN`; 0 for the original. */
  version: number
  width: number | null
  height: number | null
}

/** A proposal to show the version the layout belongs to. */
export interface PathRepair {
  assetId: string
  itemId: string
  /** The item's title, or the file name when it has none. */
  name: string
  shown: RepairFile
  proposed: RepairFile
  token: string
}

/** A proposal to record, on an old layout, the file it was computed from. */
export interface SourceCompletion {
  assetId: string
  name: string
  fileName: string
  token: string
}

/** How many assets the review left in each state. */
export interface RepairCounts {
  scanned: number
  matching: number
  repairable: number
  needsLayoutRerun: number
  ambiguous: number
  noCandidate: number
  candidateAfterLayout: number
  fileMissing: number
}

export interface RepairScan {
  counts: RepairCounts
  repairs: PathRepair[]
  completions: SourceCompletion[]
}

export interface RepairApplied {
  repaired: { itemId: string; assetId: string; path: string }[]
  completed: number
}

export interface RepairScanProgress {
  done: number
  total: number
}

type Confirmed = { assetId: string; token: string }

const confirmed = ({ assetId, token }: Confirmed): Confirmed => ({ assetId, token })

/**
 * Reviews every image that has a layout. Reads only. `onProgress` hears how
 * far the review is while it runs.
 */
export async function scanImageVersionRepairs(
  onProgress?: (progress: RepairScanProgress) => void
): Promise<RepairScan> {
  let unlisten: UnlistenFn | null = null
  if (onProgress) {
    // Progress is a courtesy: a review that cannot report it still runs.
    unlisten = await listen<RepairScanProgress>('image-version-repairs:progress', (event) =>
      onProgress(event.payload)
    ).catch(() => null)
  }
  try {
    return await invoke<RepairScan>('scan_image_version_repairs')
  } finally {
    unlisten?.()
  }
}

/** Applies the confirmed proposals: all of them, or none. */
export function applyImageVersionRepairs(
  repairs: PathRepair[],
  completions: SourceCompletion[]
): Promise<RepairApplied> {
  return invoke<RepairApplied>('apply_image_version_repairs', {
    repairs: repairs.map(confirmed),
    completions: completions.map(confirmed),
  })
}

/**
 * Whether an apply was refused because a proposal no longer holds: the archive
 * changed after the scan, and nothing was written.
 */
export function isStaleProposalError(error: unknown): boolean {
  const message = error instanceof Error ? error.message : String(error)
  return message.startsWith('stale_proposal')
}
