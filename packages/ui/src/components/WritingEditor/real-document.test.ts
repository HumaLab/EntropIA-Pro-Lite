import { Editor } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import fixture from './__fixtures__/real-document.json'
import { createWritingExtensions } from './extensions'
import { repairCanonical, validateCanonical, type CanonicalDocument } from './document-contract'
import { outlineFromDocument } from './outline'

/**
 * A manuscript written in the running app whose outline listed all seven of
 * its headings while the editor showed nothing. The stored JSON was intact —
 * 17 top-level nodes — so the failure was in rendering, not in persistence.
 * This runs that exact document through the real schema and editor.
 */
const REAL = fixture as unknown as CanonicalDocument

let editor: Editor | undefined
afterEach(() => {
  editor?.destroy()
  editor = undefined
})

describe('a document written by the running app', () => {
  it('passes the contract', () => {
    expect(validateCanonical(REAL)).toEqual({ ok: true })
  })

  it('has the headings its outline showed', () => {
    expect(outlineFromDocument(REAL).length).toBeGreaterThanOrEqual(7)
  })

  /**
   * Under `tiptap-footnotes@3` this document took the editor down when mounted
   * raw (a `nodeType` error drawing a marker with no footnote): the
   * blank-editor failure. Version 4 draws such a marker from its own
   * attributes, so it mounts. `parseCanonical` still repairs before handing
   * anything over, because builds on version 3 open the same documents.
   */
  it('mounts raw since tiptap-footnotes 4, keeping every top-level node', () => {
    const element = document.createElement('div')
    document.body.appendChild(element)

    editor = new Editor({ element, extensions: createWritingExtensions(), content: REAL.doc })

    expect(editor.state.doc.childCount).toBe(REAL.doc.content!.length)
    expect(element.querySelectorAll('h1,h2,h3,h4').length).toBeGreaterThanOrEqual(7)
  })
})

describe('repairing what the schema cannot catch', () => {
  it('reports the orphan footnote markers this document carries', () => {
    const { report } = repairCanonical(REAL)
    expect(report.orphanFootnoteReferences).toBeGreaterThan(0)
  })

  it('renders once repaired, keeping every paragraph and heading', () => {
    const { document: healed } = repairCanonical(REAL)
    const element = document.createElement('div')
    document.body.appendChild(element)
    editor = new Editor({ element, extensions: createWritingExtensions(), content: healed.doc })

    expect(editor.state.doc.childCount).toBe(REAL.doc.content!.length)
    expect(element.textContent).toContain('Para el estudio de la actividad pesquera')
    expect(element.querySelectorAll('h1,h2,h3,h4').length).toBeGreaterThanOrEqual(7)
  })

  it('leaves a healthy document untouched', () => {
    const healthy: CanonicalDocument = {
      schemaVersion: 1,
      doc: {
        type: 'doc',
        content: [{ type: 'paragraph', content: [{ type: 'text', text: 'x' }] }],
      },
    }
    const { document: out, report } = repairCanonical(healthy)
    expect(report.orphanFootnoteReferences).toBe(0)
    expect(out).toBe(healthy)
  })
})
