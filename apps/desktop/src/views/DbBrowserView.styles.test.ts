import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const SOURCE = readFileSync(resolve(import.meta.dirname, 'DbBrowserView.svelte'), 'utf-8')
const STYLES = SOURCE.slice(SOURCE.indexOf('<style')).replace(/\/\*[\s\S]*?\*\//g, '')

function ruleBody(selector: string): string {
  const at = STYLES.indexOf(`${selector} {`)
  expect(at, `${selector} is no longer in the stylesheet`).toBeGreaterThan(-1)
  return STYLES.slice(at, STYLES.indexOf('}', at))
}

describe('DbBrowserView draws its expanded-cell dialog like the system Dialog', () => {
  it('uses the dialog backdrop and layer tokens (R10)', () => {
    const overlay = ruleBody('.modal-overlay')
    expect(overlay).toMatch(/background-color:\s*var\(--color-overlay\)/)
    expect(overlay).toMatch(/z-index:\s*var\(--z-dialog\)/)
  })

  it('leaves the icon buttons their own tooltip and hover', () => {
    // IconButton already shows its title through the shared tooltip layer; a
    // second, hand-drawn ::after tooltip and a :global hover restyle were
    // duplicates of the system component.
    expect(SOURCE).not.toMatch(/db-browser-action-shell/)
    expect(STYLES).not.toMatch(/:global\(\.db-browser-table__cell-action\)/)
  })
})
