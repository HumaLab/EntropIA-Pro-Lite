import { readdirSync, readFileSync } from 'node:fs'
import { basename, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

function readSource(relativePath: string): string {
  return readFileSync(resolve(import.meta.dirname, relativePath), 'utf-8')
}

const TOKEN_DEFINITION = /(--[a-z0-9-]+)\s*:/g
const TOKEN_USE_WITHOUT_FALLBACK = /var\((--[a-z0-9-]+)\s*\)/g

/**
 * The token names a pattern captured. Group 1 is not optional in any pattern
 * here — a match without it cannot occur — but an index into a match array is
 * still typed as possibly undefined, so it is narrowed once, in one place,
 * rather than at every call site.
 */
function capturedNames(matches: Iterable<RegExpMatchArray>): string[] {
  return Array.from(matches, ([, name]) => name).filter(
    (name): name is string => name !== undefined
  )
}

/**
 * Every custom property the design system publishes. A `var()` with no
 * fallback that names anything else is dead on arrival: the browser drops the
 * whole declaration, silently, and the element quietly inherits instead.
 */
function definedTokens(): Set<string> {
  const tokensDir = resolve(import.meta.dirname, '../../tokens')
  const tokens = new Set<string>()

  for (const file of readdirSync(tokensDir).filter((name) => name.endsWith('.css'))) {
    const css = readFileSync(resolve(tokensDir, file), 'utf-8')
    for (const token of capturedNames(css.matchAll(TOKEN_DEFINITION))) tokens.add(token)
  }

  return tokens
}

function undefinedTokensIn(componentPath: string, published: Set<string>): string[] {
  const source = readFileSync(componentPath, 'utf-8')
  const styles = source.split('<style>').slice(1).join('<style>')
  if (!styles) return []

  // A component may declare its own properties; those are not system tokens.
  const local = new Set(capturedNames(source.matchAll(TOKEN_DEFINITION)))

  return Array.from(
    new Set(
      capturedNames(styles.matchAll(TOKEN_USE_WITHOUT_FALLBACK)).filter(
        (token) => !published.has(token) && !local.has(token)
      )
    )
  )
}

/** The declarations of one theme scope, up to its closing brace. */
function themeBlock(css: string, selector: string): string {
  const start = css.indexOf(selector)
  if (start < 0) return ''
  return css.slice(start, css.indexOf('}', start))
}

const ACCENT_FILL = /background(-color)?:\s*var\(--color-accent(-hover)?\)/
const CSS_RULE = /([^{}]+)\{([^{}]*)\}/g

/** Selectors of button rules that pour the accent colour across their surface. */
function buttonRulesWithAccentFill(componentSource: string): string[] {
  const styles = componentSource.split('<style>').slice(1).join('<style>')

  return Array.from(styles.matchAll(CSS_RULE))
    .map((rule) => [rule[1] ?? '', rule[2] ?? ''] as const)
    .filter(([selector, body]) => /btn|button/i.test(selector) && ACCENT_FILL.test(body))
    .map(([selector]) => selector.trim())
}

function componentsUnder(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = resolve(dir, entry.name)
    if (entry.isDirectory()) return componentsUnder(full)
    return entry.name.endsWith('.svelte') ? [full] : []
  })
}

