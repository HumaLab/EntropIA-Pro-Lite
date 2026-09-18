import { describe, expect, it } from 'vitest'
import { documentStats } from './document-stats'

/**
 * The Export tab's counts: what a writer checks against a word limit before
 * sending a manuscript out.
 *
 * The body is what is counted, as a word processor counts it: footnotes are a
 * number of their own, not words added to the text. What the page shows is
 * what is counted — a citation atom reads as its quotation or rendering on
 * screen, so it counts as that.
 */

type N = { type: string; text?: string; attrs?: Record<string, unknown>; content?: N[] }
const doc = (...content: N[]): N => ({ type: 'doc', content })
const p = (...content: N[]): N => ({ type: 'paragraph', content })
const h = (...content: N[]): N => ({ type: 'heading', attrs: { level: 1 }, content })
const text = (value: string): N => ({ type: 'text', text: value })

describe('an empty manuscript', () => {
  it('counts nothing', () => {
    expect(documentStats(doc())).toEqual({
      words: 0,
      characters: 0,
      charactersNoSpaces: 0,
      paragraphs: 0,
      footnotes: 0,
    })
  })

  it('does not count an empty paragraph as one', () => {
    expect(documentStats(doc(p(), p(text('   ')))).paragraphs).toBe(0)
  })
})

describe('the body', () => {
  it('counts words, characters with and without spaces, and paragraphs', () => {
    expect(documentStats(doc(h(text('Título')), p(text('Hola mundo.'))))).toEqual({
      words: 3,
      characters: 17,
      charactersNoSpaces: 16,
      paragraphs: 2,
      footnotes: 0,
    })
  })

  it('keeps words in separate paragraphs apart', () => {
    expect(documentStats(doc(p(text('fin')), p(text('inicio')))).words).toBe(2)
  })

  it('joins text split across marks into one word', () => {
    const bold = { type: 'text', text: 'pala', marks: [{ type: 'bold' }] } as N
    expect(documentStats(doc(p(bold, text('bra')))).words).toBe(1)
  })

  it('does not count a dash standing alone as a word', () => {
    expect(documentStats(doc(p(text('esto — aquello')))).words).toBe(2)
  })

  it('counts accented letters and ñ as one character each', () => {
    expect(documentStats(doc(p(text('año')))).characters).toBe(3)
  })

  it('counts the paragraphs inside lists, quotes and tables', () => {
    const list = {
      type: 'bulletList',
      content: [
        { type: 'listItem', content: [p(text('uno'))] },
        { type: 'listItem', content: [p(text('dos'))] },
      ],
    }
    const quote = { type: 'blockquote', content: [p(text('tres'))] }
    expect(documentStats(doc(list, quote))).toMatchObject({ words: 3, paragraphs: 3 })
  })
})

describe('what a citation shows', () => {
  it('counts a corpus quotation, a note snapshot and a Zotero rendering', () => {
    const stats = documentStats(
      doc(
        p(
          text('Dice '),
          { type: 'documentCitation', attrs: { quotedText: 'el queso' } },
          text(' y '),
          { type: 'noteLink', attrs: { contentSnapshot: 'mi nota' } },
          text(' '),
          { type: 'zoteroCitation', attrs: { renderedText: '(Ginzburg, 1976)' } }
        )
      )
    )
    expect(stats.words).toBe(8)
  })
})

describe('the footnotes', () => {
  it('are counted as notes, and their words are not added to the body', () => {
    const notes = {
      type: 'footnotes',
      content: [
        { type: 'footnote', attrs: { 'data-id': 'a' }, content: [p(text('una nota larga'))] },
        { type: 'footnote', attrs: { 'data-id': 'b' }, content: [p(text('otra'))] },
      ],
    }
    const ref = { type: 'footnoteReference', attrs: { 'data-id': 'a', referenceNumber: 1 } }

    expect(documentStats(doc(p(text('Cuerpo'), ref), notes))).toEqual({
      words: 1,
      characters: 6,
      charactersNoSpaces: 6,
      paragraphs: 1,
      footnotes: 2,
    })
  })
})
