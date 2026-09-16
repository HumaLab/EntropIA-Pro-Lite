import { describe, expect, it } from 'vitest'
import { writingSchema } from './document-contract'
import { findMatches } from './search'

/**
 * Finding text in the manuscript (plan-editor.md §26, "búsqueda y reemplazo").
 *
 * The interesting cases are not "does it find a word". They are the ones a
 * per-node scan gets wrong: a phrase broken in two by a bold word, a query that
 * happens to contain regular-expression syntax, and two blocks whose text would
 * run together into a match that is not on the page.
 */

function docOf(content: unknown[]) {
  return writingSchema().nodeFromJSON({ type: 'doc', content })
}

const text = (value: string) => ({ type: 'text', text: value })
const para = (...content: unknown[]) => ({ type: 'paragraph', content })

describe('findMatches', () => {
  it('finds a plain occurrence and reports a range the editor can select', () => {
    const doc = docOf([para(text('el molino de viento'))])

    const [match, ...rest] = findMatches(doc, 'molino')

    expect(rest).toHaveLength(0)
    expect(doc.textBetween(match!.from, match!.to)).toBe('molino')
  })

  it('finds every occurrence, in document order', () => {
    const doc = docOf([para(text('sal y mas sal')), para(text('sal'))])

    const matches = findMatches(doc, 'sal')

    expect(matches).toHaveLength(3)
    expect(matches.map((m) => m.from)).toEqual([...matches.map((m) => m.from)].sort((a, b) => a - b))
  })

  /** A bold word mid-sentence splits the text into three nodes. */
  it('finds a phrase broken across marks', () => {
    const doc = docOf([
      para(text('la sociedad de los '), { ...text('molineros'), marks: [{ type: 'bold' }] }, text(' rurales')),
    ])

    const matches = findMatches(doc, 'los molineros rurales')

    expect(matches).toHaveLength(1)
    expect(doc.textBetween(matches[0]!.from, matches[0]!.to)).toBe('los molineros rurales')
  })

  /** Two paragraphs are not one sentence, however they concatenate. */
  it('does not run two blocks together into a match', () => {
    const doc = docOf([para(text('moli')), para(text('nero'))])

    expect(findMatches(doc, 'molinero')).toHaveLength(0)
  })

  it('is case-insensitive by default and exact when asked', () => {
    const doc = docOf([para(text('Molino molino MOLINO'))])

    expect(findMatches(doc, 'molino')).toHaveLength(3)
    expect(findMatches(doc, 'molino', { caseSensitive: true })).toHaveLength(1)
  })

  /**
   * The query is what someone typed, not a pattern. `(` must find a
   * parenthesis, never open a group and throw.
   */
  it('treats regular-expression syntax as literal text', () => {
    const doc = docOf([para(text('una cita (Marx, 1867) al paso'))])

    expect(findMatches(doc, '(Marx, 1867)')).toHaveLength(1)
    expect(() => findMatches(doc, '[')).not.toThrow()
    expect(findMatches(doc, '*')).toHaveLength(0)
  })

  /** Nothing is not everything; but a space is a query like any other. */
  it('finds nothing for an empty query, and whitespace when asked for it', () => {
    const doc = docOf([para(text('el molino de viento'))])

    expect(findMatches(doc, '')).toHaveLength(0)
    expect(findMatches(doc, ' ')).toHaveLength(3)
  })

  it('does not overlap matches', () => {
    const doc = docOf([para(text('aaaa'))])

    expect(findMatches(doc, 'aa')).toHaveLength(2)
  })

  /** A citation is an inline atom with no text; it must not shift the offsets. */
  it('keeps positions right across an inline atom', () => {
    const doc = docOf([
      para(text('antes '), { type: 'documentCitation' }, text(' despues del molino')),
    ])

    const [match] = findMatches(doc, 'molino')

    expect(doc.textBetween(match!.from, match!.to)).toBe('molino')
  })
})
