import { writingSchema } from '@entropia/ui'
import { describe, expect, it } from 'vitest'
import {
  CITATION_FIDELITY,
  MARK_FIDELITY,
  NODE_FIDELITY,
  REQUIRED_BY_SPEC,
  fidelityWarnings,
  losesRequiredElement,
  supportOfNode,
} from './export-fidelity'

/**
 * The fidelity matrix (plan-editor.md §17.4).
 *
 * The test that matters most is the last one in the first block: the matrix is
 * checked against the schema itself, so a node added to the manuscript and
 * forgotten here is caught now rather than by exporting as silence.
 */

const p = (text: string) => ({ type: 'paragraph', content: [{ type: 'text', text }] })

describe('the matrix covers what a manuscript can contain', () => {
  /**
   * The schema is the single definition of what a manuscript may hold. A node
   * missing from the matrix would export as nothing, with no warning — the
   * quietest possible data loss.
   */
  it('has a row for every node in the writing schema', () => {
    const missing = Object.keys(writingSchema().nodes).filter((name) => !(name in NODE_FIDELITY))

    expect(missing, 'schema nodes with no fidelity row').toEqual([])
  })

  it('has a row for every mark in the writing schema', () => {
    const missing = Object.keys(writingSchema().marks).filter((name) => !(name in MARK_FIDELITY))

    expect(missing, 'schema marks with no fidelity row').toEqual([])
  })

  /** And nothing in the matrix that the schema does not have, so rows die too. */
  it('has no row for a node the schema does not have', () => {
    const schema = writingSchema()
    const orphans = Object.keys(NODE_FIDELITY).filter((name) => !(name in schema.nodes))

    expect(orphans, 'fidelity rows for nodes that do not exist').toEqual([])
  })

  /**
   * §17.4: a warning may not stand in for an obligatory element in DOCX. S4
   * verified every one of them natively, so anything less here is a regression
   * in the exporter, not a limitation of the format.
   */
  it('keeps every obligatory element natively in DOCX', () => {
    const downgraded = REQUIRED_BY_SPEC.filter(
      (element) => supportOfNode(element, 'docx') !== 'native'
    )

    expect(downgraded, 'obligatory elements DOCX no longer keeps').toEqual([])
  })

  /** An unknown node is unsupported, never assumed to be fine. */
  it('treats a node it has never heard of as unsupported', () => {
    expect(supportOfNode('somethingNewEntirely', 'docx')).toBe('unsupported')
  })
})

describe('warning about what a format cannot carry', () => {
  it('says nothing about a document every format can carry whole', () => {
    const doc = { type: 'doc', content: [p('un parrafo')] }

    expect(fidelityWarnings(doc, 'docx')).toEqual([])
  })

  it('reports a mark the format has no equivalent for', () => {
    const doc = {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          content: [{ type: 'text', text: 'x', marks: [{ type: 'underline' }] }],
        },
      ],
    }

    expect(fidelityWarnings(doc, 'markdown')).toEqual([
      { element: 'underline', kind: 'mark', support: 'fallback', count: 1 },
    ])
    expect(fidelityWarnings(doc, 'html')).toEqual([])
  })

  /**
   * "One footnote could not be represented" and "forty could not" are different
   * facts, and the second usually changes the writer's mind about the format.
   */
  it('counts occurrences rather than reporting a set', () => {
    const link = { type: 'noteLink', attrs: {} }
    const doc = { type: 'doc', content: [{ type: 'paragraph', content: [link, link, link] }] }

    expect(fidelityWarnings(doc, 'markdown')).toEqual([
      { element: 'noteLink', kind: 'node', support: 'fallback', count: 3 },
    ])
  })

  /**
   * A warning about a representation nothing uses trains people to ignore
   * warnings, so it is only raised when the document actually cites the corpus.
   */
  it('warns about the citation representation only when the document cites', () => {
    const withCitation = {
      type: 'doc',
      content: [{ type: 'paragraph', content: [{ type: 'documentCitation', attrs: {} }] }],
    }
    const without = { type: 'doc', content: [p('nada que citar')] }

    expect(fidelityWarnings(withCitation, 'markdown', 'comment')).toContainEqual({
      element: 'comment',
      kind: 'citation',
      support: 'unsupported',
      count: 1,
    })
    expect(fidelityWarnings(without, 'markdown', 'comment')).toEqual([])
  })

  it('finds what is nested deep inside a table cell', () => {
    const doc = {
      type: 'doc',
      content: [
        {
          type: 'table',
          content: [
            {
              type: 'tableRow',
              content: [
                {
                  type: 'tableCell',
                  content: [{ type: 'paragraph', content: [{ type: 'noteLink', attrs: {} }] }],
                },
              ],
            },
          ],
        },
      ],
    }

    expect(fidelityWarnings(doc, 'docx')).toEqual([
      { element: 'noteLink', kind: 'node', support: 'fallback', count: 1 },
    ])
  })
})

