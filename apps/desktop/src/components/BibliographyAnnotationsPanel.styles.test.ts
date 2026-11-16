import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const SOURCE = readFileSync(
  resolve(import.meta.dirname, 'BibliographyAnnotationsPanel.svelte'),
  'utf-8'
)
const MARKUP = SOURCE.replace(/<style\b[\s\S]*?<\/style>/g, '')

describe('BibliographyAnnotationsPanel builds its fields and badges on the design system', () => {
  it('writes the annotation comment in the Textarea component (R2)', () => {
    expect(MARKUP).not.toMatch(/<textarea\b/)
    expect(MARKUP).toMatch(/<Textarea\b/)
  })

  it('shows the sync state in a StatusBadge, not a local badge class (R8)', () => {
    expect(SOURCE).not.toMatch(/annotation-entry__badge/)
    expect(MARKUP).toMatch(/<StatusBadge\b[^>]*data-testid="annotation-sync-badge"/)
  })
})
