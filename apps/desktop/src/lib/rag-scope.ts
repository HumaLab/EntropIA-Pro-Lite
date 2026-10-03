import { t, type I18nKey } from './i18n'
import type { RagBibliographyLocation, RagLibraryRef, RagSource } from './rag'

/**
 * Pure helpers of the research chat's scope (Corpus / Biblioteca / Ambos):
 * what a source is, how its location reads, which message a backend notice
 * means, how the library choice changes, and the window of text the passage
 * reader shows. No state, no invoke: the view and the store compose these.
 */

export function isBibliographySource(source: RagSource): boolean {
  return Boolean(source.bibliography)
}

/** The label key of the scope a source came from. */
export function sourceScopeKey(source: RagSource): I18nKey {
  return isBibliographySource(source) ? 'ragChat.scopeBiblioteca' : 'ragChat.scopeCorpus'
}

/**
 * "p. 3", "pp. 3–4", "párr. 2" or "párr. 2–3". A PDF is cited by page and an
 * HTML snapshot by paragraph: the snapshot is stored as a single page 1, and
 * saying "p. 1" there would send the reader looking for a page that is not.
 */
export function locationText(location: RagBibliographyLocation | null | undefined): string {
  if (!location) return ''
  const single = location.from === location.to
  const params = { n: location.from, from: location.from, to: location.to }
  if (location.kind === 'paragraphs') {
    return t(single ? 'ragChat.locationParagraph' : 'ragChat.locationParagraphs', params)
  }
  return t(single ? 'ragChat.locationPage' : 'ragChat.locationPages', params)
}

/** "authors · year", skipping what is missing. */
export function workLine(work: { authors: string; year: number | null }): string {
  return [work.authors.trim(), work.year === null ? '' : String(work.year)]
    .filter(Boolean)
    .join(' · ')
}

/**
 * The message key for the backend's reason the Biblioteca leg found nothing.
 * An unknown code is still said, as the generic failure: silence would read
 * as "searched and found nothing".
 */
export function bibliographyNoticeKey(code: string | null | undefined): I18nKey | null {
  if (!code) return null
  switch (code) {
    case 'no_library_synced':
      return 'ragChat.biblioNotice.noLibrarySynced'
    case 'no_embeddings':
      return 'ragChat.biblioNotice.noEmbeddings'
    case 'embedding_unavailable':
      return 'ragChat.biblioNotice.embeddingUnavailable'
    default:
      return 'ragChat.biblioNotice.failed'
  }
}

/** User 0 and group 0 are different libraries: the type is part of the key. */
export function libraryChoiceKey(library: RagLibraryRef): string {
  return `${library.libraryType}:${library.libraryId}`
}

/**
 * The library choice after toggling one. `null` means "every synced library",
 * which is also what checking them all again returns to; the selection is
 * never empty (the last checked library stays checked) because an empty
 * choice would silently mean "search nothing".
 */
export function selectedLibrariesAfterToggle(
  current: RagLibraryRef[] | null,
  all: RagLibraryRef[],
  toggled: RagLibraryRef
): RagLibraryRef[] | null {
  const key = libraryChoiceKey(toggled)
  const selected = current ?? all
  const isChecked = selected.some((library) => libraryChoiceKey(library) === key)
  const next = isChecked
    ? selected.filter((library) => libraryChoiceKey(library) !== key)
    : [...selected, toggled]
  if (next.length === 0) return current
  return next.length >= all.length ? null : next
}

export interface PassageSegment {
  text: string
  marked: boolean
}

export interface PassageWindow {
  segments: PassageSegment[]
  truncatedBefore: boolean
  truncatedAfter: boolean
}

/**
 * The part of a page's text the reader shows: the cited range(s) marked, with
 * `radius` characters of context on each side. Offsets are Unicode scalars,
 * as the backend counts them (`Array.from`, not UTF-16 `length`), and a page
 * can be megabytes for an HTML snapshot, so the window is bounded.
 */
export function passageWindow(
  text: string,
  ranges: Array<[number, number]>,
  radius: number
): PassageWindow {
  const chars = Array.from(text)
  const merged: Array<[number, number]> = []
  for (const [rawStart, rawEnd] of [...ranges].sort((a, b) => a[0] - b[0])) {
    const start = Math.max(0, rawStart)
    const end = Math.min(chars.length, rawEnd)
    if (end <= start) continue
    const last = merged[merged.length - 1]
    if (last && start <= last[1]) last[1] = Math.max(last[1], end)
    else merged.push([start, end])
  }
  if (merged.length === 0) {
    const end = Math.min(chars.length, radius)
    return {
      segments: end > 0 ? [{ text: chars.slice(0, end).join(''), marked: false }] : [],
      truncatedBefore: false,
      truncatedAfter: end < chars.length,
    }
  }
  const first = merged[0]
  const last = merged[merged.length - 1]
  if (!first || !last) return { segments: [], truncatedBefore: false, truncatedAfter: false }
  const lo = Math.max(0, first[0] - radius)
  const hi = Math.min(chars.length, last[1] + radius)
  const segments: PassageSegment[] = []
  let cursor = lo
  const push = (to: number, marked: boolean) => {
    if (to > cursor) segments.push({ text: chars.slice(cursor, to).join(''), marked })
    cursor = Math.max(cursor, to)
  }
  for (const [start, end] of merged) {
    push(start, false)
    push(end, true)
  }
  push(hi, false)
  return { segments, truncatedBefore: lo > 0, truncatedAfter: hi < chars.length }
}
