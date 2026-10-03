import type { RagSource } from './rag'

/**
 * Where a corpus citation of the research chat should land (opens the item at
 * the cited fragment, the way writing citations already do).
 *
 * A source carries `provenance.startChar..endChar`, counted in Unicode chars by
 * the Rust retrieval, over the text row named by `sourceKind/sourceId`. That
 * range is only trusted when the row still exists under the same id AND the
 * words at that range are still the quoted snippet. Otherwise the snippet
 * itself is looked up in the current text, and highlighted only when it is
 * found exactly once: highlighting an ambiguous or missing fragment would point
 * at text nobody cited, which is worse than opening the page unmarked.
 */

export interface RagCitationRange {
  start: number
  end: number
  text: string
}

export interface RagCitationTarget {
  /** Raw-text range in UTF-16 offsets (what the viewer's highlighter expects). */
  citationRange: RagCitationRange | null
  /** Where to seek an audio asset; null for sources without a timestamp. */
  citationSeconds: number | null
}

interface TextRow {
  id?: string
  textContent: string | null
}

export interface RagCitationStore {
  extractions: { findByAsset(assetId: string): Promise<TextRow | null> }
  transcriptions: { findByAsset(assetId: string): Promise<TextRow | null> }
}

function usableText(row: TextRow | null): string | null {
  const text = row?.textContent ?? null
  return text && text.trim() ? text : null
}

/** The row the range was computed over; extraction wins when none is named. */
async function readSourceRow(store: RagCitationStore, source: RagSource): Promise<TextRow | null> {
  const kind = source.provenance?.sourceKind
  if (kind === 'transcription') return store.transcriptions.findByAsset(source.assetId)
  if (kind === 'extraction') return store.extractions.findByAsset(source.assetId)
  const extracted = await store.extractions.findByAsset(source.assetId)
  if (usableText(extracted)) return extracted
  return store.transcriptions.findByAsset(source.assetId)
}

/** Rust counts Unicode chars; JS strings index UTF-16 code units. */
function charOffsetToUtf16(text: string, charOffset: number): number | null {
  if (!Number.isInteger(charOffset) || charOffset < 0) return null
  let units = 0
  let chars = 0
  while (chars < charOffset) {
    if (units >= text.length) return null
    const code = text.codePointAt(units)!
    units += code > 0xffff ? 2 : 1
    chars += 1
  }
  return units
}

function provenanceRange(text: string, source: RagSource): RagCitationRange | null {
  const provenance = source.provenance
  if (!provenance) return null
  const start = charOffsetToUtf16(text, provenance.startChar)
  const end = charOffsetToUtf16(text, provenance.endChar)
  if (start === null || end === null || end <= start) return null
  return text.slice(start, end) === source.snippet ? { start, end, text: source.snippet } : null
}

/** Collapses whitespace runs, keeping for each kept char its source offset. */
function normalizeWithMap(text: string): { normalized: string; map: number[] } {
  let normalized = ''
  const map: number[] = []
  let pendingSpace = false
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index]!
    if (/\s/.test(char)) {
      pendingSpace = normalized.length > 0
      continue
    }
    if (pendingSpace) {
      normalized += ' '
      map.push(index)
      pendingSpace = false
    }
    normalized += char
    map.push(index)
  }
  return { normalized, map }
}

function locateSnippet(text: string, snippet: string): RagCitationRange | null {
  const trimmed = snippet.trim()
  if (!trimmed) return null

  const first = text.indexOf(trimmed)
  if (first !== -1) {
    if (text.indexOf(trimmed, first + 1) !== -1) return null
    return { start: first, end: first + trimmed.length, text: trimmed }
  }

  const haystack = normalizeWithMap(text)
  const needle = normalizeWithMap(trimmed).normalized
  if (!needle) return null
  const at = haystack.normalized.indexOf(needle)
  if (at === -1 || haystack.normalized.indexOf(needle, at + 1) !== -1) return null
  const start = haystack.map[at]!
  const end = haystack.map[at + needle.length - 1]! + 1
  return { start, end, text: text.slice(start, end) }
}

export async function resolveRagCitation(
  store: RagCitationStore,
  source: RagSource
): Promise<RagCitationTarget> {
  const citationSeconds =
    typeof source.startSeconds === 'number' && Number.isFinite(source.startSeconds)
      ? Math.max(0, source.startSeconds)
      : null
  try {
    const row = await readSourceRow(store, source)
    const text = usableText(row)
    if (text === null) return { citationRange: null, citationSeconds }

    const sameRow =
      !source.provenance || row?.id === undefined ? true : row.id === source.provenance.sourceId
    const exact = sameRow ? provenanceRange(text, source) : null
    return { citationRange: exact ?? locateSnippet(text, source.snippet), citationSeconds }
  } catch {
    return { citationRange: null, citationSeconds }
  }
}
