import { describe, expect, it } from 'vitest'
import {
  buildContext,
  evidenceOf,
  sentRecord,
  type ContextPiece,
} from './agent-context'

/**
 * What the agent is told (plan-editor.md §14.3, §14.4).
 *
 * Every case here is about the writer being able to account for what left their
 * machine: nothing sent that was not shown, nothing dropped without being
 * reported, and no claim of evidence that was not actually given.
 */

const piece = (over: Partial<ContextPiece> = {}): ContextPiece => ({
  kind: 'corpus',
  label: 'Un fragmento',
  text: 'el molino de viento',
  ...over,
})

describe('building the context', () => {
  it('keeps what it was given, in the order the action needs', () => {
    const built = buildContext([
      piece({ kind: 'outline', label: 'Esquema', text: 'un resumen' }),
      piece({ kind: 'selection', label: 'Lo seleccionado', text: 'el parrafo' }),
      piece({ kind: 'instruction', label: 'Lo pedido', text: 'mejorar la claridad' }),
    ])

    expect(built.pieces.map((p) => p.kind)).toEqual(['instruction', 'selection', 'outline'])
  })

  it('ignores a piece with nothing in it', () => {
    const built = buildContext([piece({ text: '   ' }), piece({ text: 'algo' })])

    expect(built.pieces).toHaveLength(1)
  })

  /**
   * A fragment reached both as corpus evidence and as a linked citation is one
   * fragment. Sending it twice spends the budget on a copy.
   */
  it('sends a piece once however many ways it was reached', () => {
    const built = buildContext([
      piece({ kind: 'corpus', sourceId: 'as1' }),
      piece({ kind: 'corpus', sourceId: 'as1', label: 'El mismo, por otro camino' }),
    ])

    expect(built.pieces).toHaveLength(1)
    expect(built.omitted).toEqual([expect.objectContaining({ reason: 'duplicate' })])
  })

  it('treats identical text with no identity as the same piece too', () => {
    const built = buildContext([piece({ text: 'igual' }), piece({ text: 'igual' })])

    expect(built.pieces).toHaveLength(1)
  })

  /** Two different works are two pieces, however alike they read. */
  it('does not collapse two pieces that merely resemble each other', () => {
    const built = buildContext([
      piece({ sourceId: 'as1' }),
      piece({ sourceId: 'as2' }),
    ])

    expect(built.pieces).toHaveLength(2)
  })

  /**
   * A budget that silently drops the tail produces an answer the writer cannot
   * account for. What does not fit is reported.
   */
  it('reports what did not fit rather than dropping it quietly', () => {
    const built = buildContext(
      [
        piece({ kind: 'selection', text: 'x'.repeat(50) }),
        piece({ kind: 'corpus', sourceId: 'as1', text: 'y'.repeat(50) }),
      ],
      { maxChars: 60, maxPieces: 10 }
    )

    expect(built.pieces).toHaveLength(1)
    expect(built.omitted).toEqual([expect.objectContaining({ reason: 'over_budget' })])
  })

  /**
   * The selection and the instruction are what the action *is*. Without them
   * there is no question to answer, so the budget never takes them first.
   */
  it('never drops the question in order to keep the evidence', () => {
    const built = buildContext(
      [
        piece({ kind: 'corpus', sourceId: 'as1', text: 'y'.repeat(100) }),
        piece({ kind: 'outline', text: 'z'.repeat(100) }),
        piece({ kind: 'selection', text: 'el parrafo' }),
        piece({ kind: 'instruction', text: 'acortar' }),
      ],
      { maxChars: 40, maxPieces: 10 }
    )

    expect(built.pieces.map((p) => p.kind)).toEqual(['instruction', 'selection'])
  })

  it('honours a limit on how many pieces may go, whatever their size', () => {
    const built = buildContext(
      [piece({ sourceId: 'a' }), piece({ sourceId: 'b' }), piece({ sourceId: 'c' })],
      { maxChars: 10_000, maxPieces: 2 }
    )

    expect(built.pieces).toHaveLength(2)
    expect(built.omitted).toHaveLength(1)
  })

  /** Deduplication before the budget: a copy must never displace a real piece. */
  it('does not spend the budget on a duplicate', () => {
    const built = buildContext(
      [
        piece({ sourceId: 'as1', text: 'x'.repeat(30) }),
        piece({ sourceId: 'as1', text: 'x'.repeat(30) }),
        piece({ sourceId: 'as2', text: 'y'.repeat(30) }),
      ],
      { maxChars: 60, maxPieces: 10 }
    )

    expect(built.pieces.map((p) => p.sourceId)).toEqual(['as1', 'as2'])
  })

  it('counts what it will actually send', () => {
    const built = buildContext([piece({ text: '12345' }), piece({ text: '123', sourceId: 'b' })])

    expect(built.chars).toBe(8)
  })
})

