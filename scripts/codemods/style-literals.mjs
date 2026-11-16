#!/usr/bin/env node
// Mechanical literal -> token replacements for the homogenization plan
// (docs/PLAN_HOMOGENEIZACION.md, step 5). It rewrites only the <style> blocks
// of .svelte files under apps/desktop/src and packages/ui/src, skipping tests,
// mocks and fixtures, and leaves CSS comments alone:
//
//   var(--token, fallback)       -> var(--token)        token defined in tokens.css
//   font-weight: 500 | 600       -> var(--font-weight-medium | semibold)
//   font-size: 11px              -> var(--font-size-2xs)
//   border-radius 4px            -> var(--radius-xs)
//   border-radius 999px | 9999px -> var(--radius-full)
//   padding/margin/gap 4..32px   -> the exact --space-* step
//   letter-spacing: 0.075em      -> var(--letter-spacing-caps)
//
// Everything that needs judgement (2px, 6px, 0.9em, line-height, negative
// offsets) is left as it is. Running it twice produces no further change.
//
// Usage:
//   node scripts/codemods/style-literals.mjs           rewrite in place, list files
//   node scripts/codemods/style-literals.mjs --check   exit 1 if anything would change
//   node scripts/codemods/style-literals.mjs --root <dir>

import { readFileSync, readdirSync, statSync, writeFileSync } from 'node:fs'
import { join, relative, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

const SOURCE_DIRS = ['apps/desktop/src', 'packages/ui/src']
const TOKEN_SHEET = 'packages/ui/src/tokens/tokens.css'
const SKIPPED_DIRS = new Set(['__tests__', '__mocks__', '__fixtures__', 'node_modules'])

const SPACE = { 4: 1, 8: 2, 12: 3, 16: 4, 20: 5, 24: 6, 32: 8 }
const SPACING_PROPERTY = /^(?:(?:padding|margin)(?:-[a-z-]+)?|(?:row-|column-)?gap)$/
const RADIUS_PROPERTY = /^border(?:-[a-z]+)*-radius$/
// A length that stands alone: not part of a larger number, an identifier or a negative.
const standalone = (n) => new RegExp(`(?<![\\w.#-])${n}px(?![\\w.-])`, 'g')

function walk(dir, out) {
  for (const name of readdirSync(dir)) {
    if (SKIPPED_DIRS.has(name)) continue
    const path = join(dir, name)
    if (statSync(path).isDirectory()) walk(path, out)
    else if (name.endsWith('.svelte') && !/\.test\./.test(name)) out.push(path)
  }
  return out
}

export function publishedTokens(tokensCss) {
  const css = tokensCss.replace(/\/\*[\s\S]*?\*\//g, '')
  return new Set([...css.matchAll(/(--[a-zA-Z0-9_-]+)\s*:/g)].map((m) => m[1]))
}

/** Drops the fallback of every var() that names a published token. */
function dropPublishedFallbacks(code, published) {
  let out = ''
  let i = 0
  const opener = /var\(\s*(--[a-zA-Z0-9_-]+)\s*,/g
  for (;;) {
    opener.lastIndex = i
    const m = opener.exec(code)
    if (!m) break
    if (!published.has(m[1])) {
      out += code.slice(i, m.index + 4)
      i = m.index + 4
      continue
    }
    // Find the parenthesis that closes this var(.
    let depth = 1
    let j = m.index + m[0].length
    for (; j < code.length && depth > 0; j++) {
      if (code[j] === '(') depth++
      else if (code[j] === ')') depth--
    }
    if (depth !== 0) break
    out += code.slice(i, m.index) + `var(${m[1]})`
    i = j
  }
  return out + code.slice(i)
}

function rewriteValue(property, value) {
  const trimmed = value.trim()
  if (property === 'font-weight') {
    if (/^500(\s*!important)?$/.test(trimmed)) return value.replace('500', 'var(--font-weight-medium)')
    if (/^600(\s*!important)?$/.test(trimmed))
      return value.replace('600', 'var(--font-weight-semibold)')
    return value
  }
  if (property === 'font-size') {
    return /^11px(\s*!important)?$/.test(trimmed)
      ? value.replace('11px', 'var(--font-size-2xs)')
      : value
  }
  if (property === 'letter-spacing') {
    return /^0\.075em(\s*!important)?$/.test(trimmed)
      ? value.replace('0.075em', 'var(--letter-spacing-caps)')
      : value
  }
  if (RADIUS_PROPERTY.test(property)) {
    return value
      .replace(standalone(9999), 'var(--radius-full)')
      .replace(standalone(999), 'var(--radius-full)')
      .replace(standalone(4), 'var(--radius-xs)')
  }
  if (SPACING_PROPERTY.test(property)) {
    let next = value
    for (const [px, step] of Object.entries(SPACE)) {
      next = next.replace(standalone(px), `var(--space-${step})`)
    }
    return next
  }
  return value
}

/** Rewrites declarations in CSS that carries no comments. */
function rewriteCode(code, published) {
  const withoutFallbacks = dropPublishedFallbacks(code, published)
  return withoutFallbacks.replace(
    /(^|[{;\s])([a-z-]+)(\s*:)([^;{}]+)(?=[;}])/g,
    (whole, lead, property, colon, value) => {
      if (property.startsWith('--')) return whole
      return lead + property + colon + rewriteValue(property, value)
    }
  )
}

/** Applies the rewrite to the code between comments, never inside one. */
export function rewriteCss(css, published) {
  return css
    .split(/(\/\*[\s\S]*?\*\/)/)
    .map((part) => (part.startsWith('/*') ? part : rewriteCode(part, published)))
    .join('')
}

export function rewriteSvelte(source, published) {
  return source.replace(
    /(<style\b[^>]*>)([\s\S]*?)(<\/style>)/g,
    (_, open, css, close) => open + rewriteCss(css, published) + close
  )
}

function main(argv) {
  const check = argv.includes('--check')
  const rootAt = argv.indexOf('--root')
  const root = resolve(rootAt >= 0 ? argv[rootAt + 1] : process.cwd())
  const published = publishedTokens(readFileSync(join(root, TOKEN_SHEET), 'utf8'))

  const changed = []
  for (const file of SOURCE_DIRS.flatMap((dir) => walk(join(root, dir), []))) {
    const source = readFileSync(file, 'utf8')
    const next = rewriteSvelte(source, published)
    if (next === source) continue
    changed.push(relative(root, file).split(sep).join('/'))
    if (!check) writeFileSync(file, next)
  }

  for (const file of changed) console.log(file)
  console.log(`${changed.length} file(s) ${check ? 'would change' : 'changed'}`)
  if (check && changed.length > 0) process.exitCode = 1
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main(process.argv.slice(2))
}
