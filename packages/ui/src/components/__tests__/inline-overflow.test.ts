import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * Primitives stay inside the box they are given, whatever its width.
 *
 * Two shapes of the same defect showed up across the app once panels got
 * narrow (the Writing research panel goes down to 240px, split panes to
 * 640px): a nowrap button label longer than its container pushed the button,
 * border and all, past the panel's edge; and a 14px icon beside a long label
 * was shrunk by its flex row into a sliver. Each could be patched per view;
 * the primitives refuse it instead, so the next view does not repeat it.
 */
function stylesOf(relativePath: string): string {
  return readFileSync(resolve(import.meta.dirname, relativePath), 'utf-8')
    .split('<style>')
    .slice(1)
    .join('<style>')
    .replace(/\/\*[\s\S]*?\*\//g, '')
}

const CSS_RULE = /([^{}]+)\{([^{}]*)\}/g

function declarations(styles: string, selector: string): Record<string, string> {
  const rule = Array.from(styles.matchAll(CSS_RULE)).find(
    (match) => (match[1] ?? '').trim().replace(/\s+/g, ' ') === selector
  )
  expect(rule, `${selector} is no longer in the stylesheet`).toBeDefined()

  return Object.fromEntries(
    (rule?.[2] ?? '')
      .split(';')
      .map((declaration) => declaration.split(/:(.*)/s).map((part) => part.trim()))
      .filter(([property, value]) => property && value)
      .map(([property, value]) => [property as string, value as string])
  )
}

describe('primitives never outgrow their container', () => {
  it('caps a text Button at its container width and clips the label inside it', () => {
    const button = stylesOf('../Button/Button.svelte')

    expect(declarations(button, '.btn')['max-width']).toBe('100%')
    // The label is the flex item that gives way; clipped on the inline axis
    // only, so descenders below a tight line-height are never cut.
    expect(declarations(button, '.btn__label')).toMatchObject({
      'min-width': '0',
      'overflow-x': 'clip',
    })
    expect(declarations(button, '.btn__label')).not.toHaveProperty('overflow')
  })

  it('keeps every ActionIcon at the size it was asked for inside a flex row', () => {
    const icon = stylesOf('../Button/ActionIcon.svelte')

    expect(declarations(icon, ':global(svg[data-action-icon])')['flex-shrink']).toBe('0')
  })
})
