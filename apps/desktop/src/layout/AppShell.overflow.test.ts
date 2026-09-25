import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * `.content` is the shell's scroll container (`overflow-y: auto`, which turns
 * the unset x axis to `auto` too). It used to carry an inline padding that its
 * split-view wrapper cancelled with `margin-inline: calc(-1 * var(--space-5))`.
 * Engines disagree on whether that cancelled inline-end padding still counts
 * as scrollable overflow: the macOS WKWebView counted it, so the Lite preview
 * on an Intel Mac (classic, always-visible scrollbars) showed a full-width
 * horizontal scrollbar right above the status bar. happy-dom does no layout,
 * so this guards the stylesheet: the scroll container has no inline padding
 * for a child to cancel, and each child brings its own inset instead.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'AppShell.svelte'), 'utf-8')
const STYLES = SOURCE.slice(SOURCE.indexOf('<style>')).replace(/\/\*[\s\S]*?\*\//g, '')

function ruleFor(selector: string): string {
  const match = new RegExp(`(^|\\n)\\s*${selector.replace(/[.]/g, '\\.')}\\s*\\{([^}]*)\\}`).exec(
    STYLES
  )
  expect(match, `${selector} is no longer in the stylesheet`).not.toBeNull()
  return match?.[2] ?? ''
}

describe('AppShell horizontal overflow', () => {
  it('gives the scroll container no inline padding', () => {
    const content = ruleFor('.content')
    expect(content).toMatch(/overflow-y:\s*auto/)
    expect(content).not.toMatch(/padding-(inline|left|right)\s*:/)
    const padding = /(?:^|\s)padding\s*:\s*([^;]+);/.exec(content)?.[1]?.trim().split(/\s+/) ?? []
    // `padding: <block> <inline> …`: every inline value must be zero.
    const inline = padding.length === 1 ? [padding[0]] : [padding[1], padding[3] ?? padding[1]]
    for (const value of inline.filter(Boolean)) {
      expect(value).toMatch(/^0(px)?$/)
    }
  })

  it('cancels no ancestor padding with a negative inline margin', () => {
    expect(STYLES).not.toMatch(/margin(-inline|-left|-right)?\s*:[^;]*(calc\(\s*-|\s-\d|\s-var)/)
  })

  it('keeps the same inset on the panes and on the notices above them', () => {
    expect(ruleFor('.content__pane')).toMatch(/padding-inline:\s*var\(--space-5\)/)
    expect(ruleFor('.store-update')).toMatch(/margin-inline:\s*var\(--space-5\)/)
    expect(ruleFor('.deps-banner')).toMatch(/margin-inline:\s*var\(--space-5\)/)
  })
})
