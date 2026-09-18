import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { DEFAULT_WRITING_EDITOR_LABELS } from '@entropia/ui'
import { afterEach, describe, expect, it } from 'vitest'

import { locale, t } from './i18n'
import { writingEditorLabels } from './writing-editor-labels'

/**
 * The manuscript editor speaks the application's language.
 *
 * Its strings used to arrive as Spanish defaults baked into the component, so
 * an English session got a Spanish toolbar. Every label now comes from the
 * translations, and these tests are what keep a new label from slipping back.
 */
const DICTATION_KEYS = Object.keys(DEFAULT_WRITING_EDITOR_LABELS).filter((key) =>
  key.startsWith('dictation')
)
const EDITOR_KEYS = Object.keys(DEFAULT_WRITING_EDITOR_LABELS).filter(
  (key) => !DICTATION_KEYS.includes(key)
) as (keyof typeof DEFAULT_WRITING_EDITOR_LABELS)[]

afterEach(() => {
  locale.set('es')
})

describe('writingEditorLabels', () => {
  it('covers every label the editor has', () => {
    expect(Object.keys(writingEditorLabels()).sort()).toEqual(
      Object.keys(DEFAULT_WRITING_EDITOR_LABELS).sort()
    )
  })

  it('says in Spanish exactly what the editor said before', () => {
    locale.set('es')
    const labels = writingEditorLabels()

    for (const key of EDITOR_KEYS) {
      expect(labels[key], key).toBe(DEFAULT_WRITING_EDITOR_LABELS[key])
    }
  })

  it('translates every one of them into English', () => {
    locale.set('en')
    const labels = writingEditorLabels()

    // A whole number reads the same in both languages (line spacing's 1 and 2).
    const untranslated = EDITOR_KEYS.filter(
      (key) =>
        (labels[key] === DEFAULT_WRITING_EDITOR_LABELS[key] && !/^\d+$/.test(labels[key])) ||
        labels[key].startsWith('writing.')
    )
    expect(untranslated).toEqual([])
    expect(labels.bold).toBe('Bold')
    expect(labels.moreTools).toBe('More tools')
  })

  it('writes line spacing with each language’s decimal separator', () => {
    locale.set('es')
    const es = writingEditorLabels()
    expect([es.lineHeight1, es.lineHeight115, es.lineHeight15, es.lineHeight2]).toEqual([
      '1',
      '1,15',
      '1,5',
      '2',
    ])
    expect(es.lineHeightDefault).toBe('Predeterminado')

    locale.set('en')
    const en = writingEditorLabels()
    expect([en.lineHeight1, en.lineHeight115, en.lineHeight15, en.lineHeight2]).toEqual([
      '1',
      '1.15',
      '1.5',
      '2',
    ])
    expect(en.lineHeightDefault).toBe('Default')
    expect(en.lineHeight).toBe('Line spacing')
  })

  it('keeps the dictation strings the notes editor already speaks', () => {
    locale.set('en')
    const labels = writingEditorLabels()

    expect(labels.dictationStart).toBe(t('item.noteEditor.dictationStart'))
    expect(labels.dictationTranscriptionFailed).toBe(t('item.noteEditor.transcriptionFailed'))
    expect(labels.dictationAutoStopInserted).toContain('{duration}')
  })

  it('is what the writing view hands to both of its editors', () => {
    const view = readFileSync(resolve(import.meta.dirname, '../views/WritingView.svelte'), 'utf-8')
    const editors = [...view.matchAll(/<WritingEditor\b[\s\S]*?\/>/g)].map(([tag]) => tag)

    expect(editors).toHaveLength(2)
    for (const editor of editors) expect(editor).toMatch(/labels=\{editorLabels\}/)
    // Derived, not read once: nothing remounts the view when the language
    // changes, so a plain call left the toolbar in the language it opened in.
    expect(view).toMatch(/const editorLabels = \$derived\(writingEditorLabels\(\)\)/)
  })
})
