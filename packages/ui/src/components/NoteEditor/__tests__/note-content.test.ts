import { afterEach, describe, expect, it, vi } from 'vitest'

import {
  convertLegacyNoteTextToHtml,
  hasNoteEditorMeaningfulChanges,
  isLegacyPlainTextNoteContent,
  normalizeNoteLinkHref,
  normalizeNoteContentForEditor,
  normalizeNoteContentForRender,
  sanitizeNoteHtml,
  shouldDisableNoteEditorSave,
} from '../note-content'

describe('note-content helpers', () => {
  it('detects legacy plain text and ignores html note content', () => {
    expect(isLegacyPlainTextNoteContent('Primera linea\n\nSegunda linea')).toBe(true)
    expect(isLegacyPlainTextNoteContent('<p><strong>Nota</strong></p>')).toBe(false)
  })

  it('converts legacy text to safe paragraph html', () => {
    expect(convertLegacyNoteTextToHtml('Hola <mundo>\nlinea 2\n\nBloque 2')).toBe(
      '<p>Hola &lt;mundo&gt;<br>linea 2</p><p>Bloque 2</p>'
    )
  })

  it('sanitizes unsafe html while preserving supported formatting', () => {
    expect(
      sanitizeNoteHtml(
        '<h2>Titulo</h2><p><a href="javascript:alert(1)">click</a> <strong>ok</strong></p><script>alert(1)</script>'
      )
    ).toBe('<h2>Titulo</h2><p>click <strong>ok</strong></p>')
  })

  describe('without a DOM (Q-04)', () => {
    afterEach(() => {
      vi.unstubAllGlobals()
    })

    it('escapes html it cannot filter instead of returning it raw', () => {
      vi.stubGlobal('document', undefined)
      const out = sanitizeNoteHtml(
        '<p>ok</p><img src=x onerror="alert(1)"><script>alert(1)</script>'
      )
      expect(out).not.toMatch(/<(img|script)\b/i)
      expect(out).toBe(
        '<p>&lt;p&gt;ok&lt;/p&gt;&lt;img src=x onerror=&quot;alert(1)&quot;&gt;&lt;script&gt;alert(1)&lt;/script&gt;</p>'
      )
    })

    it('still converts legacy plain text into paragraphs', () => {
      vi.stubGlobal('document', undefined)
      expect(sanitizeNoteHtml('Linea 1\nLinea 2')).toBe('<p>Linea 1<br>Linea 2</p>')
    })
  })

  it('normalizes safe links and preserves href metadata for rendering', () => {
    expect(normalizeNoteLinkHref('entropia.dev')).toBe('https://entropia.dev/')
    expect(normalizeNoteLinkHref('https://entropia.dev/docs')).toBe('https://entropia.dev/docs')
    expect(normalizeNoteLinkHref('javascript:alert(1)')).toBeNull()
    expect(sanitizeNoteHtml('<p><a href="entropia.dev/docs">Docs</a></p>')).toBe(
      '<p><a href="https://entropia.dev/docs" target="_blank" rel="noopener noreferrer nofollow">Docs</a></p>'
    )
  })

  it('normalizes both legacy and rich text content for rendering and editing', () => {
    expect(normalizeNoteContentForRender('Linea 1\nLinea 2')).toBe('<p>Linea 1<br>Linea 2</p>')
    expect(normalizeNoteContentForEditor('<p>Hola <em>mundo</em></p><iframe></iframe>')).toBe(
      '<p>Hola <em>mundo</em></p>'
    )
  })

  it('detects meaningful note changes after html normalization', () => {
    expect(
      hasNoteEditorMeaningfulChanges({
        originalContent: 'Linea uno\n\nLinea dos',
        currentContent: '<p>Linea uno</p><p>Linea dos</p>',
      })
    ).toBe(false)

    expect(
      hasNoteEditorMeaningfulChanges({
        originalContent: '<p>Texto original</p>',
        currentContent: '<p>Texto actualizado</p>',
      })
    ).toBe(true)
  })

  it('disables save only when empty or unchanged during edit mode', () => {
    expect(
      shouldDisableNoteEditorSave({
        currentContent: '<p></p>',
        originalContent: '<p></p>',
        isEditing: false,
      })
    ).toBe(true)

    expect(
      shouldDisableNoteEditorSave({
        currentContent: '<p>Nota existente</p>',
        originalContent: '<p>Nota existente</p>',
        isEditing: true,
      })
    ).toBe(true)

    expect(
      shouldDisableNoteEditorSave({
        currentContent: '<p>Nota existente</p>',
        originalContent: '<p>Nota original</p>',
        isEditing: true,
      })
    ).toBe(false)

    expect(
      shouldDisableNoteEditorSave({
        currentContent: '<p>Nota lista para guardar</p>',
        originalContent: '<p>Nota lista para guardar</p>',
        isEditing: false,
      })
    ).toBe(false)
  })
})
