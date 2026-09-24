import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * The words the UI shows follow the archive's hierarchy, not the schema's:
 * Colección → Documento → Página, and an audio file is "audio" (user rules,
 * 2026-09-23/24). `item` and `asset` stay in code, keys and data; they never
 * reach the screen. This reads every dictionary value in i18n.ts.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'i18n.ts'), 'utf-8')
const ENTRY = /'([A-Za-z0-9_.]+)':\s*'((?:[^'\\]|\\.)*)'/g

// InvestigationView belongs to another session; its report heading is theirs
// to rename.
const ALLOWED = new Set(['investigation.report.items'])

function visibleEntries(): Array<[string, string]> {
  return Array.from(SOURCE.matchAll(ENTRY), (match) => [
    match[1]!,
    // Placeholders such as {asset} are variable names, not visible words.
    match[2]!.replace(/\{[^}]*\}/g, ''),
  ])
}

describe('i18n terminology', () => {
  it('never shows item or asset to the user', () => {
    const offenders = visibleEntries()
      .filter(([key, text]) => !ALLOWED.has(key) && /\b(items?|ítems?|assets?)\b/i.test(text))
      .map(([key, text]) => `${key}: ${text}`)

    expect(offenders).toEqual([])
  })

  it('calls an audio file audio, never a page', () => {
    const offenders = visibleEntries()
      .filter(([, text]) => /p[aá]ginas? de audio|audio pages?/i.test(text))
      .map(([key, text]) => `${key}: ${text}`)

    expect(offenders).toEqual([])
  })
})
