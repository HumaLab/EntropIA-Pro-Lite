/**
 * Accent- and case-insensitive text matching shared by the lists that filter
 * in the webview (the Zotero tab's held library). The corpus side folds in
 * Rust; this is the same contract for text already in memory.
 */

/** Lowercase without diacritics: `Producción` and `produccion` are one word. */
export function foldText(text: string): string {
  return text
    .normalize('NFD')
    .replace(/\p{M}+/gu, '')
    .toLowerCase()
}

function words(folded: string): string[] {
  return folded.split(/[^\p{L}\p{N}]+/u).filter(Boolean)
}

/** Edit distance with an early exit once it exceeds `limit`. */
function withinDistance(a: string, b: string, limit: number): boolean {
  if (Math.abs(a.length - b.length) > limit) return false
  let previous = Array.from({ length: b.length + 1 }, (_, i) => i)
  for (let i = 1; i <= a.length; i++) {
    const row = [i]
    let best = i
    for (let j = 1; j <= b.length; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1
      const value = Math.min(previous[j]! + 1, row[j - 1]! + 1, previous[j - 1]! + cost)
      row.push(value)
      if (value < best) best = value
    }
    if (best > limit) return false
    previous = row
  }
  return previous[b.length]! <= limit
}

/** Typos allowed in a word: none up to 4 letters, one up to 7 letters, then two. */
function allowedTypos(length: number): number {
  return length < 5 ? 0 : length <= 7 ? 1 : 2
}

/**
 * Whether `haystack` answers `query`. Both sides are folded. Exact mode asks
 * for the query as a phrase; approximate mode asks for every query word to be
 * in the text, each allowed a few typos (exact containment still counts).
 */
export function matchesQuery(haystack: string, query: string, fuzzy: boolean): boolean {
  const needle = foldText(query).trim()
  if (!needle) return true
  const text = foldText(haystack)
  if (text.includes(needle)) return true
  if (!fuzzy) return false
  const held = words(text)
  return words(needle).every((word) => {
    if (text.includes(word)) return true
    const typos = allowedTypos(word.length)
    return typos > 0 && held.some((candidate) => withinDistance(word, candidate, typos))
  })
}