describe('refusing to call a lossy DOCX a partial export', () => {
  /**
   * §17.4 is explicit: a warning does not discharge an obligatory element that
   * DOCX must keep. So this is a failure, not a caveat.
   */
  it('names the obligatory element a DOCX export would lose', () => {
    const warnings = [
      { element: 'footnote', kind: 'node' as const, support: 'fallback' as const, count: 2 },
    ]

    expect(losesRequiredElement(warnings, 'docx')).toEqual(['footnote'])
  })

  /**
   * §17.1 qualifies its list with "en la medida admitida por cada formato", and
   * Markdown genuinely has no underline. Holding it to the DOCX bar would make
   * the format unusable for a reason the spec does not ask for.
   */
  it('does not hold markdown and html to the DOCX bar', () => {
    const warnings = [
      { element: 'footnote', kind: 'node' as const, support: 'fallback' as const, count: 2 },
    ]

    expect(losesRequiredElement(warnings, 'markdown')).toEqual([])
    expect(losesRequiredElement(warnings, 'html')).toEqual([])
  })

  it('passes a DOCX whose only warnings are of things not on the list', () => {
    const warnings = [
      { element: 'noteLink', kind: 'node' as const, support: 'fallback' as const, count: 1 },
    ]

    expect(losesRequiredElement(warnings, 'docx')).toEqual([])
  })
})

describe('the four representations of a corpus citation', () => {
  /**
   * Markdown has no comment that survives rendering. Quietly turning the option
   * into a footnote would give the writer a document they did not ask for.
   */
  it('does not admit a comment in markdown rather than substituting one', () => {
    expect(CITATION_FIDELITY.comment.markdown).toBe('unsupported')
    expect(CITATION_FIDELITY.comment.docx).toBe('native')
  })
})

/**
 * The typography marks. HTML and DOCX have each of them; Markdown has none, so
 * each goes out as the inline HTML underline already uses, and says so.
 */
describe('the typography marks', () => {
  it('are native in HTML and DOCX and a declared stand-in in Markdown', () => {
    for (const mark of ['subscript', 'superscript', 'textStyle', 'highlight']) {
      expect(MARK_FIDELITY[mark], mark).toEqual({
        markdown: 'fallback',
        html: 'native',
        docx: 'native',
      })
    }
  })

  it('are reported when a Markdown export stands in for them', () => {
    const doc = {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          content: [
            { type: 'text', text: '2', marks: [{ type: 'subscript' }] },
            { type: 'text', text: 'g', marks: [{ type: 'textStyle', attrs: { fontSize: '2em' } }] },
          ],
        },
      ],
    }

    expect(fidelityWarnings(doc, 'markdown')).toEqual([
      { element: 'subscript', kind: 'mark', support: 'fallback', count: 1 },
      { element: 'textStyle', kind: 'mark', support: 'fallback', count: 1 },
    ])
    expect(fidelityWarnings(doc, 'docx')).toEqual([])
  })

  it('report a colour and a highlight when a Markdown export stands in for them', () => {
    const doc = {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          content: [
            { type: 'text', text: 'r', marks: [{ type: 'textStyle', attrs: { color: 'red' } }] },
            { type: 'text', text: 'h', marks: [{ type: 'highlight', attrs: { color: 'blue' } }] },
          ],
        },
      ],
    }

    expect(fidelityWarnings(doc, 'markdown')).toEqual([
      { element: 'textStyle', kind: 'mark', support: 'fallback', count: 1 },
      { element: 'highlight', kind: 'mark', support: 'fallback', count: 1 },
    ])
    expect(fidelityWarnings(doc, 'html')).toEqual([])
    expect(fidelityWarnings(doc, 'docx')).toEqual([])
  })
})
