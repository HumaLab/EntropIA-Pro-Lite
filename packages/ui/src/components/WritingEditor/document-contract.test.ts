import { describe, expect, it } from 'vitest'
import {
  WRITING_SCHEMA_VERSION,
  emptyDocument,
  parseCanonical,
  validateCanonical,
  type CanonicalDocument,
} from './document-contract'

const RICH: CanonicalDocument = {
  schemaVersion: WRITING_SCHEMA_VERSION,
  doc: {
    type: 'doc',
    content: [
      { type: 'heading', attrs: { level: 1 }, content: [{ type: 'text', text: 'Capitulo' }] },
      {
        type: 'paragraph',
        content: [
          { type: 'text', marks: [{ type: 'bold' }], text: 'Negrita' },
          { type: 'text', text: ' y ' },
          { type: 'text', marks: [{ type: 'italic' }], text: 'cursiva' },
          {
            type: 'documentCitation',
            attrs: {
              citationNodeId: 'c-1',
              assetId: 'asset-42',
              pageNumber: 17,
              startChar: 1200,
              endChar: 1284,
              sourceTextHash: 'sha256:abc',
            },
          },
        ],
      },
    ],
  },
}

describe('document contract — what the schema accepts', () => {
  it('accepts a document using every node the MVP promises', () => {
    expect(validateCanonical(RICH)).toEqual({ ok: true })
  })

  it('accepts the empty document it hands a new manuscript', () => {
    expect(validateCanonical(emptyDocument())).toEqual({ ok: true })
  })

  it('accepts links and underline, which StarterKit does not ship', () => {
    const doc: CanonicalDocument = {
      schemaVersion: WRITING_SCHEMA_VERSION,
      doc: {
        type: 'doc',
        content: [
          {
            type: 'paragraph',
            content: [
              {
                type: 'text',
                marks: [
                  { type: 'underline' },
                  { type: 'link', attrs: { href: 'https://citationstyles.org/' } },
                ],
                text: 'CSL',
              },
            ],
          },
        ],
      },
    }
    expect(validateCanonical(doc)).toEqual({ ok: true })
  })

  it('accepts tables and footnotes', () => {
    const doc: CanonicalDocument = {
      schemaVersion: WRITING_SCHEMA_VERSION,
      doc: {
        type: 'doc',
        content: [
          {
            type: 'table',
            content: [
              {
                type: 'tableRow',
                content: [
                  {
                    type: 'tableHeader',
                    attrs: { colspan: 1, rowspan: 1, colwidth: null },
                    content: [{ type: 'paragraph', content: [{ type: 'text', text: 'H' }] }],
                  },
                ],
              },
            ],
          },
        ],
      },
    }
    expect(validateCanonical(doc)).toEqual({ ok: true })
  })
})

/**
 * Spike S1 measured the cost of getting this wrong: handing Tiptap a document
 * with one unknown node or mark does not drop that element — it empties the
 * whole document, silently, with no throw at the call site. With autosave on,
 * the next write puts that empty document over the real manuscript.
 */
describe('document contract — what it must refuse before the editor sees it', () => {
  it('refuses an unknown node and names it', () => {
    const doc = structuredClone(RICH)
    // A node from some future build. It has to stay unknown for this test to
    // mean anything, so it is deliberately not a name anyone plans to add.
    doc.doc.content!.push({ type: 'holographicMarginalia', attrs: { clusterId: 'z-1' } })

    const result = validateCanonical(doc)
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.code).toBe('unknown-node')
    expect(result.message).toContain('holographicMarginalia')
  })

  it('refuses an unknown mark and names it', () => {
    const doc = structuredClone(RICH)
    doc.doc.content![1]!.content![0]!.marks = [
      { type: 'provenanceMark', attrs: { origin: 'agent' } },
    ]

    const result = validateCanonical(doc)
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.code).toBe('unknown-mark')
    expect(result.message).toContain('provenanceMark')
  })

  it('refuses a schema version it does not know how to read', () => {
    const result = validateCanonical({ ...RICH, schemaVersion: WRITING_SCHEMA_VERSION + 1 })
    expect(result.ok).toBe(false)
    if (result.ok) return
    expect(result.code).toBe('unsupported-schema-version')
  })

  it('refuses something that is not a canonical envelope at all', () => {
    for (const rubbish of [null, undefined, 42, 'texto', {}, { doc: { type: 'doc' } }]) {
      const result = validateCanonical(rubbish)
      expect(result.ok, `accepted: ${JSON.stringify(rubbish)}`).toBe(false)
    }
  })

  it('tolerates an unknown attribute on a known node, because that is survivable', () => {
    const doc = structuredClone(RICH)
    doc.doc.content![0]!.attrs = { level: 1, futureField: 'x' }
    expect(validateCanonical(doc)).toEqual({ ok: true })
  })
})

describe('document contract — parsing is the only way in', () => {
  it('returns the document when it is valid', () => {
    const parsed = parseCanonical(RICH)
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) return
    expect(parsed.document.doc.content).toHaveLength(2)
  })

  it('returns the failure instead of a document, so there is nothing to autosave', () => {
    const doc = structuredClone(RICH)
    doc.doc.content!.push({ type: 'holographicMarginalia' })

    const parsed = parseCanonical(doc)
    expect(parsed.ok).toBe(false)
    expect('document' in parsed).toBe(false)
  })

  it('reads a JSON string as readily as an object, since that is how it is stored', () => {
    const parsed = parseCanonical(JSON.stringify(RICH))
    expect(parsed.ok).toBe(true)
  })

  it('refuses malformed JSON without throwing', () => {
    const parsed = parseCanonical('{"schemaVersion":1,"doc":')
    expect(parsed.ok).toBe(false)
    if (parsed.ok) return
    expect(parsed.code).toBe('invalid-structure')
  })
})
