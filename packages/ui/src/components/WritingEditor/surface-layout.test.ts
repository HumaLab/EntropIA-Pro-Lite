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

/**
 * The number that labels each note at the foot of the manuscript.
 *
 * A browser's own list marker cannot be raised: `::marker` accepts a font size
 * but not `vertical-align`, so matching the superscript reference in the body
 * means numbering the notes ourselves. The `footnote` node holds `paragraph+`,
 * so the counter also has to leave the flow — an inline `::before` would open
 * an anonymous block above the note's first paragraph instead of labelling it.
 */
describe('the footnote numbers', () => {
  const NOTES = STYLES.slice(STYLES.indexOf('.writing-editor__surface .footnotes'))

  it('turns off the browser list marker so the number can be styled', () => {
    expect(NOTES.slice(0, NOTES.indexOf('}'))).toMatch(/list-style:\s*none/)
  })

  it('numbers the notes with a counter of its own', () => {
    expect(NOTES).toMatch(/content:\s*counter\(footnote\)/)
  })

  it('raises the number out of the flow rather than inline', () => {
    const marker = NOTES.slice(NOTES.indexOf('::before'))
    expect(marker.slice(0, marker.indexOf('}'))).toMatch(/position:\s*absolute/)
  })
})

/**
 * Paragraph indent (paragraph-format.ts). A block carries its level as
 * `--writing-indent`; the surface turns it into a margin. The step is the body
 * size, never the block's own em: a heading indented one level must line up
 * with a paragraph indented one level, not step further by its larger font.
 */
describe('the paragraph indent', () => {
  const RULE = STYLES.slice(STYLES.indexOf('.writing-editor__surface [data-indent]'))
  const body = RULE.slice(0, RULE.indexOf('}'))

  it('is drawn by the surface from the level each block carries', () => {
    expect(STYLES).toContain('.writing-editor__surface [data-indent]')
    expect(body).toMatch(/margin-inline-start:\s*calc\(var\(--writing-indent/)
  })

  it('steps by the body size, not by the block’s own em', () => {
    expect(body).toContain('var(--font-size-md)')
    expect(body).not.toMatch(/\d\s*em\b/)
  })
})

describe('highlights', () => {
  /**
   * A background fills the glyph box, which is taller than the line at tight
   * spacing: at line-height 1 each highlighted line painted over the
   * descenders of the one above. A band exactly one line tall, centred on the
   * glyph box, tiles the lines edge to edge instead.
   */
  it('paints a highlight as a band no taller than its line', () => {
    const at = STYLES.indexOf('.writing-editor__surface mark[data-highlight])')
    expect(at).toBeGreaterThan(-1)
    const rule = STYLES.slice(at, STYLES.indexOf('}', at))
    expect(rule).toMatch(/background-image:\s*linear-gradient\(\s*var\(--writing-highlight,/)
    expect(rule).toMatch(/background-size:\s*100%\s+1lh/)
    expect(rule).toMatch(/background-position:\s*center/)
    expect(rule).toMatch(/background-repeat:\s*no-repeat/)
  })
})

describe('a highlight never covers text', () => {
  /**
   * The browser paints line by line, background then glyphs, so a highlight's
   * next line lands on the descenders of the line above whenever the glyphs
   * reach past their line — at line-height 1 in any serif. No band height fixes
   * that for every spacing. Blending does: on a dark page the lighter pixel
   * wins, so light ink shows through a dark highlight; on a light page the
   * darker one wins. Either way the ink stays on top of every highlight.
   */
  it('blends the highlight with a mode each theme chooses', () => {
    const at = STYLES.indexOf('.writing-editor__surface mark[data-highlight])')
    const rule = STYLES.slice(at, STYLES.indexOf('}', at))
    expect(rule).toMatch(/mix-blend-mode:\s*var\(--writing-blend-highlight,/)
  })

  it('lets the lighter pixel win on dark themes and the darker one on light themes', () => {
    const tokens = readFileSync(resolve(import.meta.dirname, '../../tokens/tokens.css'), 'utf-8')
    const blendIn = (selector: string) => {
      const block = tokens.slice(
        tokens.indexOf(selector),
        tokens.indexOf('}', tokens.indexOf(selector))
      )
      return /--writing-blend-highlight:\s*([a-z-]+)/.exec(block)?.[1]
    }
    expect(blendIn(':root {')).toBe('lighten')
    expect(blendIn(":root[data-theme='dim'] {")).toBe('lighten')
    expect(blendIn(":root[data-theme='light'] {")).toBe('darken')
    expect(blendIn(":root[data-theme='lite'] {")).toBe('darken')
  })
})

describe('line spacing', () => {
  /**
   * "1" is Word's single spacing: the font's own line, ascenders to
   * descenders. As CSS line-height 1 it was one em, shorter than every reading
   * face (1.28 to 1.49 em), so the lines of a paragraph ran into each other.
   */
  it('multiplies the chosen spacing by the reading face single line', () => {
    const at = STYLES.indexOf('[data-line-height]')
    expect(at).toBeGreaterThan(-1)
    const rule = STYLES.slice(at, STYLES.indexOf('}', at))
    expect(rule).toMatch(
      /line-height:\s*calc\(\s*var\(--writing-line-height,\s*1\)\s*\*\s*var\(--font-reading-single-line,/
    )
  })

  it('gives every typography preset the single line of its reading face', () => {
    const tokens = readFileSync(resolve(import.meta.dirname, '../../tokens/tokens.css'), 'utf-8')
    const singleIn = (selector: string) => {
      const at = tokens.indexOf(selector)
      const block = tokens.slice(at, tokens.indexOf('}', at))
      return Number(/--font-reading-single-line:\s*([\d.]+)/.exec(block)?.[1])
    }
    // hhea ascender - descender + line gap over units per em, measured from
    // each face's latin woff2: what Chromium lays out as line-height: normal.
    expect(singleIn("[data-font='academic'] {")).toBe(1.371)
    expect(singleIn("[data-font='modern'] {")).toBe(1.485)
    expect(singleIn("[data-font='editorial'] {")).toBe(1.28)
    expect(singleIn("[data-font='archive'] {")).toBe(1.362)
  })
})
