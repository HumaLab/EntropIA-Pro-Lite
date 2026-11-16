import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const SOURCE = readFileSync(resolve(import.meta.dirname, 'DbBrowserView.svelte'), 'utf-8')
const STYLES = SOURCE.slice(SOURCE.indexOf('<style')).replace(/\/\*[\s\S]*?\*\//g, '')

describe('DbBrowserView draws its expanded-cell dialog with the system Dialog', () => {
  it('opens the expanded cell in Dialog, with no hand-built overlay or panel (R10)', () => {
    expect(SOURCE).toMatch(/<Dialog\s[\s\S]{0,80}size="lg"/)
    expect(SOURCE).not.toMatch(/role="dialog"/)
    expect(STYLES).not.toMatch(/\.modal-overlay|\.db-browser-modal \{/)
  })

  it('leaves the icon buttons their own tooltip and hover', () => {
    // IconButton already shows its title through the shared tooltip layer; a
    // second, hand-drawn ::after tooltip and a :global hover restyle were
    // duplicates of the system component.
    expect(SOURCE).not.toMatch(/db-browser-action-shell/)
    expect(STYLES).not.toMatch(/:global\(\.db-browser-table__cell-action\)/)
  })
})
