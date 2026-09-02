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

/**
 * Colour on a button surface has to mean something. Success, info and warning
 * describe an outcome, never the action that starts one, so tinting a button
 * with them says nothing — and four such buttons in a row read as a rainbow
 * toolbar rather than one system. Two families stay allowed: danger, whose
 * colour IS its meaning, and the accent wash, which is the app's neutral hover
 * on every ghost control. Thin accents — borders, text, badges, progress fills
 * — are untouched by this rule; only the surface is.
 */
const DECORATIVE_TINT = /background(-color)?:\s*var\(--color-(success|info|warning)[a-z-]*\)/
const CSS_RULE = /([^{}]+)\{([^{}]*)\}/g

function tintedButtonRulesIn(componentPath: string): string[] {
  const styles = readFileSync(componentPath, 'utf-8').split('<style>').slice(1).join('<style>')

  return Array.from(styles.matchAll(CSS_RULE))
    .map((rule) => [rule[1] ?? '', rule[2] ?? ''] as const)
    .filter(([selector, body]) => /btn|button/i.test(selector) && DECORATIVE_TINT.test(body))
    .map(([selector]) => selector.trim().replace(/\s+/g, ' '))
}

/**
 * The control scale, in pixels: IconButton's containers (24/28/32/40, one per
 * icon step) and Button's heights (30/36/40), plus 44 for the banner step.
 * A button sized off this ladder is the drift that makes two neighbouring
 * controls look like they came from different apps, so a view that hardcodes
 * its own height has to land on a rung.
 */
const CONTROL_SCALE_PX = new Set([24, 28, 30, 32, 36, 40, 44])
const HARDCODED_HEIGHT = /(?:min-)?height:\s*(\d+)px/g

function offScaleButtonHeightsIn(componentPath: string): string[] {
  const styles = readFileSync(componentPath, 'utf-8').split('<style>').slice(1).join('<style>')

  return Array.from(styles.matchAll(CSS_RULE))
    .map((rule) => [rule[1] ?? '', rule[2] ?? ''] as const)
    .filter(([selector]) => /btn|button|action(?!s\b)/i.test(selector))
    .flatMap(([selector, body]) =>
      Array.from(body.matchAll(HARDCODED_HEIGHT))
        .filter((match) => !CONTROL_SCALE_PX.has(Number(match[1])))
        .map((match) => `${selector.trim().replace(/\s+/g, ' ')} -> ${match[0]}`)
    )
}

describe('desktop design tokens', () => {
  it('sizes action buttons from the control scale', () => {
    const offenders = componentsUnder(import.meta.dirname).flatMap((path) =>
      offScaleButtonHeightsIn(path).map((rule) => `${basename(path)}: ${rule}`)
    )

    expect(offenders).toEqual([])
  })

  it('keeps action buttons off decorative semantic tints', () => {
    const offenders = componentsUnder(import.meta.dirname).flatMap((path) =>
      tintedButtonRulesIn(path).map((rule) => `${basename(path)}: ${rule}`)
    )

    expect(offenders).toEqual([])
  })

  it('names only published tokens in var() calls that carry no fallback', () => {
    const published = publishedTokens()
    const offenders = componentsUnder(import.meta.dirname)
      .map((path) => [path, undefinedTokensIn(path, published)] as const)
      .filter(([, missing]) => missing.length > 0)
      .map(([path, missing]) => `${basename(path)}: ${missing.join(', ')}`)

    expect(offenders).toEqual([])
  })
})
