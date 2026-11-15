import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const SOURCE = readFileSync(resolve(import.meta.dirname, 'RagChatView.svelte'), 'utf-8')
const STYLES = SOURCE.slice(SOURCE.indexOf('<style>')).replace(/\/\*[\s\S]*?\*\//g, '')

describe('RagChatView builds its controls on the design system', () => {
  it('writes the question in Textarea, not a raw textarea', () => {
    // docs/AUDITORIA_ESTILOS.md R2.
    expect(SOURCE).not.toMatch(/<textarea\b/)
    expect(SOURCE).toMatch(/<Textarea\b/)
  })

  it('renames a conversation in Input, whose focus ring lives on focus only', () => {
    // E6: the hand-built field drew the accent border and the ring at rest.
    expect(SOURCE).not.toMatch(/<input\b/)
    expect(SOURCE).toMatch(/<Input\b/)
    expect(STYLES).not.toMatch(/conversation-title-input/)
  })

  it('opens the collection and library menus from SelectTrigger', () => {
    // R3: one select trigger for every menu of choices.
    expect(SOURCE).not.toMatch(/library-trigger/)
    expect(SOURCE.match(/<SelectTrigger\b/g)?.length ?? 0).toBe(2)
  })

  it('keeps the conversation search on SearchBar', () => {
    expect(SOURCE).toMatch(/<SearchBar\b/)
  })

  it('takes type sizes, line-height, letter-spacing and disabled opacity from tokens', () => {
    // F5: the inline code used a relative 0.9em.
    expect(STYLES).not.toMatch(/font-size:\s*[0-9.]+(px|em|rem)/)
    expect(STYLES).not.toMatch(/line-height:\s*[0-9.]/)
    expect(STYLES).not.toMatch(/letter-spacing:\s*-?[0-9.]/)
    expect(STYLES).not.toMatch(/opacity:\s*0?\.\d/)
  })
})
