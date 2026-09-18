import { Editor, getSchema } from '@tiptap/core'
import { describe, expect, it } from 'vitest'
import { createWritingExtensions } from './extensions'
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

/**
 * The typography marks (sub, sup, relative size) change what a manuscript may
 * hold without changing the version: a build that predates them already
 * refuses a document carrying an unknown mark, safely and without writing, and
 * bumping the version would make that same build refuse every document.
 */
/** What the editor writes on a paragraph with no paragraph formatting. */
const NO_PARAGRAPH_FORMAT = { textAlign: null, indent: null, lineHeight: null }

describe('document contract — the typography marks', () => {
  const TYPESET: CanonicalDocument = {
    schemaVersion: WRITING_SCHEMA_VERSION,
    doc: {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          // Every paragraph carries its block attributes, none set here.
          attrs: NO_PARAGRAPH_FORMAT,
          content: [
            { type: 'text', text: 'H' },
            { type: 'text', marks: [{ type: 'subscript' }], text: '2' },
            { type: 'text', text: 'O y m' },
            { type: 'text', marks: [{ type: 'bold' }, { type: 'superscript' }], text: '2' },
            { type: 'text', text: ' ' },
            {
              type: 'text',
              // In the schema's own order: textStyle ranks above the other marks.
              // The editor writes every attribute of the mark, so the colour it
              // does not have is there as null.
              marks: [
                { type: 'textStyle', attrs: { fontSize: '1.5em', color: null } },
                { type: 'italic' },
              ],
              text: 'grande',
            },
          ],
        },
      ],
    },
  }

  it('stays at schema version 1', () => {
    expect(WRITING_SCHEMA_VERSION).toBe(1)
  })

  it('accepts a document carrying them', () => {
    expect(validateCanonical(TYPESET)).toEqual({ ok: true })
  })

  it('reloads them exactly as they were saved', () => {
    const element = document.createElement('div')
    document.body.appendChild(element)
    const first = new Editor({
      element,
      extensions: createWritingExtensions(),
      content: TYPESET.doc,
    })
    const saved = JSON.parse(JSON.stringify({ schemaVersion: 1, doc: first.getJSON() }))
    first.destroy()

    const parsed = parseCanonical(JSON.stringify(saved))
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) return
    const second = new Editor({
      element,
      extensions: createWritingExtensions(),
      content: parsed.document.doc,
    })

    expect(second.getJSON()).toEqual(TYPESET.doc)
    second.destroy()
  })

  it('still opens a version 1 manuscript written before they existed', () => {
    expect(validateCanonical(RICH)).toEqual({ ok: true })
    expect(validateCanonical(emptyDocument())).toEqual({ ok: true })
  })
})

/**
 * Text colour and highlight store palette names (writing-colors.ts), at the
 * same schema version, for the same reason as the typography marks.
 */
describe('document contract — colours', () => {
  const COLOURED: CanonicalDocument = {
    schemaVersion: WRITING_SCHEMA_VERSION,
    doc: {
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          attrs: NO_PARAGRAPH_FORMAT,
          content: [
            {
              type: 'text',
              marks: [{ type: 'textStyle', attrs: { fontSize: null, color: 'red' } }],
              text: 'rojo',
            },
            { type: 'text', text: ' y ' },
            {
              type: 'text',
              marks: [
                { type: 'textStyle', attrs: { fontSize: '1.25em', color: 'blue' } },
                { type: 'bold' },
                { type: 'highlight', attrs: { color: 'yellow' } },
              ],
              text: 'resaltado',
            },
          ],
        },
      ],
    },
  }

  function reload(source: CanonicalDocument) {
    const element = document.createElement('div')
    document.body.appendChild(element)
    const first = new Editor({
      element,
      extensions: createWritingExtensions(),
      content: source.doc,
    })
    const saved = JSON.stringify({ schemaVersion: WRITING_SCHEMA_VERSION, doc: first.getJSON() })
    first.destroy()

    const parsed = parseCanonical(saved)
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) throw new Error(parsed.message)
    const second = new Editor({
      element,
      extensions: createWritingExtensions(),
      content: parsed.document.doc,
    })
    const json = second.getJSON()
    second.destroy()
    return json
  }

  it('stays at schema version 1 and accepts a document carrying them', () => {
    expect(WRITING_SCHEMA_VERSION).toBe(1)
    expect(validateCanonical(COLOURED)).toEqual({ ok: true })
  })

  it('reloads them exactly as they were saved', () => {
    expect(reload(COLOURED)).toEqual(COLOURED.doc)
  })

  /** A newer build's colour, or a hand edit: drawn as none, kept, never refused. */
  it('opens and keeps a colour name it does not know', () => {
    const future = structuredClone(COLOURED)
    const runs = future.doc.content![0]!.content!
    runs[0]!.marks = [{ type: 'textStyle', attrs: { fontSize: null, color: 'chartreuse' } }]
    runs[2]!.marks = [{ type: 'highlight', attrs: { color: 'ultraviolet' } }]

    expect(validateCanonical(future)).toEqual({ ok: true })
    expect(reload(future)).toEqual(future.doc)
  })

  it('opens a size saved before colours existed', () => {
    const before: CanonicalDocument = {
      schemaVersion: 1,
      doc: {
        type: 'doc',
        content: [
          {
            type: 'paragraph',
            content: [
              {
                type: 'text',
                marks: [{ type: 'textStyle', attrs: { fontSize: '2em' } }],
                text: 'a',
              },
            ],
          },
        ],
      },
    }

    expect(validateCanonical(before)).toEqual({ ok: true })
    expect(reload(before).content![0]!.content![0]!.marks).toEqual([
      { type: 'textStyle', attrs: { fontSize: '2em', color: null } },
    ])
  })
})

