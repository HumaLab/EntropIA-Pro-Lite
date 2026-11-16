#!/usr/bin/env node
// Style metrics for the homogenization plan (docs/PLAN_HOMOGENEIZACION.md, step 1).
// Counts the audit's metrics (docs/AUDITORIA_ESTILOS.md §1) in the <style> blocks and
// markup of apps/desktop/src and packages/ui/src, skipping tests and mocks.
//
// Usage:
//   node scripts/style-metrics.mjs                 human-readable totals
//   node scripts/style-metrics.mjs --json          totals and per-file counts as JSON
//   node scripts/style-metrics.mjs --file <path>   counts for one file (repeatable)
//   node scripts/style-metrics.mjs --root <dir>    measure another checkout

import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join, relative, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

const SOURCE_DIRS = ['apps/desktop/src', 'packages/ui/src']
const TOKEN_SHEETS = ['packages/ui/src/tokens/tokens.css', 'apps/desktop/src/app.css']
const UI_COMPONENTS_DIR = 'packages/ui/src/components'
const SKIPPED_DIRS = new Set(['__tests__', '__mocks__', 'node_modules'])

/** Metric keys, in report order, with the label printed in the text report. */
export const METRICS = [
  ['hex', 'Hex literals in <style>'],
  ['rgba', 'rgb[a]() literals in <style>'],
  ['colorMixBlackWhite', 'color-mix() with black/white'],
  ['unknownTokens', 'var() to tokens that do not exist'],
  ['fallbacks', 'var() with a fallback (any token)'],
  ['publishedFallbacks', 'var() with a fallback on a published token'],
  ['fontSize', 'font-size literals (px, em, rem)'],
  ['fontWeight', 'font-weight numeric literals'],
  ['lineHeight', 'line-height literals'],
  ['letterSpacing', 'letter-spacing literals'],
  ['disabledOpacity', 'opacity literals in :disabled rules'],
  ['zIndex', 'z-index literals'],
  ['transitions', 'transition/animation declarations with literal durations'],
  ['globalOverrides', ':global() over design-system components (outside packages/ui)'],
  ['rawInput', '<input> in markup (outside packages/ui, radios included)'],
  ['textarea', '<textarea> in markup'],
  ['progress', '<progress> in markup'],
  ['details', '<details> in markup'],
  ['radio', 'radio <input> in markup'],
]

/** Metrics whose distinct literal values are also reported. */
const DISTINCT = ['unknownTokens', 'lineHeight', 'letterSpacing', 'disabledOpacity', 'zIndex']

function walk(dir, out, pattern = /\.svelte$/) {
  for (const name of readdirSync(dir)) {
    if (SKIPPED_DIRS.has(name)) continue
    const path = join(dir, name)
    if (statSync(path).isDirectory()) walk(path, out, pattern)
    else if (pattern.test(name) && !/\.test\./.test(name)) out.push(path)
  }
  return out
}

const stripCssComments = (css) => css.replace(/\/\*[\s\S]*?\*\//g, '')

function splitSvelte(source) {
  const styles = []
  const markup = source
    .replace(/<style\b[^>]*>([\s\S]*?)<\/style>/g, (_, css) => {
      styles.push(css)
      return ''
    })
    .replace(/<script\b[^>]*>[\s\S]*?<\/script>/g, '')
    .replace(/<!--[\s\S]*?-->/g, '')
  return { css: stripCssComments(styles.join('\n')), markup }
}

/** Innermost CSS rules as { selector, body }; at-rule wrappers are flattened. */
function cssRules(css) {
  const rules = []
  const stack = []
  let start = 0
  for (let i = 0; i < css.length; i++) {
    const ch = css[i]
    if (ch === '{') {
      stack.push({ selector: css.slice(start, i).trim(), bodyStart: i + 1, hasChild: false })
      if (stack.length > 1) stack[stack.length - 2].hasChild = true
      start = i + 1
    } else if (ch === '}') {
      const frame = stack.pop()
      if (frame && !frame.hasChild && !frame.selector.startsWith('@')) {
        rules.push({ selector: frame.selector, body: css.slice(frame.bodyStart, i) })
      }
      start = i + 1
    } else if (ch === ';' && stack.length > 0 && !stack[stack.length - 1].hasChild) {
      // declarations; nothing to track
    }
  }
  return rules
}

const count = (text, re) => (text.match(re) ?? []).length
const values = (text, re) => [...text.matchAll(re)].map((m) => m[1])

function definedProperties(text) {
  return values(text, /(--[a-zA-Z0-9_-]+)\s*:/g)
}

/** Block names (BEM) that design-system components style for themselves. */
function systemBlocks(root) {
  const blocks = new Set()
  const dir = join(root, UI_COMPONENTS_DIR)
  for (const file of walk(dir, [])) {
    const { css } = splitSvelte(readFileSync(file, 'utf8'))
    for (const rule of cssRules(css)) {
      for (const cls of values(
        rule.selector.replace(/:global\(([^)]*)\)/g, '$1'),
        /\.([a-z][a-z0-9-]*)/g
      )) {
        blocks.add(cls.split('__')[0].split('--')[0])
      }
    }
  }
  return blocks
}

