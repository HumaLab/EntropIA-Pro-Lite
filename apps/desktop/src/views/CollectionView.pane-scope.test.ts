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
const SOURCE = readFileSync(resolve(import.meta.dirname, 'CollectionView.svelte'), 'utf-8')

const paneContextImports = [
  ...SOURCE.matchAll(/import\s*\{([^}]*)\}\s*from\s*'\$lib\/pane-context'/g),
].map(([, names]) => names)

describe('pane-scoped ids in the collection view', () => {
  it('imports getPaneId from pane-context alongside getNavigation', () => {
    expect(paneContextImports.join(' ')).toMatch(/\bgetNavigation\b/)
    expect(paneContextImports.join(' ')).toMatch(/\bgetPaneId\b/)
  })

  it('captures its own pane id at init', () => {
    expect(SOURCE).toMatch(/const paneId = getPaneId\(\)/)
  })

  it('suffixes the import-progress description id and its aria-describedby reference with the pane id', () => {
    expect(SOURCE).toMatch(/aria-describedby="collection-import-progress-description-\{paneId\}"/)
    expect(SOURCE).toMatch(/id="collection-import-progress-description-\{paneId\}"/)
  })

  it('leaves no unsuffixed literal id or reference behind', () => {
    expect(SOURCE).not.toMatch(/id="collection-import-progress-description"/)
    expect(SOURCE).not.toMatch(/aria-describedby="collection-import-progress-description"/)
  })
})
