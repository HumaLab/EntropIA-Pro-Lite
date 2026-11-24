import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * The Lotes collection picker scrolls inside a box. A fixed 200px box could end
 * exactly on a row boundary (the card height follows the font), so a whole row
 * hid below the edge with nothing showing more existed. The box is sized in
 * card rows from the card's own line height, so half a row always peeks out.
 */
function scopeListRule(): string {
  const source = readFileSync(resolve(import.meta.dirname, 'BatchProcessingTab.svelte'), 'utf-8')
  return source.match(/\.batch-field__scope-list\s*\{([^}]*)\}/)?.[1] ?? ''
}

describe('Lotes collection picker height', () => {
  it('shows a partial row instead of a fixed pixel height', () => {
    const rule = scopeListRule()
    expect(rule).not.toMatch(/max-height:\s*\d+px/)
    expect(rule).toMatch(/max-height:\s*calc\([^;]*4\.5[^;]*\)/)
    expect(rule).toMatch(/1lh/)
    expect(rule).toMatch(/overflow-y:\s*auto/)
  })
})
