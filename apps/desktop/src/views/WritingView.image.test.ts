import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * The insert-image toolbar button is wired, and wired to the editable
 * editor — the same fact WritingView.dictation.test.ts checks for the
 * microphone, and for the same reason: WritingEditor shows the tool only
 * when it is handed `oninsertimage`, so a view that forgot to pass it
 * renders a perfectly good toolbar with the feature silently missing.
 * Rendering the view needs the whole store and Tauri behind it; the wiring
 * itself is a fact about the source.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'WritingView.svelte'), 'utf-8')

/** Each `<WritingEditor …/>` tag, attributes and all. */
const editors = [...SOURCE.matchAll(/<WritingEditor\b[\s\S]*?\/>/g)].map(([tag]) => tag)

describe('inserting an image from the writing view', () => {
  it('hands the editable manuscript the picker', () => {
    const editable = editors.find((tag) => tag.includes('onchange='))

    expect(editable, 'no editable WritingEditor in the view').toBeDefined()
    expect(editable).toMatch(/\boninsertimage=\{insertWritingImageFromPicker\}/)
  })

  it('leaves the placeholder behind a refused document without a picker', () => {
    const placeholder = editors.find((tag) => tag.includes('toolbar={false}'))

    expect(placeholder).toBeDefined()
    expect(placeholder).not.toContain('oninsertimage')
  })
})
