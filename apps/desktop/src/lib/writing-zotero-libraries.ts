import { invoke } from '@tauri-apps/api/core'

/**
 * The Zotero tab's library list (E1c-2, UI half).
 *
 * The single authority for *which libraries are offered*: what the backend
 * knows (`writing_zotero_known_libraries` — personal always first, catalog
 * names winning over the mirror) merged with what the person added by hand,
 * kept in localStorage. Selection itself stays in `writing-zotero.ts`
 * (E1c-1 semantics unchanged): this module names and lists, the store selects.
 */

export type ZoteroLibraryType = 'user' | 'group'

export type KnownLibrarySource = 'personal' | 'mirror' | 'catalog'

/** One library the backend offers without asking Zotero. */
export interface KnownLibrary {
  libraryType: ZoteroLibraryType
  libraryId: string
  name: string | null | undefined
  source: KnownLibrarySource
}

/** Whether Zotero answers for one library. A state, never "closed". */
export type CheckLibrary =
  | { status: 'available'; version?: number | null }
  | { status: 'unverifiable' }
  | { status: 'not_found' }

/** One library added by hand, as persisted. */
export interface AddedLibrary {
  libraryType: ZoteroLibraryType
  libraryId: string
  /** True when Zotero could not confirm it; shown as a factual hint. */
  unverified: boolean
}

/** One offered entry: known data winning, manual data filling the gaps. */
export interface LibraryOption {
  libraryType: ZoteroLibraryType
  libraryId: string
  name: string | null | undefined
  source: KnownLibrarySource | 'added'
  unverified: boolean
}

export type LibrarySelection = Pick<LibraryOption, 'libraryType' | 'libraryId'>

/** Stable key for the hand-added list. */
export const ADDED_LIBRARIES_STORAGE_KEY = 'entropia:zotero:added-libraries'

const PERSONAL_OPTION: LibraryOption = {
  libraryType: 'user',
  libraryId: '0',
  name: null,
  source: 'personal',
  unverified: false,
}

function isLibraryType(value: unknown): value is ZoteroLibraryType {
  return value === 'user' || value === 'group'
}

/**
 * Reads a hand-added list out of a raw stored value.
 *
 * Anything may be in localStorage — an older build, a hand edit — so anything
 * unrecognised is skipped rather than failing. Ids are trimmed; a blank id is
 * not a library.
 */
export function parseAddedLibraries(raw: string | null): AddedLibrary[] {
  if (!raw) return []
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return []
  }
  if (!Array.isArray(parsed)) return []
  const kept: AddedLibrary[] = []
  for (const entry of parsed) {
    if (typeof entry !== 'object' || entry === null) continue
    const { libraryType, libraryId, unverified } = entry as Record<string, unknown>
    if (!isLibraryType(libraryType)) continue
    if (typeof libraryId !== 'string') continue
    const id = libraryId.trim()
    if (!id) continue
    kept.push({ libraryType, libraryId: id, unverified: unverified === true })
  }
  return kept
}

/** The hand-added list as stored, or empty when storage is unusable. */
export function readAddedLibraries(): AddedLibrary[] {
  try {
    return parseAddedLibraries(localStorage.getItem(ADDED_LIBRARIES_STORAGE_KEY))
  } catch {
    // Storage can be unavailable outright: the known list still offers.
    return []
  }
}

/** Remembers the hand-added list. Best effort, like every localStorage write. */
export function writeAddedLibraries(added: AddedLibrary[]): void {
  try {
    localStorage.setItem(ADDED_LIBRARIES_STORAGE_KEY, JSON.stringify(added))
  } catch {
    // Unavailable storage forgets the manual entries for this session only.
  }
}

function keyOf(libraryType: string, libraryId: string): string {
  return `${libraryType}/${libraryId}`
}

/**
 * Merges the known list with the hand-added one.
 *
 * The personal library is always offered first with no name, even knowing
 * nothing else. A known entry wins over a manual duplicate for the same
 * `(type, id)` — the backend's name and source, and no unverified hint — and
 * `user/1` and `group/1` stay different libraries. The rest follows the
 * backend order: type then id ascending.
 */