function measureFile(path, ctx) {
  const { css, markup } = splitSvelte(readFileSync(path, 'utf8'))
  const rel = relative(ctx.root, path).split(sep).join('/')
  const inUi = rel.startsWith('packages/ui/')
  const m = Object.fromEntries(METRICS.map(([key]) => [key, 0]))
  const distinct = Object.fromEntries(DISTINCT.map((key) => [key, []]))

  m.hex = count(css, /(?<![\w-])#[0-9a-fA-F]{3,8}\b(?![\w-]*\s*[{,])/g)
  m.rgba = count(css, /\brgba?\(/g)
  m.colorMixBlackWhite = count(css, /color-mix\((?:[^()]|\([^()]*\))*\b(?:black|white)\b/g)

  // A custom property named after one of the file's own BEM blocks
  // (`--number-field-max-width` in NumberField) is a hook its parent may set,
  // not a design token, so leaving it undefined is not a missing token.
  const ownBlocks = new Set(
    values(css, /\.([a-z][a-z0-9-]*)/g).map((cls) => cls.split('__')[0].split('--')[0])
  )
  const isOwnHook = (name) => [...ownBlocks].some((block) => name.startsWith(`--${block}-`))
  for (const [, name, fallback] of css.matchAll(/var\(\s*(--[a-zA-Z0-9_-]+)\s*(,)?/g)) {
    if (!ctx.defined.has(name) && !isOwnHook(name)) distinct.unknownTokens.push(name)
    if (fallback) {
      m.fallbacks++
      if (ctx.published.has(name)) m.publishedFallbacks++
    }
  }

  m.unknownTokens = distinct.unknownTokens.length

  m.fontSize = count(css, /font-size\s*:\s*[0-9.]+(?:px|r?em)\b/g)
  m.fontWeight = count(css, /font-weight\s*:\s*\d{3}\b/g)

  distinct.lineHeight = values(css, /line-height\s*:\s*([0-9.]+(?:px|em|rem|%)?)\s*[;}\n!]/g)
  m.lineHeight = distinct.lineHeight.length
  distinct.letterSpacing = values(css, /letter-spacing\s*:\s*(-?[0-9.]+(?:px|em|rem)?)/g)
  m.letterSpacing = distinct.letterSpacing.length
  distinct.zIndex = values(css, /z-index\s*:\s*(-?\d+)/g)
  m.zIndex = distinct.zIndex.length

  m.transitions = count(
    css,
    /(?:transition|animation)(?:-duration)?\s*:[^;}]*?(?<![\w-])\d*\.?\d+m?s\b[^;}]*/g
  )

  for (const rule of cssRules(css)) {
    if (/:disabled|\[disabled\]|aria-disabled|--disabled\b/.test(rule.selector)) {
      distinct.disabledOpacity.push(...values(rule.body, /(?<![\w-])opacity\s*:\s*([0-9.]+)/g))
    }
    if (!inUi) {
      // One count per `:global(...)` that names a design-system class.
      for (const inner of values(rule.selector, /:global\(([^)]*)\)/g)) {
        const hit = values(inner, /\.([a-z][a-z0-9-]*)/g).some((cls) =>
          ctx.blocks.has(cls.split('__')[0].split('--')[0])
        )
        if (hit) m.globalOverrides++
      }
    }
  }
  m.disabledOpacity = distinct.disabledOpacity.length

  if (!inUi) m.rawInput = count(markup, /<input\b/g)
  m.textarea = count(markup, /<textarea\b/g)
  m.progress = count(markup, /<progress\b/g)
  m.details = count(markup, /<details\b/g)
  m.radio = count(markup, /<input\b[^>]*\btype\s*=\s*["']radio["']/g)

  return { rel, metrics: m, distinct }
}

export function measure(root) {
  const sources = SOURCE_DIRS.flatMap((dir) => walk(join(root, dir), []))
  const sheetText = TOKEN_SHEETS.map((p) => stripCssComments(readFileSync(join(root, p), 'utf8')))
  const published = new Set(definedProperties(sheetText[0]))
  const defined = new Set(sheetText.flatMap(definedProperties))
  // Properties set from markup (`style:--x`, `style="--x: …"`) or from TS
  // (`style: \`--writing-indent: ${level}\``) exist at runtime.
  const scripts = SOURCE_DIRS.flatMap((dir) => walk(join(root, dir), [], /\.ts$/))
  for (const file of [...sources, ...scripts]) {
    const text = readFileSync(file, 'utf8')
    for (const name of definedProperties(text)) defined.add(name)
    for (const name of values(text, /style:(--[a-zA-Z0-9_-]+)/g)) defined.add(name)
  }
  const ctx = { root, published, defined, blocks: systemBlocks(root) }

  const files = {}
  const totals = Object.fromEntries(METRICS.map(([key]) => [key, 0]))
  const distinct = Object.fromEntries(DISTINCT.map((key) => [key, new Set()]))
  for (const file of sources) {
    const result = measureFile(file, ctx)
    const nonZero = Object.entries(result.metrics).filter(([, n]) => n > 0)
    if (nonZero.length > 0) files[result.rel] = Object.fromEntries(nonZero)
    for (const [key, n] of Object.entries(result.metrics)) totals[key] += n
    for (const key of DISTINCT) for (const v of result.distinct[key]) distinct[key].add(v)
  }
  const distinctValues = Object.fromEntries(
    DISTINCT.map((key) => [key, [...distinct[key]].sort((a, b) => parseFloat(a) - parseFloat(b))])
  )
  return { files: Object.keys(files).length, totals, distinctValues, perFile: files }
}

function main(argv) {
  const args = argv.slice(2)
  const json = args.includes('--json')
  const rootIndex = args.indexOf('--root')
  const root = resolve(
    rootIndex >= 0 ? args[rootIndex + 1] : join(fileURLToPath(import.meta.url), '..', '..')
  )
  const wanted = args.flatMap((arg, i) => (arg === '--file' ? [args[i + 1]] : []))
  const result = measure(root)

  if (wanted.length > 0) {
    const out = {}
    for (const path of wanted) {
      const rel = relative(root, resolve(root, path)).split(sep).join('/')
      const known = SOURCE_DIRS.some((dir) => rel.startsWith(`${dir}/`)) && rel.endsWith('.svelte')
      if (!known) {
        console.error(
          `style-metrics: ${path} is not a .svelte file under ${SOURCE_DIRS.join(' or ')}`
        )
        process.exitCode = 2
        continue
      }
      out[rel] = Object.fromEntries(METRICS.map(([key]) => [key, result.perFile[rel]?.[key] ?? 0]))
    }
    if (json) console.log(JSON.stringify(out, null, 2))
    else
      for (const [rel, metrics] of Object.entries(out)) {
        console.log(rel)
        for (const [key, label] of METRICS) console.log(`  ${label}: ${metrics[key]}`)
      }
    return
  }

  if (json) {
    console.log(JSON.stringify(result, null, 2))
    return
  }
  console.log(`Files with findings: ${result.files}`)
  for (const [key, label] of METRICS) console.log(`${label}: ${result.totals[key]}`)
  for (const key of DISTINCT) {
    const list = result.distinctValues[key]
    console.log(`Distinct ${key} values (${list.length}): ${list.join(', ')}`)
  }
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url))
  main(process.argv)
