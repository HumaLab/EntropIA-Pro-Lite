import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const SOURCE = readFileSync(resolve(import.meta.dirname, 'CollectionAnalysisPanel.svelte'), 'utf-8')
const STYLES = SOURCE.slice(SOURCE.indexOf('<style>')).replace(/\/\*[\s\S]*?\*\//g, '')

describe('CollectionAnalysisPanel builds on the design system', () => {
  it('writes the stopwords in Textarea, not a raw textarea', () => {
    // docs/AUDITORIA_ESTILOS.md R2.
    expect(SOURCE).not.toMatch(/<textarea\b/)
    expect(SOURCE).toMatch(/<Textarea\b/)
  })

  it('labels the chart axes at a type token, never below 11px', () => {
    // F1: the tick and word labels were 9px.
    expect(STYLES).not.toMatch(/font-size:\s*[0-9.]+(px|em|rem)/)
  })

  it('lays the panel out from an element it owns, not by restyling Panel or TabList', () => {
    expect(STYLES).not.toMatch(/:global\(\.panel/)
    expect(STYLES).not.toMatch(/:global\(\.analysis-panel-tabs\)/)
  })

  it('takes letter-spacing from the caps token', () => {
    expect(STYLES).not.toMatch(/letter-spacing:\s*-?[0-9.]/)
  })
})