describe('design system visual contract', () => {
  it('defines the desktop typography and control tokens', () => {
    const tokens = readSource('../../tokens/tokens.css')

    expect(tokens).toContain('--font-size-xs: 12px;')
    expect(tokens).toContain('--font-size-sm: 14px;')
    expect(tokens).toContain('--font-size-md: 16px;')
    expect(tokens).toContain('--font-size-lg: 18px;')
    expect(tokens).toContain('--font-size-xl: 22px;')
    expect(tokens).toContain('--control-height-sm: 30px;')
    expect(tokens).toContain('--control-height-md: 36px;')
    expect(tokens).toContain('--control-height-lg: 40px;')
    expect(tokens).toContain('--color-surface-glass: #10131a;')
    expect(tokens).toContain('--color-accent-faint: rgba(154, 164, 199, 0.06);')
    expect(tokens).toContain(":root[data-theme='dim']")
    expect(tokens).toContain('--color-surface-glass: #211e17;')
    expect(tokens).toContain('--focus-ring: 0 0 0 2px rgba(154, 164, 199, 0.22);')
  })

  it('anchors the item card delete overlay above the button base rules', () => {
    const itemCard = readSource('../ItemCard/ItemCard.svelte')

    // Button sets `position: relative` on itself. A bare :global() rule ties on
    // specificity and loses on source order, which drops the overlay into the
    // card's flow, so the placement has to be scoped through the card.
    expect(itemCard).toMatch(
      /\.item-card :global\(\.item-card__delete\)\s*\{[^}]*position: absolute;/,
    )
  })

  it('names only published tokens in var() calls that carry no fallback', () => {
    const published = definedTokens()
    const offenders = componentsUnder(resolve(import.meta.dirname, '../..'))
      .map((path) => [path, undefinedTokensIn(path, published)] as const)
      .filter(([, missing]) => missing.length > 0)
      .map(([path, missing]) => `${basename(path)}: ${missing.join(', ')}`)

    expect(offenders).toEqual([])
  })

  it('keeps card delete actions on the neutral button colour', () => {
    // Deliberate: these controls read as neutral, and the danger colour they
    // used to declare never reached the screen anyway — it tied with the ghost
    // variant on specificity and lost on source order.
    expect(readSource('../ItemCard/ItemCard.svelte')).not.toContain('--color-danger')
    expect(readSource('../CollectionCard/CollectionCard.svelte')).not.toContain('--color-danger')
  })

  it('lets callers add a class without losing the button base classes', () => {
    const button = readSource('../Button/Button.svelte')

    // A `class` left in the {...rest} spread overwrites the computed class
    // attribute outright, stripping every .btn rule from the control.
    expect(button).toContain("class: className = ''")
    expect(button).toContain('class="btn btn--{variant} btn--{size} {className}"')
  })

  it('aligns button, input and search controls on shared tokens', () => {
    const button = readSource('../Button/Button.svelte')
    const input = readSource('../Input/Input.svelte')
    const searchBar = readSource('../SearchBar/SearchBar.svelte')

    expect(button).toContain('min-height: var(--control-height-md);')
    expect(button).toContain('box-shadow: var(--focus-ring);')

    expect(input).toContain('min-height: var(--control-height-md);')
    expect(input).toContain('box-shadow: var(--focus-ring);')

    expect(searchBar).toContain('min-height: var(--control-height-md);')
    expect(searchBar).toContain('box-shadow: var(--focus-ring);')
  })

  it('gives cards elevated sections and subtle dividers', () => {
    const card = readSource('../Card/Card.svelte')

    expect(card).toContain('var(--color-surface-elevated)')
    expect(card).toContain('border-bottom: 1px solid var(--color-hairline);')
    expect(card).toContain('border-top: 1px solid var(--color-hairline);')
  })

  it('keeps visual primitives on semantic surface and state tokens', () => {
    const panel = readSource('../Panel/Panel.svelte')
    const tabList = readSource('../Tabs/TabList.svelte')
    const tabButton = readSource('../Tabs/TabButton.svelte')
    const iconButton = readSource('../IconButton/IconButton.svelte')
    const statusBadge = readSource('../StatusBadge/StatusBadge.svelte')

    expect(panel).toContain('var(--surface-panel)')
    expect(panel).toContain('var(--surface-card)')
    expect(panel).toContain('var(--surface-glass)')
    expect(panel).toContain('var(--shadow-surface)')
    expect(panel).toContain('var(--focus-ring)')

    expect(tabList).toContain('role="tablist"')
    expect(tabList).toContain('var(--surface-input)')
    expect(tabButton).toContain('role="tab"')
    expect(tabButton).toContain('aria-selected={active}')

    // One container per icon step: xs 24/12, sm 28/14, md 32/16, lg 40/20.
    expect(iconButton).toContain('width: 24px;')
    expect(iconButton).toContain('width: 28px;')
    expect(iconButton).toContain('width: 32px;')
    expect(iconButton).toContain('width: var(--control-height-lg);')

    expect(statusBadge).toContain('var(--state-ai-soft)')
    expect(statusBadge).toContain('var(--state-evidence-soft)')
  })

  it('publishes the primary control ladder in every theme', () => {
    const RUNGS = [
      '--control-primary-bg:',
      '--control-primary-bg-hover:',
      '--control-primary-bg-active:',
      '--control-primary-border:',
      '--control-primary-text:',
    ]

    // Prominence for a primary control comes from this ladder, so each theme has
    // to carry its own rungs. A theme that inherited the dark values would land
    // a near-black fill on a white page, or a cold grey on the warm dim palette.
    const missing = [':root {', ":root[data-theme='dim']", ":root[data-theme='light']"].flatMap(
      (theme) => {
        const scope = themeBlock(readSource('../../tokens/tokens.css'), theme)
        return RUNGS.filter((rung) => !scope.includes(rung)).map((rung) => `${theme} ${rung}`)
      },
    )

    expect(missing).toEqual([])
  })

  it('keeps primary controls off the accent fill', () => {
    // The accent is a light lavender-grey. Poured across a whole control it
    // reads as a different design system from the dark surfaces around it, so
    // primary earns its prominence from the control ladder's luminosity and
    // border instead. The accent stays for focus rings, links, progress fills
    // and other thin accents, so only button rules are in scope here.
    const offenders = [
      'Button/Button.svelte',
      'IconButton/IconButton.svelte',
      'NoteEditor/NoteEditor.svelte',
      'AudioPlayer/AudioPlayer.svelte',
    ].flatMap((relativePath) =>
      buttonRulesWithAccentFill(readSource(`../${relativePath}`)).map(
        (selector) => `${basename(relativePath)}: ${selector}`,
      ),
    )

    expect(offenders).toEqual([])
  })

  it('sizes annotation toolbar icons from the canonical scale', () => {
    // The toolbar overrides the rendered svg size in CSS
    // (.annotation-toolbar__button :global(svg)), so the `size` prop passed to
    // ActionIcon has no effect here — this token is the real icon size and it
    // has to sit on ACTION_ICON_SIZES like every other icon in the app.
    const annotationToolbar = readSource('../AnnotationToolbar/AnnotationToolbar.svelte')

    expect(annotationToolbar).toContain(
      '--annotation-toolbar-icon-size: calc(16px * var(--annotation-toolbar-scale));'
    )
  })
})
