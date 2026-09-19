/**
 * Approximate term matching: over the corpus index ({@link pickVariants}) and
 * over small hand-written texts such as notes ({@link matchText}).
 *
 * # What problem this solves
 *
 * The corpus is OCR output. A name the source spells correctly on every page
 * reaches the index as `sindicato` 269 times and as `sindigato`, `sinicato` or
 * `bindicato` a handful of times each. An exact search misses exactly the
 * pages a historian is least likely to find any other way.
 *
 * # Why the vocabulary, and why frequency decides
 *
 * Variants are drawn from the index's own vocabulary (`fts_items_vocab`), so
 * nothing is suggested that no document contains, and proper names absent
 * from any dictionary are covered for free.
 *
 * Edit distance alone is not enough: `pescado` is one edit from `pesado`, and
 * both are real words. What separates a misreading from a different word is
 * weight. Measured on a real corpus, an OCR error is far rarer than the word
 * it corrupts, while a different word — or a plural — is of comparable
 * weight. So a variant is kept only when it is clearly rarer than the term
 * (a misreading of it), or clearly more common (the term is itself the
 * misspelling, typed by the writer or read by the OCR). Everything of similar
 * weight is left alone.
 */

/** A variant must differ in document frequency by at least this factor. */
const WEIGHT_RATIO = 4

/** Misreadings one term may add. Past this the query drowns in noise. */
const MAX_MISREADINGS = 8

/**
 * The same folding the `unicode61 remove_diacritics 1` tokenizer applies, so a
 * query term can be looked up in the vocabulary it produced.
 */
export function normalizeTerm(term: string): string {
  return term.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase()
}

/**
 * How many edits a term tolerates. Short terms none — one edit turns `mar`
 * into `mas` — and nothing with a digit, because dates, folios and signatures
 * are only useful exact.
 */
export function maxEditsFor(term: string): number {
  if (/\d/.test(term)) return 0
  if (term.length < 5) return 0
  if (term.length < 8) return 1
  return 2
}

/**
 * Optimal-string-alignment distance: substitutions, insertions, deletions and
 * adjacent transpositions each cost one. Returns `max + 1` as soon as the
 * distance is known to exceed `max`, which is what keeps a scan over the
 * whole vocabulary cheap.
 */
export function editDistance(a: string, b: string, max: number): number {
  if (Math.abs(a.length - b.length) > max) return max + 1

  let before: number[] | null = null
  let previous = Array.from({ length: b.length + 1 }, (_, j) => j)

  for (let i = 1; i <= a.length; i++) {
    const current = [i]
    let rowMin = i
    for (let j = 1; j <= b.length; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1
      let value = Math.min(previous[j]! + 1, current[j - 1]! + 1, previous[j - 1]! + cost)
      if (before && i > 1 && j > 1 && a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]) {
        value = Math.min(value, before[j - 2]! + 1)
      }
      current.push(value)
      if (value < rowMin) rowMin = value
    }
    if (rowMin > max) return max + 1
    before = previous
    previous = current
  }

  return previous[b.length]!
}

interface Candidate {
  term: string
  distance: number
  docs: number
}

function byCloseness(x: Candidate, y: Candidate): number {
  return x.distance - y.distance || y.docs - x.docs || x.term.localeCompare(y.term)
}

/**
 * The vocabulary terms worth searching alongside `term`.
 *
 * `vocab` maps each indexed term to the number of documents holding it.
 */
export function pickVariants(term: string, vocab: ReadonlyMap<string, number>): string[] {
  const needle = normalizeTerm(term)
  const max = maxEditsFor(needle)
  if (max === 0) return []

  const weight = vocab.get(needle) ?? 0
  const misreadings: Candidate[] = []
  const corrections: Candidate[] = []

  for (const [candidate, docs] of vocab) {
    if (candidate === needle || Math.abs(candidate.length - needle.length) > max) continue
    const distance = editDistance(needle, candidate, max)
    if (distance > max) continue

    if (docs * WEIGHT_RATIO <= weight) {
      misreadings.push({ term: candidate, distance, docs })
    } else if (docs >= Math.max(2, weight * WEIGHT_RATIO)) {
      corrections.push({ term: candidate, distance, docs })
    }
  }

  // Only the best-supported correction: a typo has one intended word, and a
  // second candidate is usually a different word that happens to be close.
  const correction = corrections.sort(byCloseness).slice(0, 1)
  const kept = misreadings.sort(byCloseness).slice(0, MAX_MISREADINGS)
  return [...correction, ...kept].map((candidate) => candidate.term)
}

/** How a text answered a query: as written, only through typos, or not at all. */
export type TextMatch = 'exact' | 'approximate' | null

/**
 * Whether `text` holds every word of `query`, in any order, ignoring case and
 * accents.
 *
 * A word matches as written when the text contains it, even inside a longer
 * word — searching `molin` still finds `molineros`, as the literal search this
 * replaces always did. Failing that, it matches with a typo on either side
 * when some word of the text is within the edits {@link maxEditsFor} allows.
 *
 * For small, hand-written texts such as notes, where every candidate can be
 * read. It is not an index: over the corpus, use the FTS vocabulary instead.
 */
export function matchText(query: string, text: string): TextMatch {
  const terms = normalizeTerm(query).split(/\s+/).filter(Boolean)
  if (terms.length === 0) return null

  const haystack = normalizeTerm(text)
  let words: string[] | null = null
  let approximate = false

  for (const term of terms) {
    if (haystack.includes(term)) continue
    const max = maxEditsFor(term)
    if (max === 0) return null
    words ??= haystack.split(/[^\p{L}\p{N}]+/u).filter(Boolean)
    if (!words.some((word) => editDistance(term, word, max) <= max)) return null
    approximate = true
  }

  return approximate ? 'approximate' : 'exact'
}
