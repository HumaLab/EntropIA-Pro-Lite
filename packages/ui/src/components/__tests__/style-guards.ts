import { readdirSync, readFileSync } from 'node:fs'
import { relative, resolve, sep } from 'node:path'

/**
 * Style guards for the homogenization plan (docs/PLAN_HOMOGENEIZACION.md,
 * step 3). Each rule names one kind of style debt. A file that breaks a rule
 * fails the guard unless the pair (file, rule) is listed in the allowlist JSON
 * of the plan step that clears it, under ./style-allowlists. Those lists only
 * shrink: a step that clears a pair removes it in the same change.
 *
 * apps/desktop keeps a twin of this module for its own components; the two
 * packages never scan each other's files.
 */

export const STYLE_RULES = [
  /** Hex, rgb() or rgba() written into a stylesheet instead of a token. */
  'color-literal',
  /** color-mix() towards black or white, which only suits one kind of theme. */
  'color-mix-black-white',
  /** var() naming a custom property nothing defines, with or without fallback. */
  'unknown-token',
  /** var() with a fallback on a published token: the fallback hides typos. */
  'published-fallback',
  /** A button or submit rule poured in --color-accent: primary is Button's job. */
  'accent-fill',
] as const

export type StyleRule = (typeof STYLE_RULES)[number]

/** One allowlist file: rule → files (relative to the package's src). */
export type Allowlist = Partial<Record<StyleRule, string[]>>

