import { Editor, type JSONContent } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { createWritingExtensions } from './extensions'
import { validateCanonical, WRITING_SCHEMA_VERSION } from './document-contract'

/**
 * `tiptap-footnotes@3` marks a footnote's `id` and `data-id` required, and
 * `WritingFootnote` (extensions.ts) gives them a null default again. That must
 * open a stored footnote missing both, and must not cost a footnote that has
 * them anything on its way through the clipboard's HTML.
 */

const editors: Editor[] = []

afterEach(() => {
  editors.splice(0).forEach((instance) => instance.destroy())
})

function mount(content: JSONContent | string) {
  const element = document.createElement('div')
  document.body.appendChild(element)
  const instance = new Editor({ element, extensions: createWritingExtensions(), content })
  editors.push(instance)
  return instance
}

const footnotesOf = (json: JSONContent) =>
  (json.content ?? []).find((node) => node.type === 'footnotes')?.content ?? []

const referencesOf = (json: JSONContent) =>
  (json.content ?? [])
    .flatMap((block) => block.content ?? [])
    .filter((node) => node.type === 'footnoteReference')

describe('footnote ids', () => {
  const STORED_ATTRS: [string, Record<string, unknown> | undefined][] = [
    ['neither id', undefined],
    ['only data-id', { 'data-id': 'f1' }],
    ['only id', { id: 'fn:1' }],
  ]

  for (const [label, attrs] of STORED_ATTRS) {
    it(`opens a stored footnote with ${label} and writes it back unchanged`, () => {
      const stored: JSONContent = {
        type: 'doc',
        content: [
          { type: 'paragraph', content: [{ type: 'text', text: 'Cuerpo' }] },
          {
            type: 'footnotes',
            content: [
              {
                type: 'footnote',
                ...(attrs ? { attrs } : {}),
                content: [{ type: 'paragraph', content: [{ type: 'text', text: 'La nota' }] }],
              },
            ],
          },
        ],
      }

      // The gate every manuscript passes before it is mounted (WritingEditor.svelte).
      expect(validateCanonical({ schemaVersion: WRITING_SCHEMA_VERSION, doc: stored })).toEqual({
        ok: true,
      })
      const first = mount(stored).getJSON() as JSONContent
      const [footnote] = footnotesOf(first)
      expect(footnote?.attrs).toEqual({ id: null, 'data-id': null, ...attrs })
      expect(JSON.stringify(footnote?.content)).toContain('La nota')
      expect(mount(first).getJSON()).toEqual(first)
    })
  }

  it('keeps both ids and the note text through its own HTML', () => {
    const original = mount({
      type: 'doc',
      content: [
        {
          type: 'paragraph',
          content: [
            { type: 'text', text: 'Una afirmación' },
            { type: 'footnoteReference', attrs: { 'data-id': 'ref-1', referenceNumber: '1' } },
            { type: 'text', text: ' y otra' },
            { type: 'footnoteReference', attrs: { 'data-id': 'ref-2', referenceNumber: '2' } },
          ],
        },
        {
          type: 'footnotes',
          content: [
            {
              type: 'footnote',
              attrs: { id: 'fn:1', 'data-id': 'ref-1' },
              content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Primera nota.' }] }],
            },
            {
              type: 'footnote',
              attrs: { id: 'fn:2', 'data-id': 'ref-2' },
              content: [{ type: 'paragraph', content: [{ type: 'text', text: 'Segunda nota.' }] }],
            },
          ],
        },
      ],
    }).getJSON() as JSONContent

    const parsed = mount(mount(original).getHTML()).getJSON() as JSONContent

    expect(footnotesOf(parsed).map((note) => note.attrs)).toEqual([
      { id: 'fn:1', 'data-id': 'ref-1' },
      { id: 'fn:2', 'data-id': 'ref-2' },
    ])
    expect(footnotesOf(parsed).map((note) => JSON.stringify(note.content))).toEqual(
      footnotesOf(original).map((note) => JSON.stringify(note.content))
    )
    expect(referencesOf(parsed).map((ref) => ref.attrs)).toEqual(
      referencesOf(original).map((ref) => ref.attrs)
    )
    // Not compared whole: a reference parsed from HTML takes its drawn number
    // as text content (`content: 'text*'` plus `contentElement` in
    // tiptap-footnotes, unchanged from 2.0.4 to 3.0.1), so the copy carries
    // "1" inside the reference where the stored JSON carries nothing.
  })
})
