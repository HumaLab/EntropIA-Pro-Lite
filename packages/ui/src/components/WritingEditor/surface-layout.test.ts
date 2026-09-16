import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * Layout rules for the manuscript surface that a rendering test cannot reach:
 * jsdom performs no layout, so the only way to hold these is to read the
 * component's own stylesheet.
 *
 * Both rules here come from the same defect report: with the caret in the
 * document, a focus ring was drawn flush around the text column while the
 * footnote list rendered its markers outside it.
 */

const SOURCE = readFileSync(resolve(import.meta.dirname, 'WritingEditor.svelte'), 'utf-8')
const STYLES = SOURCE.slice(SOURCE.indexOf('<style>'))

describe('the manuscript surface', () => {
  /**
   * `:focus-visible` always matches an element that accepts text input, so a
   * ring on the editing surface is not an occasional keyboard affordance — it
   * is drawn the entire time someone is writing, hugging the text column. The
   * caret is the focus indicator a writing surface already has.
   */
  it('draws no focus ring around the text column', () => {
    expect(STYLES).not.toMatch(/\.writing-editor__surface:focus-visible\)\s*\{[^}]*box-shadow/)
  })

  /**
   * A list marker is rendered outside its item's content box by default. The
   * surface is a fixed-width column, so `1.` and `•` land to the left of the
   * text — visibly outside the manuscript. Padding is what brings them back in.
   */
  it('keeps bullet and ordered list markers inside the column', () => {
    const rule = STYLES.slice(STYLES.indexOf('.writing-editor__surface ul'))
    expect(rule).toContain('.writing-editor__surface ol')
    expect(rule.slice(0, rule.indexOf('}'))).toContain('padding-inline-start')
  })

  it('keeps the footnote list markers inside the column too', () => {
    expect(STYLES).toMatch(/\.footnotes\)\s*\{[^}]*padding-inline-start/)
  })
})
