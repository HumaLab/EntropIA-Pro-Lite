import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * Split view lets two panes each show a view of the same kind side by side.
 * A fixed DOM id is invalid HTML and an accessibility-tooling foot-gun the
 * moment that happens, so every view-owned id gets suffixed with the pane
 * id — see WritingView.pane-scope.test.ts, the first view this pattern was
 * proven on.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'ResearchView.svelte'), 'utf-8')

const paneContextImports = [
  ...SOURCE.matchAll(/import\s*\{([^}]*)\}\s*from\s*'\$lib\/pane-context'/g),
].map(([, names]) => names)

describe('pane-scoped ids in the research view', () => {
  it('imports getPaneId from pane-context alongside getNavigation', () => {
    expect(paneContextImports.join(' ')).toMatch(/\bgetNavigation\b/)
    expect(paneContextImports.join(' ')).toMatch(/\bgetPaneId\b/)
  })

  it('captures its own pane id at init', () => {
    expect(SOURCE).toMatch(/const paneId = getPaneId\(\)/)
  })

  it('suffixes the research title id and its aria-labelledby reference with the pane id', () => {
    expect(SOURCE).toMatch(/aria-labelledby="research-title-\{paneId\}"/)
    expect(SOURCE).toMatch(/id="research-title-\{paneId\}"/)
  })

  it('suffixes the research-jobs title id and its aria-labelledby reference with the pane id', () => {
    expect(SOURCE).toMatch(/aria-labelledby="research-jobs-title-\{paneId\}"/)
    expect(SOURCE).toMatch(/id="research-jobs-title-\{paneId\}"/)
  })

  it('suffixes the research-form title id and its aria-labelledby reference with the pane id', () => {
    expect(SOURCE).toMatch(/aria-labelledby="research-form-title-\{paneId\}"/)
    expect(SOURCE).toMatch(/id="research-form-title-\{paneId\}"/)
  })

  it('leaves no unsuffixed literal id or reference behind', () => {
    expect(SOURCE).not.toMatch(/id="research-title"/)
    expect(SOURCE).not.toMatch(/aria-labelledby="research-title"/)
    expect(SOURCE).not.toMatch(/id="research-jobs-title"/)
    expect(SOURCE).not.toMatch(/aria-labelledby="research-jobs-title"/)
    expect(SOURCE).not.toMatch(/id="research-form-title"/)
    expect(SOURCE).not.toMatch(/aria-labelledby="research-form-title"/)
  })
})
