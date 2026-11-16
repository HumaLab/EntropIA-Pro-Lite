import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'

/**
 * The contract every design-system component added or widened by stage C of
 * docs/PLAN_HOMOGENEIZACION.md holds: its own styles carry no colour
 * literals and no fallbacks, its heights come from tokens, every token it
 * names exists in each of the seven themes, and the package exports it.
 */

const COMPONENTS_DIR = resolve(import.meta.dirname, '..')
const TOKENS = readFileSync(resolve(COMPONENTS_DIR, '../tokens/tokens.css'), 'utf-8').replace(
  /\/\*[\s\S]*?\*\//g,
  ''
)
const INDEX = readFileSync(resolve(COMPONENTS_DIR, '../index.ts'), 'utf-8')

const THEME_SELECTORS = [
  ':root',
  ":root[data-theme='dim']",
  ":root[data-theme='light']",
  ":root[data-theme='lite']",
  ":root[data-theme='paper']",
  ":root[data-theme='forest']",
  ":root[data-theme='vivid']",
]

function ruleBody(selector: string): string {
  const at = TOKENS.indexOf(`${selector} {`)
  return at < 0 ? '' : TOKENS.slice(at, TOKENS.indexOf('}', at))
}

const definedIn = (css: string) =>
  new Set([...css.matchAll(/(--[a-z0-9-]+)\s*:/g)].map((m) => m[1]!))

/**
 * What a theme resolves: the default theme, the theme's own block, and the
 * rules that hold for every theme (typography presets and the like).
 */
function themeTokens(selector: string): Set<string> {
  // Every rule except the theme-scoped ones, which only apply to their theme.
  const shared = TOKENS.replace(/:root\[data-theme=[^{]*\{[^}]*\}/g, '')
  return new Set([...definedIn(shared), ...definedIn(ruleBody(selector))])
}

function stylesOf(file: string): string {
  return readFileSync(file, 'utf-8')
    .split(/<style[^>]*>/)
    .slice(1)
    .join('\n')
    .replace(/<\/style>/g, '')
    .replace(/\/\*[\s\S]*?\*\//g, '')
}

function svelteFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    if (entry.name === '__tests__') return []
    const full = resolve(dir, entry.name)
    if (entry.isDirectory()) return svelteFiles(full)
    return entry.name.endsWith('.svelte') ? [full] : []
  })
}

/** Every way the component at `components/<name>` breaks the contract. */
export function componentContractProblems(name: string): string[] {
  const dir = resolve(COMPONENTS_DIR, name)
  if (!existsSync(dir)) return [`${name}: no such component directory`]

  const problems: string[] = []
  const files = svelteFiles(dir)
  if (files.length === 0) problems.push(`${name}: no .svelte file`)

  for (const file of files) {
    const label = `${name}/${file.slice(dir.length + 1).replace(/\\/g, '/')}`
    const source = readFileSync(file, 'utf-8')
    const css = stylesOf(file)
    // Properties the component sets itself (in CSS, `style:--x` or a style
    // string) and hooks named after its own BEM blocks are not system tokens.
    const local = new Set([
      ...definedIn(source),
      ...[...source.matchAll(/style:(--[a-z0-9-]+)/g)].map((m) => m[1]!),
    ])
    const blocks = new Set(
      [...css.matchAll(/\.([a-z][a-z0-9-]*)/g)].map((m) => m[1]!.split('__')[0]!.split('--')[0]!)
    )
    const isOwnHook = (token: string) =>
      [...blocks].some((block) => token.startsWith(`--${block}-`))

    for (const hex of css.match(/(?<![\w-])#[0-9a-fA-F]{3,8}\b/g) ?? []) {
      problems.push(`${label}: colour literal ${hex}`)
    }
    for (const rgba of css.match(/\brgba?\([^)]*\)/g) ?? []) {
      problems.push(`${label}: colour literal ${rgba}`)
    }
    for (const [, token] of css.matchAll(/var\(\s*(--[a-z0-9-]+)\s*,/g)) {
      problems.push(`${label}: fallback on ${token}`)
    }
    for (const [declaration] of css.matchAll(/(?<![\w-])(?:min-|max-)?height\s*:\s*\d+px/g)) {
      problems.push(`${label}: literal height (${declaration.trim()})`)
    }

    const used = new Set([...css.matchAll(/var\(\s*(--[a-z0-9-]+)/g)].map((m) => m[1]!))
    for (const selector of THEME_SELECTORS) {
      const available = themeTokens(selector)
      for (const token of used) {
        if (!available.has(token) && !local.has(token) && !isOwnHook(token)) {
          problems.push(`${label}: ${token} is not defined for ${selector}`)
        }
      }
    }
  }

  if (!INDEX.includes(`./components/${name}/index`)) {
    problems.push(`${name}: not exported from src/index.ts`)
  }

  return problems
}
