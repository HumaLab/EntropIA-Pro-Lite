import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const SOURCE = readFileSync(resolve(import.meta.dirname, 'BibliotecaView.svelte'), 'utf-8')

describe('BibliotecaView builds its menu triggers on the design system', () => {
  it('opens the library and order menus from SelectTrigger, not a hand-built button', () => {
    // docs/AUDITORIA_ESTILOS.md R3: one select trigger for every menu of choices.
    expect(SOURCE).not.toMatch(/library-trigger/)
    expect(SOURCE.match(/<SelectTrigger\b/g)?.length ?? 0).toBe(2)
  })
})
