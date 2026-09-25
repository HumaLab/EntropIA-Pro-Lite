import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * The outline and research panel ids are pane-scoped.
 *
 * Writing can only be open in one tab today (the single-tab rule), so two
 * WritingView instances can't yet coexist to make a duplicate DOM id
 * observable — but a fixed id is still invalid HTML the moment a second
 * pane can render one, and an accessibility-tooling foot-gun regardless.
 * Rendering the view needs the whole store and Tauri behind it (the same
 * reason WritingView.dictation.test.ts and friends stay at the source
 * level), so this file checks the ids the same way those check wiring: as
 * a fact about the source.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'WritingView.svelte'), 'utf-8')

/** Every `import { ... } from '$lib/pane-context'` line, names joined. */
const paneContextImports = [
  ...SOURCE.matchAll(/import\s*\{([^}]*)\}\s*from\s*'\$lib\/pane-context'/g),
].map(([, names]) => names)

describe('pane-scoped ids in the writing view', () => {
  it('imports getPaneId from pane-context alongside getNavigation', () => {
    expect(paneContextImports.join(' ')).toMatch(/\bgetNavigation\b/)
    expect(paneContextImports.join(' ')).toMatch(/\bgetPaneId\b/)
  })

  it('captures its own pane id at init', () => {
    expect(SOURCE).toMatch(/const paneId = getPaneId\(\)/)
  })

  it('suffixes the outline panel id and its resize handle with the pane id', () => {
    expect(SOURCE).toMatch(/id="writing-outline-panel-\{paneId\}"/)
    expect(SOURCE).toMatch(/controls="writing-outline-panel-\{paneId\}"/)
  })

  it('suffixes the research panel id and its resize handle with the pane id', () => {
    expect(SOURCE).toMatch(/controls="writing-research-panel-\{paneId\}"/)
    expect(SOURCE).toMatch(/id="writing-research-panel-\{paneId\}"/)
  })

  it('leaves no unsuffixed literal id or controls reference behind', () => {
    expect(SOURCE).not.toMatch(/id="writing-outline-panel"/)
    expect(SOURCE).not.toMatch(/controls="writing-outline-panel"/)
    expect(SOURCE).not.toMatch(/controls="writing-research-panel"/)
    expect(SOURCE).not.toMatch(/id="writing-research-panel"/)
  })
})