/**
 * §14.4: the writer must be able to know what will be sent. That is only true
 * while the preview, the request and the record are the same object.
 */
describe('the record of what was sent', () => {
  it('describes exactly the pieces that go, and nothing else', () => {
    const built = buildContext(
      [
        piece({ kind: 'selection', text: 'el parrafo' }),
        piece({ kind: 'corpus', sourceId: 'as1', text: 'evidencia' }),
        piece({ kind: 'outline', text: 'z'.repeat(500) }),
      ],
      { maxChars: 30, maxPieces: 10 }
    )

    const record = sentRecord(built)

    expect(record.map((r) => r.kind)).toEqual(built.pieces.map((p) => p.kind))
    expect(record.every((r) => r.chars > 0)).toBe(true)
    expect(record.some((r) => r.kind === 'outline')).toBe(false)
  })
})

/**
 * §14.2 asks a suggestion to distinguish corpus evidence, Zotero metadata and
 * text that was actually consulted. Claiming evidence that was never sent would
 * make the whole provenance record worthless.
 */
describe('what the proposal rests on', () => {
  it('names the corpus, Zotero and notes it was given', () => {
    const built = buildContext([
      piece({ kind: 'corpus', sourceId: 'as1' }),
      piece({ kind: 'zotero', sourceId: 'ABCD1234', text: 'Ginzburg 1976' }),
      piece({ kind: 'note', sourceId: 'n1', text: 'una nota' }),
    ])

    expect(evidenceOf(built)).toMatchObject({
      corpus: ['as1'],
      zotero: ['ABCD1234'],
      notes: ['n1'],
    })
  })

  /**
   * A bibliographic record is not the work. A proposal that saw a title and an
   * author has not read anything, and §14.2 draws that line on purpose.
   */
  it('does not call Zotero metadata a consulted text', () => {
    const built = buildContext([
      piece({ kind: 'zotero', sourceId: 'ABCD1234', text: 'Ginzburg, C. (1976)' }),
    ])

    expect(evidenceOf(built).consultedText).toBe(false)
  })

  it('says text was consulted when a fragment really went', () => {
    const built = buildContext([piece({ kind: 'corpus', sourceId: 'as1' })])

    expect(evidenceOf(built).consultedText).toBe(true)
  })

  /** A proposal with no evidence is the model's own prose, and says so. */
  it('claims nothing for a proposal that was given no evidence', () => {
    const built = buildContext([
      piece({ kind: 'selection', text: 'el parrafo' }),
      piece({ kind: 'instruction', text: 'acortar' }),
    ])

    expect(evidenceOf(built)).toEqual({
      corpus: [],
      zotero: [],
      notes: [],
      consultedText: false,
    })
  })

  /** Evidence that did not fit the budget was not sent, so it is not evidence. */
  it('does not count a piece the budget left behind', () => {
    const built = buildContext(
      [
        piece({ kind: 'selection', text: 'el parrafo' }),
        piece({ kind: 'corpus', sourceId: 'as1', text: 'x'.repeat(500) }),
      ],
      { maxChars: 20, maxPieces: 10 }
    )

    expect(evidenceOf(built).corpus).toEqual([])
    expect(evidenceOf(built).consultedText).toBe(false)
  })
})
