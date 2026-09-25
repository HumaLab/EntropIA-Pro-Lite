import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * Split view lets two panes each show a view of the same kind side by side.
 * A fixed DOM id is invalid HTML and an accessibility-tooling foot-gun the
 * moment that happens, so every view-owned id gets suffixed with the pane
 * id — see WritingView.pane-scope.test.ts, the first view this pattern was
 * proven on.
 *
 * `id="api-key"` legitimately appears twice (an if/else pair of text and
 * password inputs sharing one visible `<label for>`, never both mounted at
 * once) — both get the identical `-{paneId}` suffix, so this file checks
 * for at least the expected count rather than exactly one. That pre-existing
 * within-view duplicate is explicitly out of scope to resolve here.
 *
 * `assemblyai-speaker-labels-label`/`-value` are two distinct ids joined in
 * one space-separated `aria-labelledby`; each half gets its own suffix.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'SettingsView.svelte'), 'utf-8')

const paneContextImports = [
  ...SOURCE.matchAll(/import\s*\{([^}]*)\}\s*from\s*'\$lib\/pane-context'/g),
].map(([, names]) => names)

function occurrences(source: string, pattern: RegExp): number {
  return [...source.matchAll(pattern)].length
}

const FOR_ID_PAIRS = [
  'local-model-filename',
  'local-model-source',
  'local-embedding-model-dir',
  'assemblyai-api-key',
  'glm-ocr-api-key',
  'ocr-correction-prompt',
  'summary-prompt',
  'ner-prompt',
  'triplets-prompt',
]

describe('pane-scoped ids in the settings view', () => {
  it('imports getPaneId from pane-context alongside getNavigation', () => {
    expect(paneContextImports.join(' ')).toMatch(/\bgetNavigation\b/)
    expect(paneContextImports.join(' ')).toMatch(/\bgetPaneId\b/)
  })

  it('captures its own pane id at init', () => {
    expect(SOURCE).toMatch(/const paneId = getPaneId\(\)/)
  })

  it.each(FOR_ID_PAIRS)('suffixes %s and its label `for` reference with the pane id', (name) => {
    const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
    expect(SOURCE).toMatch(new RegExp(`for="${escaped}-\\{paneId\\}"`))
    expect(SOURCE).toMatch(new RegExp(`id="${escaped}-\\{paneId\\}"`))
    expect(SOURCE).not.toMatch(new RegExp(`for="${escaped}"`))
    expect(SOURCE).not.toMatch(new RegExp(`id="${escaped}"`))
  })

  it('suffixes both api-key input ids and their shared label `for` reference', () => {
    expect(SOURCE).toMatch(/for="api-key-\{paneId\}"/)
    expect(occurrences(SOURCE, /id="api-key-\{paneId\}"/g)).toBe(2)
    expect(SOURCE).not.toMatch(/for="api-key"/)
    expect(SOURCE).not.toMatch(/id="api-key"/)
  })

  it('suffixes both halves of the speaker-labels aria-labelledby pair, and their own ids', () => {
    expect(SOURCE).toMatch(
      /aria-labelledby="assemblyai-speaker-labels-label-\{paneId\} assemblyai-speaker-labels-value-\{paneId\}"/
    )
    expect(SOURCE).toMatch(/id="assemblyai-speaker-labels-label-\{paneId\}"/)
    expect(SOURCE).toMatch(/id="assemblyai-speaker-labels-value-\{paneId\}"/)
    expect(SOURCE).not.toMatch(/id="assemblyai-speaker-labels-label"/)
    expect(SOURCE).not.toMatch(/id="assemblyai-speaker-labels-value"/)
  })
})
