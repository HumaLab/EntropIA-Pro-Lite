import { describe, expect, it } from 'vitest'
import { wordRanges } from './text-highlight'

const cut = (text: string, ranges: Array<[number, number]>) =>
  ranges.map(([start, end]) => text.slice(start, end))

describe('wordRanges', () => {
  it('finds each occurrence of a word, as whole words', () => {
    const text = 'Crocitto dijo que Crocitto no era crocittos'
    expect(cut(text, wordRanges(text, ['crocitto']))).toEqual(['Crocitto', 'Crocitto'])
  })

  it('ignores case and accents, and returns offsets into the original text', () => {
    const text = 'La reunión en ZÁRATE'
    const ranges = wordRanges(text, ['zarate', 'reunion'])
    expect(cut(text, ranges)).toEqual(['reunión', 'ZÁRATE'])
  })

  it('returns ranges in reading order, without overlaps', () => {
    const text = 'sindigato y sindicato'
    expect(wordRanges(text, ['sindicato', 'sindigato', 'sindicato'])).toEqual([
      [0, 9],
      [12, 21],
    ])
  })

  it('finds nothing for no words or no text', () => {
    expect(wordRanges('algo', [])).toEqual([])
    expect(wordRanges('', ['algo'])).toEqual([])
  })
})
