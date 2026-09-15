import { describe, expect, it } from 'vitest'
import { getSchema } from '@tiptap/core'
import { createWritingExtensions } from './extensions'
import { outlineDepth, outlineFromDocument } from './outline'
import { WRITING_SCHEMA_VERSION, type CanonicalDocument } from './document-contract'

function doc(content: unknown[]): CanonicalDocument {
  return { schemaVersion: WRITING_SCHEMA_VERSION, doc: { type: 'doc', content: content as never } }
}

const heading = (level: number, text: string) => ({
  type: 'heading',
  attrs: { level },
  content: [{ type: 'text', text }],
})

const para = (text: string) => ({ type: 'paragraph', content: [{ type: 'text', text }] })

describe('outline — what it collects', () => {
  it('is empty for an empty document', () => {
    expect(outlineFromDocument(doc([{ type: 'paragraph' }]))).toEqual([])
    expect(outlineFromDocument(null)).toEqual([])
  })

  it('collects headings in document order with their level and text', () => {
    const outline = outlineFromDocument(
      doc([heading(1, 'Capitulo'), para('texto'), heading(2, 'Seccion'), heading(3, 'Sub')])
    )

    expect(outline.map((e) => [e.level, e.text])).toEqual([
      [1, 'Capitulo'],
      [2, 'Seccion'],
      [3, 'Sub'],
    ])
  })

  it('keeps a heading that has not been typed yet, so the panel does not flicker', () => {
    const outline = outlineFromDocument(doc([{ type: 'heading', attrs: { level: 2 } }]))
    expect(outline).toHaveLength(1)
    expect(outline[0]?.text).toBe('')
  })

  it('ignores headings nested inside other blocks — they are not sections', () => {
    const outline = outlineFromDocument(
      doc([
        heading(1, 'Real'),
        { type: 'blockquote', content: [heading(2, 'Dentro de una cita')] },
        {
          type: 'table',
          content: [
            {
              type: 'tableRow',
              content: [
                {
                  type: 'tableCell',
                  attrs: { colspan: 1, rowspan: 1, colwidth: null },
                  content: [heading(3, 'Dentro de una tabla')],
                },
              ],
            },
          ],
        },
      ])
    )

    expect(outline.map((e) => e.text)).toEqual(['Real'])
  })
})

/**
 * The positions are what the editor scrolls to, so they have to agree with
 * ProseMirror's own accounting rather than with our arithmetic. This resolves
 * the document through the real schema and compares.
 */
describe('outline — positions agree with ProseMirror', () => {
  it('points at each heading node, whatever precedes it', () => {
    const source = doc([
      para('un parrafo antes'),
      heading(1, 'Capitulo'),
      para('otro parrafo, mas largo que el anterior'),
      heading(2, 'Seccion'),
      { type: 'bulletList', content: [{ type: 'listItem', content: [para('item')] }] },
      heading(3, 'Final'),
    ])

    const schema = getSchema(createWritingExtensions())
    const node = schema.nodeFromJSON(source.doc)

    const expected: number[] = []
    node.forEach((child, offset) => {
      if (child.type.name === 'heading') expected.push(offset + 1)
    })

    expect(outlineFromDocument(source).map((e) => e.position)).toEqual(expected)
  })

  it('accounts for inline marks and atoms inside the text', () => {
    const source = doc([
      {
        type: 'paragraph',
        content: [
          { type: 'text', marks: [{ type: 'bold' }], text: 'negrita' },
          {
            type: 'documentCitation',
            attrs: { citationNodeId: 'c-1', assetId: 'a1' },
          },
        ],
      },
      heading(2, 'Despues de una cita'),
    ])

    const schema = getSchema(createWritingExtensions())
    const node = schema.nodeFromJSON(source.doc)
    const expected: number[] = []
    node.forEach((child, offset) => {
      if (child.type.name === 'heading') expected.push(offset + 1)
    })

    expect(outlineFromDocument(source).map((e) => e.position)).toEqual(expected)
  })
})

describe('outline — display depth', () => {
  it('normalises so a document that starts at H2 is not pushed right', () => {
    const entries = outlineFromDocument(doc([heading(2, 'A'), heading(3, 'B'), heading(2, 'C')]))
    expect(entries.map((e) => outlineDepth(entries, e))).toEqual([0, 1, 0])
  })

  it('keeps real depth when the document starts at H1', () => {
    const entries = outlineFromDocument(doc([heading(1, 'A'), heading(3, 'B')]))
    expect(entries.map((e) => outlineDepth(entries, e))).toEqual([0, 2])
  })
})
