import { readdirSync, readFileSync } from 'node:fs'
import { basename, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const TOKEN_DEFINITION = /(--[a-z0-9-]+)\s*:/g
const TOKEN_USE_WITHOUT_FALLBACK = /var\((--[a-z0-9-]+)\s*\)/g

/**
 * Every custom property a desktop view may name: the design system's tokens
 * plus the app's own sheet. A `var()` with no fallback that names anything
 * else is dead on arrival — the browser drops the whole declaration, silently,
 * and the element quietly inherits instead.
 */
function publishedTokens(): Set<string> {
  const tokens = new Set<string>()
  const sheets = [
    ...readdirSync(resolve(import.meta.dirname, '../../../packages/ui/src/tokens'))
      .filter((name) => name.endsWith('.css'))
      .map((name) => resolve(import.meta.dirname, '../../../packages/ui/src/tokens', name)),
    resolve(import.meta.dirname, 'app.css'),
  ]

  for (const sheet of sheets) {
    for (const [, token] of readFileSync(sheet, 'utf-8').matchAll(TOKEN_DEFINITION)) {
      tokens.add(token)
    }
  }

  return tokens
}

function componentsUnder(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = resolve(dir, entry.name)
    if (entry.isDirectory()) return componentsUnder(full)
    return entry.name.endsWith('.svelte') ? [full] : []
  })
}

function undefinedTokensIn(componentPath: string, published: Set<string>): string[] {
  const source = readFileSync(componentPath, 'utf-8')
  const styles = source.split('<style>').slice(1).join('<style>')
  if (!styles) return []

  // A component may declare its own properties; those are not system tokens.
  const local = new Set(Array.from(source.matchAll(TOKEN_DEFINITION), ([, token]) => token))

  return Array.from(
    new Set(
      Array.from(styles.matchAll(TOKEN_USE_WITHOUT_FALLBACK), ([, token]) => token).filter(
        (token) => !published.has(token) && !local.has(token),
      ),
    ),
  )
}

describe('desktop design tokens', () => {
  it('names only published tokens in var() calls that carry no fallback', () => {
    const published = publishedTokens()
    const offenders = componentsUnder(import.meta.dirname)
      .map((path) => [path, undefinedTokensIn(path, published)] as const)
      .filter(([, missing]) => missing.length > 0)
      .map(([path, missing]) => `${basename(path)}: ${missing.join(', ')}`)

    expect(offenders).toEqual([])
  })
})
