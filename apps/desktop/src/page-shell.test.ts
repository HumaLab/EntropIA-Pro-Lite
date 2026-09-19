import { readdirSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const APP_CSS = readFileSync(resolve(import.meta.dirname, 'app.css'), 'utf-8')
const VIEWS_DIR = resolve(import.meta.dirname, 'views')

function ruleBody(css: string, selector: string): string {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const match = new RegExp(`(^|\\n)\\s*${escaped}\\s*\\{([^}]*)\\}`).exec(css)
  return match?.[2] ?? ''
}

/** Each view's root class, taken from the element that carries `page-shell`. */
function pageShellViews(): Array<{ file: string; rootClass: string; source: string }> {
  return readdirSync(VIEWS_DIR)
    .filter((name) => name.endsWith('.svelte'))
    .flatMap((file) => {
      const source = readFileSync(resolve(VIEWS_DIR, file), 'utf-8')
      const rootClass = /class="([a-z-]+) page-shell"/.exec(source)?.[1]
      return rootClass ? [{ file, rootClass, source }] : []
    })
}

describe('page shell', () => {
  it('keeps the space above the status bar in the shared class', () => {
    expect(ruleBody(APP_CSS, '.page-shell')).toMatch(/padding-block-end:\s*var\(--space-4\)/)
  })

  it('leaves that space to the shared class instead of each view', () => {
    const views = pageShellViews()
    expect(views.length).toBeGreaterThan(0)
    for (const { file, rootClass, source } of views) {
      expect(ruleBody(source, `.${rootClass}`), file).not.toMatch(/padding-block-end/)
    }
  })
})
