import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * Toolbar controls keep the height of their token, whatever the layout around
 * them asks for.
 *
 * A split pane squeezed the collections toolbar into a column, and the
 * search bar's `flex: 1 1 260px` — a WIDTH basis in a row — became a HEIGHT
 * basis: the field grew to 260px and the row's `align-items: stretch` pulled
 * the "new collection" button along with it. Earlier, a `width: 100%` meant
 * for labeled buttons turned the icon-only ones (then `aspect-ratio: 1`) into
 * huge squares. Each was patched in its own view; the next toolbar would have
 * repeated it. So the primitives refuse it: a control's block size is capped
 * at its token (`max-height` equal to `min-height`), and an icon-only button
 * is a fixed square that never flexes. A text button may still take the full
 * width a narrow layout gives it — never more height.
 */
function stylesOf(relativePath: string): string {
  return readFileSync(resolve(import.meta.dirname, relativePath), 'utf-8')
    .split('<style>')
    .slice(1)
    .join('<style>')
    .replace(/\/\*[\s\S]*?\*\//g, '')
}

const CSS_RULE = /([^{}]+)\{([^{}]*)\}/g

/** The declarations of the rule whose selector is exactly `selector`. */
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

function expectCappedAt(styles: string, selector: string, height: string): void {
  const rule = declarations(styles, selector)
  expect({ min: rule['min-height'], max: rule['max-height'] }, selector).toEqual({
    min: height,
    max: height,
  })
}

describe('control primitives keep their token height under any flex or grid parent', () => {
  it('caps every Button size at its control-height token', () => {
    const button = stylesOf('../Button/Button.svelte')

    expectCappedAt(button, '.btn', 'var(--control-height-md)')
    expectCappedAt(button, '.btn--sm', 'var(--control-height-sm)')
    expectCappedAt(button, '.btn--md', 'var(--control-height-md)')
    expectCappedAt(button, '.btn--lg', 'var(--control-height-lg)')
  })

  it('makes an icon-only Button a fixed square that never flexes', () => {
    const button = stylesOf('../Button/Button.svelte')

    expect(declarations(button, '.btn--icon-only')).toMatchObject({ flex: 'none' })
    // `aspect-ratio` is what turned a stretched width into a stretched height.
    expect(declarations(button, '.btn--icon-only')).not.toHaveProperty('aspect-ratio')

    for (const size of ['sm', 'md', 'lg']) {
      const token = `var(--control-height-${size})`
      expect(declarations(button, `.btn--icon-only.btn--${size}`), size).toMatchObject({
        width: token,
        'min-width': token,
        'max-width': token,
      })
    }
  })

  it('makes IconButton a fixed square at every size, driven by one property', () => {
    const iconButton = stylesOf('../IconButton/IconButton.svelte')
    const side = 'var(--icon-button-size)'

    expect(declarations(iconButton, '.icon-button')).toMatchObject({
      flex: 'none',
      width: side,
      'min-width': side,
      'max-width': side,
      height: side,
      'min-height': side,
      'max-height': side,
    })

    const sizes = { xs: '24px', sm: '28px', md: '32px', lg: 'var(--control-height-lg)' }
    for (const [size, px] of Object.entries(sizes)) {
      expect(declarations(iconButton, `.icon-button--${size}`), size).toEqual({
        '--icon-button-size': px,
      })
    }
  })

  it('sizes the search clear button through the IconButton size property', () => {
    const clear = stylesOf('../SearchClearButton/SearchClearButton.svelte')

    expect(
      declarations(clear, ':global(.icon-button.icon-button--sm.search-clear-button)')
    ).toMatchObject({ '--icon-button-size': '24px' })
  })

  it('caps the text input and the search bar at the medium control height', () => {
    expectCappedAt(
      stylesOf('../Input/Input.svelte'),
      '.input-field__input',
      'var(--control-height-md)'
    )
    expectCappedAt(
      stylesOf('../SearchBar/SearchBar.svelte'),
      '.search-bar',
      'var(--control-height-md)'
    )
  })

  it('never lets the ToolbarMenu trigger stretch the button it wraps', () => {
    expect(
      declarations(stylesOf('../ToolbarMenu/ToolbarMenu.svelte'), '.toolbar-menu__trigger')
    ).toMatchObject({
      'align-items': 'center',
      flex: 'none',
    })
  })
})
