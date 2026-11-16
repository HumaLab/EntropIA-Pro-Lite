import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const SOURCE = readFileSync(resolve(import.meta.dirname, 'BibliographyWorkView.svelte'), 'utf-8')
const MARKUP = SOURCE.replace(/<style\b[\s\S]*?<\/style>/g, '')

describe('BibliographyWorkView builds its controls on the design system', () => {
  it('opens the attachment menu from SelectTrigger (R3)', () => {
    expect(SOURCE).not.toMatch(/work-attachment-trigger/)
    expect(MARKUP).toMatch(/<SelectTrigger\b/)
  })

  it('shows the library name in a StatusBadge, not a local chip (R8)', () => {
    expect(SOURCE).not.toMatch(/work-header__chip/)
    expect(MARKUP).toMatch(/<StatusBadge\b/)
  })

  it('opens an attachment with IconButton, not a hand-built square button', () => {
    expect(SOURCE).not.toMatch(/work-attachments__open/)
  })
})
