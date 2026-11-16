import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const SOURCE = readFileSync(resolve(import.meta.dirname, 'ResearchView.svelte'), 'utf-8')
const STYLES = SOURCE.slice(SOURCE.indexOf('<style>')).replace(/\/\*[\s\S]*?\*\//g, '')

describe('ResearchView builds its controls on the design system', () => {
  it('writes the question in Textarea, not a raw textarea', () => {
    // docs/AUDITORIA_ESTILOS.md R2.
    expect(SOURCE).not.toMatch(/<textarea\b/)
    expect(SOURCE).toMatch(/<Textarea\b/)
  })

  it('folds the collection scope and the handoff preview with Disclosure', () => {
    // N11: the native details marker, focus outline and type differed per view.
    expect(SOURCE).not.toMatch(/<details\b/)
    expect(SOURCE.match(/<Disclosure\b/g)?.length ?? 0).toBe(2)
  })

  it('opens the library menu from SelectTrigger, not a hand-built button', () => {
    // R3: one select trigger for every menu of choices.
    expect(SOURCE).not.toMatch(/library-trigger/)
    expect(SOURCE).toMatch(/<SelectTrigger\b/)
  })

  it('sizes the submit button from a wrapper it owns, not by restyling .btn', () => {
    expect(STYLES).not.toMatch(/:global\(\.btn\)/)
  })

  it('takes letter-spacing from the caps token', () => {
    expect(STYLES).not.toMatch(/letter-spacing:\s*-?[0-9.]/)
  })
})
