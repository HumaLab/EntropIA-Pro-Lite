import { readdirSync, readFileSync } from 'node:fs'
import { relative, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * No responsive rule may stretch a toolbar or a control.
 *
 * The collections toolbar grew into a 260px-tall block inside a narrow split
 * pane: a `@container pane` rule turned the search wrapper into a column,
 * where the search bar's flex basis became a height, and stretched the row so
 * the button followed. Before that, a `width: 100%` in the same kind of rule
 * blew icon-only buttons up into squares. Both hid inside the breakpoint
 * blocks that only fire at a narrow width, which no one looks at while
 * editing a wide window.
 *
 * The primitives now cap their own height (packages/ui
 * control-block-size.test.ts); this keeps the views from asking for it in the
 * first place. Inside any `@container` or `@media` block, a rule whose
 * selector targets a Button, an IconButton, a search bar, an input or a
 * toolbar row may not set:
 *  - `height` / `block-size: 100%`,
 *  - a growing `flex` (`flex: 1`, `flex: 1 1 …`) or a `flex-grow` above 0,
 *  - `align-items: stretch` or `align-self: stretch`,
 *  - `flex-direction: column` on a toolbar row, where every flex basis of its
 *    children turns into a height,
 *  - `max-height: none` on a control, or `max-width: none` on a button: that
 *    lifts the cap the primitive puts on it, and a blanket `.btn` width rule
 *    then stretches an icon-only square into a full-width bar.
 * A toolbar row sizes to its content height; a text control may still take a
 * full `width`.
 */
const SRC = import.meta.dirname

function stylesheets(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = resolve(dir, entry.name)
    if (entry.isDirectory()) {
      return /^(__mocks__|__fixtures__|test|pane-context-fixtures)$/.test(entry.name)
        ? []
        : stylesheets(full)
    }
    return /\.(svelte|css)$/.test(entry.name) ? [full] : []
  })
}

function cssOf(path: string): string {
  const source = readFileSync(path, 'utf-8')
  const css = path.endsWith('.css')
    ? source
    : Array.from(source.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g), (m) => m[1] ?? '').join('\n')
  return css.replace(/\/\*[\s\S]*?\*\//g, '')
}

interface Rule {
  selector: string
  body: string
  atRules: string[]
}

/** Every style rule with the at-rules that enclose it, outermost first. */
function rulesOf(css: string): Rule[] {
  const rules: Rule[] = []
  const stack: string[] = []
  let prelude = ''
  for (let i = 0; i < css.length; i++) {
    const char = css[i]
    if (char === '{') {
      const head = prelude.trim()
      prelude = ''
      if (head.startsWith('@')) {
        stack.push(head)
        continue
      }
      const close = css.indexOf('}', i)
      rules.push({ selector: head, body: css.slice(i + 1, close), atRules: [...stack] })
      i = close
    } else if (char === '}') {
      stack.pop()
      prelude = ''
    } else if (char === ';' && !prelude.includes('{')) {
      // A statement at-rule (`@import …;`) or a stray declaration.
      prelude = ''
    } else {
      prelude += char
    }
  }
  return rules
}

const CONTROL = /\.btn\b|\.icon-button\b|\.search-bar\b|input-field__input|\binput\b|\bselect\b/
const TOOLBAR = /toolbar|controls|__actions\b|__search\b/

const GROWING_FLEX = /(?:^|;)\s*flex:\s*(?:[1-9]\d*(?:\.\d+)?|0?\.\d*[1-9])(?:\s|;|$)/
const FLEX_GROW = /(?:^|;)\s*flex-grow:\s*(?!0(?:\.0*)?\s*(?:;|$))[\d.]+/
const FULL_HEIGHT = /(?:^|;)\s*(?:height|block-size):\s*100%/
const STRETCH = /(?:^|;)\s*align-(?:items|self):\s*stretch/
const COLUMN = /(?:^|;)\s*flex-direction:\s*column/
const UNCAPPED_HEIGHT = /(?:^|;)\s*max-(?:height|block-size):\s*none/
const UNCAPPED_WIDTH = /(?:^|;)\s*max-(?:width|inline-size):\s*none/
const BUTTON = /\.btn\b|\.icon-button\b/

/**
 * Rules that stretch on purpose. Each entry names why; an exception with no
 * reason is just a hole.
 */
const ALLOWED: Record<string, string> = {}

function offendersIn(path: string): string[] {
  return rulesOf(cssOf(path))
    .filter((rule) => rule.atRules.some((at) => /^@(container|media)\b/.test(at)))
    .flatMap((rule) => {
      const selector = rule.selector.replace(/\s+/g, ' ')
      const control = CONTROL.test(selector)
      const toolbar = TOOLBAR.test(selector)
      if (!control && !toolbar) return []

      const body = rule.body.replace(/\s+/g, ' ')
      const found = [
        FULL_HEIGHT.test(body) && 'height: 100%',
        GROWING_FLEX.test(body) && 'growing flex',
        FLEX_GROW.test(body) && 'flex-grow',
        STRETCH.test(body) && 'align stretch',
        toolbar && COLUMN.test(body) && 'column toolbar',
        control && UNCAPPED_HEIGHT.test(body) && 'uncapped height',
        BUTTON.test(selector) && UNCAPPED_WIDTH.test(body) && 'uncapped button width',
      ].filter((hit): hit is string => Boolean(hit))

      const key = `${relative(SRC, path).replace(/\\/g, '/')}: ${selector}`
      return found.length && !(key in ALLOWED) ? [`${key} -> ${found.join(', ')}`] : []
    })
}

describe('responsive rules never stretch a toolbar or a control', () => {
  const files = stylesheets(SRC)

  it('finds the stylesheets it guards', () => {
    // A walk that matched nothing would pass forever.
    expect(files.some((file) => file.endsWith('CollectionsView.svelte'))).toBe(true)
    expect(files.some((file) => file.endsWith('app.css'))).toBe(true)
  })

  it('reads rules nested in @container and @media blocks', () => {
    const rules = rulesOf(
      '.a { flex: 1; } @container pane (max-width: 720px) { .b :global(.btn) { flex: 1; } }'
    )
    expect(rules.map((rule) => [rule.selector, rule.atRules])).toEqual([
      ['.a', []],
      ['.b :global(.btn)', ['@container pane (max-width: 720px)']],
    ])
  })

  it('has no stretching rule on a toolbar or a control inside a breakpoint', () => {
    expect(files.flatMap(offendersIn)).toEqual([])
  })
})
