import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * Dictation in the manuscript is wired, and wired to the editable editor.
 *
 * WritingEditor shows its microphone only when it is handed a transcriber, so
 * a view that forgot to pass one renders a perfectly good toolbar with the
 * feature silently missing. Rendering the view needs the whole store and Tauri
 * behind it; the wiring itself is a fact about the source.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'WritingView.svelte'), 'utf-8')

/** Each `<WritingEditor …/>` tag, attributes and all. */
const editors = [...SOURCE.matchAll(/<WritingEditor\b[\s\S]*?\/>/g)].map(([tag]) => tag)

describe('dictation in the writing view', () => {
  it('hands the editable manuscript a transcriber and a log', () => {
    const editable = editors.find((tag) => tag.includes('onchange='))

    expect(editable, 'no editable WritingEditor in the view').toBeDefined()
    expect(editable).toMatch(/\bondictate=\{transcribeDictation\}/)
    expect(editable).toMatch(/\bondictationlog=/)
    expect(editable).toMatch(/\blabels=/)
  })

  it('logs dictation under its own source, like the notes panel does', () => {
    expect(SOURCE).toMatch(/appendLog\(\s*level,\s*'dictation'/)
  })

  it('leaves the placeholder behind a refused document without a microphone', () => {
    const placeholder = editors.find((tag) => tag.includes('toolbar={false}'))

    expect(placeholder).toBeDefined()
    expect(placeholder).not.toContain('ondictate')
  })
})
