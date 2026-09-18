import { readdirSync, readFileSync } from 'node:fs'
import { basename, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * One tooltip system, and no way back to the browser's own.
 *
 * A native `title` is the easiest thing in the world to reach for: it is one
 * attribute, it works, and it looks nothing like the rest of EntropIA — white
 * box, system font, its own delay, and no keyboard behaviour at all. Every one
 * of them now goes through `use:tooltip`, and this is what keeps it that way.
 *
 * Component props happen to share the name — `<ConfirmDialog title="…" />` is a
 * dialog heading, not a tooltip — so only lowercase DOM tags are examined.
 */
function componentsUnder(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = resolve(dir, entry.name)
    if (entry.isDirectory()) {
      return /__tests__|__mocks__|__fixtures__/.test(entry.name) ? [] : componentsUnder(full)
    }
    return entry.name.endsWith('.svelte') ? [full] : []
  })
}

function everyComponent(): string[] {
  return [
    ...componentsUnder(import.meta.dirname),
    ...componentsUnder(resolve(import.meta.dirname, '../../../packages/ui/src')),
  ]
}

/**
 * Tags whose `title` is not a tooltip at all.
 *
 * On embedded content `title` IS the accessible name — ARIA requires it, and a
 * tooltip cannot take its place. Each entry states why, because an exception
 * without a reason is just a hole.
 */
const TITLE_IS_THE_NAME: Record<string, string> = {
  embed: 'title is the accessible name ARIA requires for embedded content',
}

/** End of an opening tag: the first '>' outside braces and outside quotes. */
function tagEnd(source: string, from: number): number {
  let depth = 0
  let quote: string | null = null
  for (let i = from; i < source.length; i++) {
    const c = source[i]
    if (quote) {
      if (c === quote) quote = null
      continue
    }
    if (c === '"' || c === "'" || c === '`') quote = c
    else if (c === '{') depth++
    else if (c === '}') depth--
    else if (c === '>' && depth === 0) return i
  }
  return -1
}

/** `title=` occurrences on lowercase DOM tags, as "file: tag". */
function nativeTitlesIn(componentPath: string): string[] {
  const source = readFileSync(componentPath, 'utf-8')
  const found: string[] = []

  for (let i = source.indexOf('<'); i !== -1; i = source.indexOf('<', i + 1)) {
    const name = source.slice(i + 1).match(/^[a-z][a-z0-9-]*/)?.[0]
    if (!name || name in TITLE_IS_THE_NAME) continue
    const end = tagEnd(source, i)
    if (end === -1) continue
    const head = source.slice(i, end)
    // An attribute, not a substring of another word or of an expression.
    for (let at = head.indexOf('title='); at > 0; at = head.indexOf('title=', at + 1)) {
      const prev = head[at - 1]
      if (prev === ' ' || prev === '\n' || prev === '\t') {
        found.push(`${basename(componentPath)}: <${name}>`)
        break
      }
    }
  }
  return found
}

describe('one tooltip system', () => {
  it('leaves no native title on a DOM element', () => {
    const offenders = everyComponent().flatMap(nativeTitlesIn)

    expect(offenders).toEqual([])
  })

  it('watches enough components for that to mean something', () => {
    // A rule that matched nothing would pass forever.
    expect(everyComponent().length).toBeGreaterThan(60)
  })

  it('states a reason for every tag it excuses', () => {
    const unreasoned = Object.entries(TITLE_IS_THE_NAME).filter(([, why]) => why.trim() === '')

    expect([Object.keys(TITLE_IS_THE_NAME), unreasoned]).toEqual([['embed'], []])
  })
})