/**
 * Alignment, indent and line spacing are attributes of paragraphs and
 * headings, at the same schema version. They are not marks, so a build that
 * predates them does not refuse a document carrying them: it opens it and
 * drops them, and a save from there loses them.
 */
describe('document contract — paragraph formatting', () => {
  const FORMATTED: CanonicalDocument = {
    schemaVersion: WRITING_SCHEMA_VERSION,
    doc: {
      type: 'doc',
      content: [
        {
          type: 'heading',
          attrs: { level: 2, textAlign: 'center', indent: null, lineHeight: '1.15' },
          content: [{ type: 'text', text: 'Título' }],
        },
        {
          type: 'paragraph',
          attrs: { textAlign: 'justify', indent: 3, lineHeight: '2' },
          content: [{ type: 'text', text: 'Uno' }],
        },
        {
          type: 'bulletList',
          content: [
            {
              type: 'listItem',
              content: [
                {
                  type: 'paragraph',
                  attrs: { textAlign: 'right', indent: null, lineHeight: '1.5' },
                  content: [{ type: 'text', text: 'Dos' }],
                },
              ],
            },
          ],
        },
      ],
    },
  }

  function reload(source: CanonicalDocument) {
    const element = document.createElement('div')
    document.body.appendChild(element)
    const first = new Editor({
      element,
      extensions: createWritingExtensions(),
      content: source.doc,
    })
    const saved = JSON.stringify({ schemaVersion: WRITING_SCHEMA_VERSION, doc: first.getJSON() })
    first.destroy()
    const parsed = parseCanonical(saved)
    if (!parsed.ok) throw new Error(parsed.message)
    const second = new Editor({
      element,
      extensions: createWritingExtensions(),
      content: parsed.document.doc,
    })
    const json = second.getJSON()
    second.destroy()
    return json
  }

  it('stays at schema version 1 and accepts a document carrying them', () => {
    expect(WRITING_SCHEMA_VERSION).toBe(1)
    expect(validateCanonical(FORMATTED)).toEqual({ ok: true })
  })

  it('reloads them exactly as they were saved', () => {
    expect(reload(FORMATTED)).toEqual(FORMATTED.doc)
  })

  it('opens a version 1 manuscript written before them, as unformatted', () => {
    const before: CanonicalDocument = {
      schemaVersion: 1,
      doc: {
        type: 'doc',
        content: [
          { type: 'heading', attrs: { level: 1 }, content: [{ type: 'text', text: 'T' }] },
          { type: 'paragraph', content: [{ type: 'text', text: 'a' }] },
        ],
      },
    }

    expect(validateCanonical(before)).toEqual({ ok: true })
    expect(reload(before).content?.map((block) => block.attrs)).toEqual([
      { level: 1, ...NO_PARAGRAPH_FORMAT },
      NO_PARAGRAPH_FORMAT,
    ])
  })

  it('is what an older build drops, rather than refuses', () => {
    const older = getSchema(
      createWritingExtensions().filter(
        (extension) => !['textAlign', 'paragraphFormat'].includes(extension.name)
      )
    )
    const node = older.nodeFromJSON(FORMATTED.doc)
    node.check()

    expect(node.toJSON().content[1].attrs).toBeUndefined()
    expect(node.toJSON().content[0].attrs).toEqual({ level: 2 })
  })
})
