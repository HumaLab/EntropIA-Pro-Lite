import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * `.topbar__icon-btn`, `.topbar__window-btn` (TopBar.svelte) and
 * `.analysis-section__download` (CollectionAnalysisPanel.svelte) each set
 * `width`/`height` on an IconButton at a single class selector — the same
 * specificity as IconButton's own scoped `.icon-button--{size}` rule, which
 * already sets `width`/`height` via `--icon-button-size`. Whichever rule the
 * bundler happens to emit last wins; today that is always IconButton's own
 * rule (measured: topbar__icon-btn renders at IconButton's `size="md"` 32px
 * rather than the override's `var(--control-height-sm)` 30px; topbar__window-btn
 * renders at `size="sm"` 28px rather than 30px; analysis-section__download's
 * 28px happens to already match `size="sm"`'s 28px). None of the three
 * express a size the `size` prop does not already produce, so they were
 * removed rather than converted to `--icon-button-size` — converting a
 * declaration that never wins today would change the rendered size instead
 * of leaving it as-is.
 */
const TOPBAR_SOURCE = readFileSync(resolve(import.meta.dirname, 'layout/TopBar.svelte'), 'utf-8')
const ANALYSIS_PANEL_SOURCE = readFileSync(
  resolve(import.meta.dirname, 'views/CollectionAnalysisPanel.svelte'),
  'utf-8'
)

function styleBlock(source: string): string {
  return source
    .slice(source.indexOf('>', source.indexOf('<style')) + 1, source.lastIndexOf('</style>'))
    .replace(/\/\*[\s\S]*?\*\//g, '')
}

describe('dead width/height overrides on IconButton call sites are gone, not silently re-enabled', () => {
  it('TopBar no longer sets width/height on .topbar__icon-btn or .topbar__window-btn', () => {
    const style = styleBlock(TOPBAR_SOURCE)
    // The rules themselves stay (border-radius, color, the --settings hook);
    // only the size declarations that never won are gone.
    expect(style).toMatch(/:global\(\.topbar__icon-btn\)\s*\{[^}]*border-radius:/)
    expect(style).toMatch(/:global\(\.topbar__window-btn\)\s*\{[^}]*border-radius:/)
    for (const selector of [':global(.topbar__icon-btn)', ':global(.topbar__window-btn)']) {
      const at = style.indexOf(selector)
      const close = style.indexOf('}', at)
      const body = style.slice(at, close)
      expect(body, `${selector} still sets a dead width/height`).not.toMatch(/\b(width|height):/)
    }
  })

  it('CollectionAnalysisPanel no longer sets width/height on .analysis-section__download', () => {
    const style = styleBlock(ANALYSIS_PANEL_SOURCE)
    expect(style).not.toMatch(/analysis-section__download/)
  })

  it('TopBar still renders these buttons through IconButton’s size prop, which now solely owns their size', () => {
    expect(TOPBAR_SOURCE).toMatch(/class="topbar__icon-btn"[\s\S]{0,80}size="md"/)
    expect(TOPBAR_SOURCE).toMatch(/class="topbar__window-btn"[\s\S]{0,80}size="sm"/)
  })
})
