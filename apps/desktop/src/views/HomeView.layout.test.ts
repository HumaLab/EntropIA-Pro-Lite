import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * The Continuar / Estado del corpus proportion and its responsive stacking
 * (odd/tasks/home-view.md T3a): jsdom performs no layout, so what it cannot
 * see is a ratio silently drifting or a breakpoint quietly dropped.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'HomeView.svelte'), 'utf-8')
const STYLES = SOURCE.replace(/\/\*[\s\S]*?\*\//g, '')

function ruleFor(selector: string): string {
  const at = STYLES.indexOf(selector)
  expect(at, `${selector} is no longer in the stylesheet`).toBeGreaterThan(-1)
  const rule = STYLES.slice(at)
  return rule.slice(0, rule.indexOf('}'))
}

/** A whole `@container …` (or any brace) block, matched by depth rather than
 *  by counting a fixed number of `}` — safe for a block that nests rules. */
function blockFor(selector: string): string {
  const at = STYLES.indexOf(selector)
  expect(at, `${selector} is no longer in the stylesheet`).toBeGreaterThan(-1)
  const open = STYLES.indexOf('{', at)
  let depth = 0
  for (let i = open; i < STYLES.length; i++) {
    if (STYLES[i] === '{') depth++
    else if (STYLES[i] === '}') {
      depth--
      if (depth === 0) return STYLES.slice(at, i + 1)
    }
  }
  throw new Error(`unterminated block for ${selector}`)
}

describe('Continuar / Estado del corpus proportion', () => {
  it('splits the top row roughly 60/40', () => {
    expect(ruleFor('.home-view__top-row {')).toMatch(/grid-template-columns:\s*3fr\s+2fr/)
  })

  it('stacks the two panels at the 720px pane-width breakpoint, not the window width', () => {
    // Keyed to the `pane` container (WorkPane.svelte) so a split pane
    // narrower than the window still stacks: a plain @media query only ever
    // sees the window, and a split pane is rarely the window's width.
    const mediaAt = STYLES.indexOf('@container pane (max-width: 720px)')
    expect(
      mediaAt,
      '@container pane (max-width: 720px) is not used in HomeView.svelte'
    ).toBeGreaterThan(-1)
    expect(STYLES).not.toContain('@media (max-width: 720px)')
    const mediaBlock = STYLES.slice(
      mediaAt,
      STYLES.indexOf('}', STYLES.indexOf('}', mediaAt) + 1) + 1
    )
    expect(mediaBlock).toContain('.home-view__top-row')
    expect(mediaBlock).toMatch(/grid-template-columns:\s*1fr/)
  })
})

/**
 * The 4-card quick-access row and the recent-activity table both reflow off
 * this pane's own rendered width (the `pane` container WorkPane.svelte
 * establishes), not the window — a split pane is rarely the window's width,
 * so content used to overflow or clip against the divider before it ever
 * got the chance to reflow (visual polish round, split view).
 */
describe('Acceso rápido and Actividad reciente reflow off the pane, not the window', () => {
  it('drops the quick-access row from 4 to 2 to 1 column as the pane narrows', () => {
    const twoCol = blockFor('@container pane (max-width: 680px)')
    expect(twoCol).toMatch(
      /\.home-view__quick-access-grid\s*\{\s*grid-template-columns:\s*repeat\(2,\s*1fr\);/
    )

    const oneCol = blockFor('@container pane (max-width: 380px)')
    expect(oneCol).toMatch(/\.home-view__quick-access-grid\s*\{\s*grid-template-columns:\s*1fr;/)
  })

  it('drops the recent-activity collection column before the row can overflow the pane', () => {
    const block = blockFor('@container pane (max-width: 560px)')
    expect(block).toMatch(
      /\.home-view__recent-row\s*\{\s*grid-template-columns:\s*minmax\(0,\s*1fr\)\s+auto;/
    )
    expect(block).toMatch(/\.home-view__recent-row\s*>\s*:nth-child\(2\)\s*\{\s*display:\s*none;/)
  })
})

/**
 * Acceso rápido micro-layout (T3h): more inner padding, a taller/comfortable
 * box, icon/copy/arrow away from the edges and vertically centered, all four
 * cards sharing the one class so they stay equal height and aligned.
 */
describe('Acceso rápido card micro-layout', () => {
  it('gives the card more breathing room than the previous 64px/16px gutter', () => {
    const rule = ruleFor('.home-view__quick-access-card {')
    expect(rule).not.toMatch(/height:\s*64px/)
    expect(rule).not.toMatch(/padding:\s*0\s+var\(--space-4\)/)
    expect(rule).toMatch(/padding:\s*var\(--space-4\)\s+var\(--space-5\)/)
  })

  it('lays icon, copy and arrow out in one row so the content sits vertically centered', () => {
    const rule = ruleFor('.home-view__quick-access-card {')
    expect(rule).toMatch(/display:\s*flex/)
    expect(rule).toMatch(/align-items:\s*center/)
    expect(rule).toMatch(/gap:\s*var\(--space-\d\)/)
  })

  it('no longer pins the arrow to the corner: it flows in the row instead', () => {
    const rule = ruleFor('.home-view__quick-access-arrow {')
    expect(rule).not.toMatch(/position:\s*absolute/)
  })

  it('groups title and description in their own column, separate from the icon and arrow', () => {
    expect(STYLES).toContain('.home-view__quick-access-copy')
    const rule = ruleFor('.home-view__quick-access-copy {')
    expect(rule).toMatch(/flex:\s*1/)
  })

  it('uses only spacing tokens for the card gutter, never a raw hex or px literal beyond the icon size', () => {
    const rule = ruleFor('.home-view__quick-access-card {')
    expect(rule).not.toMatch(/#[0-9a-fA-F]{3,8}/)
  })
})

/**
 * Quick-access arrows are readable at rest (T3k-3): they used to sit at
 * `opacity: 0.55` on top of an already-muted color, reading as almost
 * invisible. At rest they must use a muted-but-readable text token — never
 * a border token — at full opacity, brightening to secondary text on hover.
 */
describe('Quick-access arrow is readable at rest', () => {
  it('uses the muted text token at rest, not a border token, and no extra dimming opacity', () => {
    const rule = ruleFor('.home-view__quick-access-arrow {')
    expect(rule).toMatch(/color:\s*var\(--color-text-muted\)/)
    expect(rule).not.toMatch(/--color-border/)
    expect(rule).not.toMatch(/opacity:\s*0(\.\d+)?;/)
  })

  it('is drawn at the large text size, not the extra-small one it started with', () => {
    // At --font-size-xs (12px) the glyph read as a speck next to the 17px title.
    const rule = ruleFor('.home-view__quick-access-arrow {')
    expect(rule).toMatch(/font-size:\s*var\(--font-size-lg\)/)
    expect(rule).toMatch(/line-height:\s*1;/)
  })

  it('brightens to secondary text on hover/focus', () => {
    const at = STYLES.indexOf('.home-view__quick-access-arrow,')
    expect(at, 'hover/focus rule for the arrow is missing').toBeGreaterThan(-1)
    const rule = STYLES.slice(at, STYLES.indexOf('}', at))
    expect(rule).toMatch(/color:\s*var\(--color-text-secondary\)/)
  })
})

/**
 * Continuar rows stay compact (T3k-1): since T3i the top row sizes to the
 * taller Estado del corpus panel, and the Continuar rows used to stretch
 * (flex: 1) to fill it — ~100px each. Rows must keep their natural, compact
 * height instead, stacked at the top, leaving any spare row height empty
 * below the last row.
 */
describe('Continuar rows stay compact instead of stretching to the panel height (T3k)', () => {
  it('does not let a Continuar row grow to fill the panel', () => {
    const rule = ruleFor('.home-view__continuar-item {')
    expect(rule).not.toMatch(/flex:\s*1\b/)
  })

  it('sizes the row from its own padding instead of a stretched 100% height', () => {
    const rule = ruleFor('.home-view__continuar-row {')
    expect(rule).not.toMatch(/height:\s*100%/)
    expect(rule).toMatch(/padding:\s*var\(--space-3\)\s+var\(--space-4\)/)
  })

  it('keeps the list itself filling the panel, so spare space lands below the last row', () => {
    const rule = ruleFor('.home-view__continuar-list {')
    expect(rule).toMatch(/flex:\s*1\b/)
  })
})

/**
 * Estado del corpus fits every indicator (T3i): the top row's original fixed
 * 250px height clipped the Embeddings row once OCR/STT/Texto/Embeddings grew
 * to six lines. The row must size to its own content instead, and the corpus
 * panel must never hide overflow that would clip a real indicator.
 */
describe('Estado del corpus panel fits every indicator without clipping (T3i)', () => {
  it('lets the top row and its panels take their content height instead of a fixed pixel height', () => {
    const topRow = ruleFor('.home-view__top-row {')
    expect(topRow).not.toMatch(/height:\s*\d/)

    const panel = ruleFor('.home-panel {')
    expect(panel).not.toMatch(/(?<!min-)height:\s*\d/)
  })

  it('never clips the corpus panel content behind overflow: hidden', () => {
    const rule = ruleFor('.home-view__corpus {')
    expect(rule).not.toMatch(/overflow:\s*hidden/)
  })
})

/**
 * The three panels (Continuar, Estado del corpus, Actividad reciente) let the
 * animated constellation show through, as in the approved canvas design
 * (panels at 78 % over the field). Solid surfaces hid it everywhere but the gaps.
 */
describe('Inicio panels are translucent over the constellation', () => {
  it('mixes the raised surface with transparency instead of painting it solid', () => {
    const panel = ruleFor('.home-panel {')
    expect(panel).toMatch(
      /background:\s*color-mix\(in srgb, var\(--color-surface-raised\) 78%, transparent\);/
    )
  })
})

describe('Estado del corpus breathes between its indicators', () => {
  it('spaces the indicator rows at --space-5, using the room the sync line left', () => {
    const grid = ruleFor('.home-view__corpus-grid {')
    expect(grid).toMatch(/gap:\s*var\(--space-5\);/)
  })
})

describe('Inicio header is translucent over the constellation', () => {
  it('overrides the shared opaque page header with the same 78 % mix as the panels', () => {
    expect(SOURCE).toMatch(/<section class="page-header home-view__header"/)
    const header = ruleFor('.home-view__header {')
    expect(header).toMatch(
      /background:\s*color-mix\(in srgb, var\(--surface-app\) 78%, transparent\);/
    )
  })
})