export function mergeLibraries(known: KnownLibrary[], added: AddedLibrary[]): LibraryOption[] {
  const byKey = new Map<string, LibraryOption>()
  const safeKnown = Array.isArray(known) ? known : []
  for (const entry of safeKnown) {
    if (!entry || !isLibraryType(entry.libraryType)) continue
    if (typeof entry.libraryId !== 'string' || !entry.libraryId.trim()) continue
    if (entry.libraryType === 'user' && entry.libraryId === '0') continue
    byKey.set(keyOf(entry.libraryType, entry.libraryId), {
      libraryType: entry.libraryType,
      libraryId: entry.libraryId,
      name: entry.name ?? null,
      source: entry.source === 'catalog' || entry.source === 'mirror' ? entry.source : 'mirror',
      unverified: false,
    })
  }
  const safeAdded = Array.isArray(added) ? added : []
  for (const entry of safeAdded) {
    if (!entry || !isLibraryType(entry.libraryType)) continue
    if (typeof entry.libraryId !== 'string' || !entry.libraryId.trim()) continue
    if (entry.libraryType === 'user' && entry.libraryId === '0') continue
    const key = keyOf(entry.libraryType, entry.libraryId)
    if (byKey.has(key)) continue
    byKey.set(key, {
      libraryType: entry.libraryType,
      libraryId: entry.libraryId,
      name: null,
      source: 'added',
      unverified: entry.unverified === true,
    })
  }
  const rest = [...byKey.values()].sort((a, b) =>
    a.libraryType === b.libraryType
      ? a.libraryId.localeCompare(b.libraryId, 'en', { numeric: true })
      : a.libraryType.localeCompare(b.libraryType)
  )
  return [{ ...PERSONAL_OPTION }, ...rest]
}

/**
 * Names one entry: the catalog name when present, `personalLabel` for user/0,
 * and a derived `user/{id}` / `group/{id}` label otherwise. The unverified
 * hint is composed by the caller, next to this label, never inside it.
 */
export function libraryLabel(option: LibraryOption, personalLabel: string): string {
  if (option.name) return option.name
  if (option.libraryType === 'user' && option.libraryId === '0') return personalLabel
  return `${option.libraryType}/${option.libraryId}`
}

/**
 * Loads the offered list: one backend call, merged with the hand-added
 * entries. A backend that cannot be asked still leaves personal plus manual.
 */
export async function loadLibraries(): Promise<LibraryOption[]> {
  let known: KnownLibrary[] = []
  try {
    const answer = await invoke<unknown>('writing_zotero_known_libraries')
    if (Array.isArray(answer)) known = answer as KnownLibrary[]
  } catch {
    known = []
  }
  return mergeLibraries(known, readAddedLibraries())
}

export type AddLibraryError = 'invalid' | 'not_found' | 'check_failed'

export type AddLibraryOutcome =
  | { ok: true; selected: LibrarySelection; libraries: LibraryOption[] }
  | { ok: false; error: AddLibraryError }

/**
 * Checks a hand-typed library before adding it.
 *
 * A blank id is inline validation without asking Zotero. `available` adds as
 * verified; `unverifiable` adds too with the hint kept (Zotero closed is not
 * an error per the E1c contract); `not_found` and a failed check add nothing.
 */
export async function addLibraryChecked(
  libraryType: ZoteroLibraryType,
  libraryId: string
): Promise<AddLibraryOutcome> {
  if (!isLibraryType(libraryType)) return { ok: false, error: 'invalid' }
  const id = typeof libraryId === 'string' ? libraryId.trim() : ''
  if (!id) return { ok: false, error: 'invalid' }

  let check: CheckLibrary
  try {
    check = await invoke<CheckLibrary>('writing_zotero_check_library', {
      libraryType,
      libraryId: id,
    })
  } catch {
    return { ok: false, error: 'check_failed' }
  }
  if (!check || typeof check !== 'object' || typeof (check as { status?: unknown }).status !== 'string') {
    return { ok: false, error: 'check_failed' }
  }
  if (check.status === 'not_found') return { ok: false, error: 'not_found' }
  if (check.status !== 'available' && check.status !== 'unverifiable') {
    return { ok: false, error: 'check_failed' }
  }

  const added = readAddedLibraries()
  const at = added.findIndex((e) => e.libraryType === libraryType && e.libraryId === id)
  const unverified = check.status === 'unverifiable'
  if (at >= 0) {
    added[at] = { libraryType, libraryId: id, unverified }
  } else {
    added.push({ libraryType, libraryId: id, unverified })
  }
  writeAddedLibraries(added)

  const libraries = await loadLibraries()
  return { ok: true, selected: { libraryType, libraryId: id }, libraries }
}