const TOKEN_DEFINITION = /(--[a-zA-Z0-9_-]+)\s*:/g
const VAR_CALL = /var\(\s*(--[a-zA-Z0-9_-]+)\s*(,)?/g
const HEX = /(?<![\w-])#[0-9a-fA-F]{3,8}\b(?![\w-]*\s*[{,])/
const RGB = /\brgba?\(/
const COLOR_MIX_BLACK_WHITE = /color-mix\((?:[^()]|\([^()]*\))*\b(?:black|white)\b/
const CSS_RULE = /([^{}]+)\{([^{}]*)\}/g
const ACCENT_FILL = /background(-color)?:\s*var\(--color-accent(-hover)?\)/
const ACCENT_SUBJECT = /btn|button|submit/i

function names(text: string, pattern: RegExp): string[] {
  return Array.from(text.matchAll(pattern), (match) => match[1]).filter(
    (name): name is string => name !== undefined
  )
}

/** The <style> blocks of a component, comments removed. */
export function stylesOf(source: string): string {
  return Array.from(source.matchAll(/<style\b[^>]*>([\s\S]*?)<\/style>/g), (m) => m[1] ?? '')
    .join('\n')
    .replace(/\/\*[\s\S]*?\*\//g, '')
}

/** BEM block names a stylesheet styles (`.deps-row__icon--ok` → `deps-row`). */
function ownBlocks(styles: string): string[] {
  return names(styles, /\.([a-z][a-z0-9-]*)/g).map((cls) => cls.split('__')[0]!.split('--')[0]!)
}

export interface TokenContext {
  /** Tokens the design system publishes from its stylesheets. */
  published: Set<string>
  /** Every custom property defined anywhere in the package or its sheets. */
  defined: Set<string>
}

/** The rules a component breaks, judged from its source alone. */
export function rulesBrokenBy(source: string, ctx: TokenContext): Set<StyleRule> {
  const styles = stylesOf(source)
  const broken = new Set<StyleRule>()
  if (!styles) return broken

  if (HEX.test(styles) || RGB.test(styles)) broken.add('color-literal')
  if (COLOR_MIX_BLACK_WHITE.test(styles)) broken.add('color-mix-black-white')

  // A property named after one of the file's own blocks is a hook its parent
  // may set (`--number-field-max-width`), not a design token.
  const blocks = ownBlocks(styles)
  const isHook = (name: string) => blocks.some((block) => name.startsWith(`--${block}-`))
  for (const match of styles.matchAll(VAR_CALL)) {
    const name = match[1] ?? ''
    if (!ctx.defined.has(name) && !isHook(name)) broken.add('unknown-token')
    if (match[2] && ctx.published.has(name)) broken.add('published-fallback')
  }

  for (const rule of styles.matchAll(CSS_RULE)) {
    if (ACCENT_SUBJECT.test(rule[1] ?? '') && ACCENT_FILL.test(rule[2] ?? '')) {
      broken.add('accent-fill')
    }
  }
  return broken
}

function filesUnder(dir: string, pattern: RegExp): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    if (entry.name === '__tests__' || entry.name === '__mocks__') return []
    const full = resolve(dir, entry.name)
    if (entry.isDirectory()) return filesUnder(full, pattern)
    return pattern.test(entry.name) && !entry.name.includes('.test.') ? [full] : []
  })
}

export interface PackageScan {
  /** "file::rule" for every rule a component of the package breaks today. */
  failing: Set<string>
  ctx: TokenContext
}

/**
 * Scans one package for `rules`. `sheets` publish tokens; the first one is the
 * design system's token sheet, the rest add app-level properties.
 */
export function scanPackage(
  root: string,
  sheets: string[],
  rules: readonly StyleRule[] = STYLE_RULES
): PackageScan {
  const published = new Set<string>()
  const defined = new Set<string>()
  sheets.forEach((sheet, index) => {
    for (const name of names(readFileSync(sheet, 'utf-8'), TOKEN_DEFINITION)) {
      if (index === 0) published.add(name)
      defined.add(name)
    }
  })
  // Properties set from markup or from TS (`style: \`--writing-indent: ${n}\``)
  // exist at runtime.
  for (const file of filesUnder(root, /\.(svelte|ts)$/)) {
    const text = readFileSync(file, 'utf-8')
    for (const name of names(text, TOKEN_DEFINITION)) defined.add(name)
    for (const name of names(text, /style:(--[a-zA-Z0-9_-]+)/g)) defined.add(name)
  }

  const ctx = { published, defined }
  const failing = new Set<string>()
  for (const file of filesUnder(root, /\.svelte$/)) {
    const rel = relative(root, file).split(sep).join('/')
    for (const rule of rulesBrokenBy(readFileSync(file, 'utf-8'), ctx)) {
      if (rules.includes(rule)) failing.add(`${rel}::${rule}`)
    }
  }
  return { failing, ctx }
}

const ALLOWLIST_FILE = /^(paso-\d+|resto)\.json$/

/** Every allowlist JSON in a directory, by file name. */
export function readAllowlists(dir: string): Record<string, Allowlist> {
  const lists: Record<string, Allowlist> = {}
  for (const name of readdirSync(dir).filter((file) => file.endsWith('.json'))) {
    lists[name] = JSON.parse(readFileSync(resolve(dir, name), 'utf-8')) as Allowlist
  }
  return lists
}

/**
 * Problems with a set of allowlists against today's failures: a badly named
 * file, an unknown rule, a pair listed twice, a pair that no longer fails
 * (stale), or a failure no list covers.
 */
export function allowlistProblems(
  lists: Record<string, Allowlist>,
  failing: Set<string>
): string[] {
  const problems: string[] = []
  const seen = new Map<string, string>()

  for (const [fileName, list] of Object.entries(lists)) {
    if (!ALLOWLIST_FILE.test(fileName)) problems.push(`${fileName}: not paso-N.json or resto.json`)
    for (const [rule, files] of Object.entries(list)) {
      if (!(STYLE_RULES as readonly string[]).includes(rule)) {
        problems.push(`${fileName}: unknown rule ${rule}`)
        continue
      }
      for (const file of files ?? []) {
        const pair = `${file}::${rule}`
        const owner = seen.get(pair)
        if (owner) problems.push(`${pair}: listed in both ${owner} and ${fileName}`)
        else seen.set(pair, fileName)
        if (!failing.has(pair)) problems.push(`${pair}: listed in ${fileName} but no longer fails`)
      }
    }
  }
  for (const pair of failing) {
    if (!seen.has(pair)) problems.push(`${pair}: fails and no allowlist covers it`)
  }
  return problems
}
