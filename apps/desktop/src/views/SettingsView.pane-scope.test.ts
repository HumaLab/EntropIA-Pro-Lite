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
]

// The prompt and stop-sequence fields are Textareas with their own label:
// the component derives the id per instance, so it is unique per pane by
// construction and the view writes none.
it('leaves the prompt fields to the Textarea’s own ids', () => {
  for (const name of ['ocr-correction-prompt', 'summary-prompt', 'ner-prompt', 'triplets-prompt']) {
    expect(SOURCE).not.toMatch(new RegExp(`id="${name}`))
  }
  expect(SOURCE).not.toMatch(/-stop-sequences/)
})

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

  // One field whose type toggles, so the id appears once.
  it('suffixes the api-key input id and its label `for` reference', () => {
    expect(SOURCE).toMatch(/for="api-key-\{paneId\}"/)
    expect(occurrences(SOURCE, /id="api-key-\{paneId\}"/g)).toBe(1)
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
