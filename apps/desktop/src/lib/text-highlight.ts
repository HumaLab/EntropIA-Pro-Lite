/**
 * Where words occur in a text, as offsets into that exact text.
 *
 * Matching folds case and accents the way the search index does, so what the
 * search found is what gets marked: `zarate` marks `ZÁRATE`. The offsets are
 * into the original string, not the folded one, because they become DOM ranges
 * over the text as displayed. Folding goes one character at a time for that
 * reason — each original character maps to its own folded run, so a position
 * in the folded text always leads back to a position in the original.
 *
 * Whole words only: a variant like `crocitto` is one token of the index, and
 * marking it inside `crocittos` would claim a match the search never made.
 */

function fold(char: string): string {
  return char.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase()
}

const WORD = /[\p{L}\p{N}]/u

export function wordRanges(text: string, words: string[]): Array<[number, number]> {
  const wanted = new Set(words.map((word) => fold(word)).filter(Boolean))
  if (wanted.size === 0 || !text) return []

  const ranges: Array<[number, number]> = []
  let start = -1
  let folded = ''
  const close = (end: number) => {
    if (start >= 0 && wanted.has(folded)) ranges.push([start, end])
    start = -1
    folded = ''
  }

  let index = 0
  for (const char of text) {
    if (WORD.test(char)) {
      if (start < 0) start = index
      folded += fold(char)
    } else {
      close(index)
    }
    index += char.length
  }
  close(index)
  return ranges
}
