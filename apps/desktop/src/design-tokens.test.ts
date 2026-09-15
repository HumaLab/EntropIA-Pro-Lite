import { readdirSync, readFileSync } from 'node:fs'
import { basename, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

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
    for (const token of capturedNames(readFileSync(sheet, 'utf-8').matchAll(TOKEN_DEFINITION))) {
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

/**
 * Every component the user can end up looking at, wherever it is authored. The
 * button rules below describe the app's surface, not one package's, and a rule
 * that only watched half of it would keep passing while the other half drifted.
 */
function everyComponent(): string[] {
  return [
    ...componentsUnder(import.meta.dirname),
    ...componentsUnder(resolve(import.meta.dirname, '../../../packages/ui/src')),
  ]
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
 *
 * Known limits, both of them narrowing what a green run proves:
 *  - The button rules find their subjects by selector NAME, so a control class
 *    called something else — `.explorer__chevron`, say — is invisible to them.
 *  - A height reaches this check only as a literal or as one of the three
 *    control-height tokens below. Any other `calc()`, `clamp()` or token
 *    arithmetic passes unread.
 * Passing is evidence about the rules these can see, not a survey of every
 * clickable surface in the app.
 */
const CONTROL_SCALE_PX = new Set([24, 28, 30, 32, 36, 40, 44])
const BUTTON_HEIGHT = /(?:min-)?height:\s*(\d+px|var\(--control-height-(?:sm|md|lg)\))/g

/** The control-height tokens, in the pixels they actually resolve to. */
const CONTROL_HEIGHT_TOKENS: Record<string, number> = {
  'var(--control-height-sm)': 30,
  'var(--control-height-md)': 36,
  'var(--control-height-lg)': 40,
}

function heightInPx(value: string): number {
  return CONTROL_HEIGHT_TOKENS[value] ?? Number.parseInt(value, 10)
}

/** A BEM child of a button — `.btn__spinner` — is furniture inside the control,
 *  not the control, and it is sized to whatever it draws. */
const BUTTON_CHILD = /\.(btn|button)__/i

/**
 * Controls that earn their size from their job rather than the ladder. Each
 * entry states why, because an exception with no reason is just a hole.
 */
const OFF_SCALE_BY_DESIGN: Record<string, string> = {
  '.audio-player__btn--play':
    "transport control: the play button is the player's one hero target and is sized for the thumb, not for a toolbar row",
  '.entity-viewer__action':
    'sized by its container, not the ladder: it sits inside a 24px chip whose content box is 20px, so the next rung up would push every chip to 28px',
}

function offScaleButtonHeightsIn(componentPath: string): string[] {
  const styles = readFileSync(componentPath, 'utf-8').split('<style>').slice(1).join('<style>')

  return Array.from(styles.matchAll(CSS_RULE))
    .map((rule) => [rule[1] ?? '', rule[2] ?? ''] as const)
    .filter(
      ([selector]) =>
        /btn|button|action(?!s\b)/i.test(selector) &&
        !BUTTON_CHILD.test(selector) &&
        !(selector.trim() in OFF_SCALE_BY_DESIGN)
    )
    .flatMap(([selector, body]) =>
      Array.from(body.matchAll(BUTTON_HEIGHT))
        .filter((match) => !CONTROL_SCALE_PX.has(heightInPx(match[1] ?? '')))
        .map((match) => `${selector.trim().replace(/\s+/g, ' ')} -> ${match[0]}`)
    )
}

/**
 * Keyboard focus is the only thing telling someone who does not use a mouse
 * where they are, so it has to look the same everywhere. A view is free to
 * skip the ring; what it may not do is draw its own.
 *
 * The ring may sit anywhere in the shadow list — a dialog composes it with its
 * own elevation — so what is checked is that it is in there at all.
 */
const FOCUS_SHADOW = /box-shadow:\s*([^;}]+)/

function bespokeFocusRingsIn(componentPath: string): string[] {
  const styles = readFileSync(componentPath, 'utf-8').split('<style>').slice(1).join('<style>')

  return Array.from(styles.matchAll(CSS_RULE))
    .map((rule) => [rule[1] ?? '', rule[2] ?? ''] as const)
    .filter(([selector, body]) => {
      if (!selector.includes(':focus-visible')) return false
      const shadow = body.match(FOCUS_SHADOW)?.[1]?.trim()
      // `none` is a rule clearing an inherited shadow, not drawing a ring.
      if (!shadow || shadow === 'none') return false
      return !shadow.includes('var(--focus-ring')
    })
    .map(([selector]) => selector.trim().replace(/\s+/g, ' '))
}

describe('desktop design tokens', () => {
  it('draws keyboard focus with the shared ring', () => {
    const offenders = everyComponent().flatMap((path) =>
      bespokeFocusRingsIn(path).map((rule) => `${basename(path)}: ${rule}`)
    )

    expect(offenders).toEqual([])
  })

  it('sizes action buttons from the control scale', () => {
    const offenders = everyComponent().flatMap((path) =>
      offScaleButtonHeightsIn(path).map((rule) => `${basename(path)}: ${rule}`)
    )

    expect(offenders).toEqual([])
  })

  it('keeps action buttons off decorative semantic tints', () => {
    const offenders = everyComponent().flatMap((path) =>
      tintedButtonRulesIn(path).map((rule) => `${basename(path)}: ${rule}`)
    )

    expect(offenders).toEqual([])
  })

  it('leaves the native checkbox to exactly one component', () => {
    // A hand-rolled `input type="checkbox"` renders with the platform accent,
    // blue on every desktop this app ships to, which in a monochrome dark theme
    // reads as a control borrowed from somewhere else. Three screens had grown
    // their own before the shared component existed. Asserting the exact set,
    // rather than excluding a name, also catches the component disappearing.
    const owners = everyComponent()
      .map((path) => [path, readFileSync(path, 'utf-8')] as const)
      .filter(([, source]) => source.includes('type="checkbox"'))
      .map(([path]) => basename(path))
      .sort()

    expect(owners).toEqual(['Checkbox.svelte'])
  })

  it('gives every search field a magnifier on its leading edge', () => {
    // Adjacency, not mere presence: a view can hold an unrelated search icon
    // elsewhere (the button that reveals a filter, say) and that must not
    // stand in for the one the field itself owes.
    // Every field in the app sits within 200 characters of its own icon;
    // anything further away belongs to some other control.
    const LEADING_WINDOW = 240
    const offenders = everyComponent()
      .map((path) => [path, readFileSync(path, 'utf-8')] as const)
      .flatMap(([path, source]) =>
        Array.from(source.matchAll(/type="search"/g))
          .filter((match) => {
            const at = match.index ?? 0
            const before = source.slice(Math.max(0, at - LEADING_WINDOW), at)
            return !before.includes('<ActionIcon name="search"')
          })
          .map(() => basename(path))
      )

    expect(offenders).toEqual([])
  })

  it('pairs the positioned magnifier with the inset it needs', () => {
    // SearchBar lays its icon out in the flow because it owns the whole row.
    // A field retrofitted into existing markup positions the icon over the
    // input instead, and then owes it padding — or the icon lands on the text.
    const offenders = everyComponent()
      .map((path) => [path, readFileSync(path, 'utf-8')] as const)
      .filter(([, source]) => source.includes('search-field__icon'))
      .filter(([, source]) => !source.includes('var(--search-field-inset)'))
      .map(([path]) => basename(path))

    expect(offenders).toEqual([])
  })

  it('settles every placeholder on one grey', () => {
    // Without a baseline the browser derives its own from `color`, which reads
    // lighter than the token and made two search boxes on one screen disagree.
    const appCss = readFileSync(resolve(import.meta.dirname, 'app.css'), 'utf-8')
    expect(appCss).toMatch(/::placeholder\s*\{[^}]*color:\s*var\(--color-text-muted\)/)

    const offenders = everyComponent()
      .map((path) => [path, readFileSync(path, 'utf-8')] as const)
      .flatMap(([path, source]) =>
        Array.from(source.matchAll(/::placeholder\s*\{([^}]*)\}/g))
          .map((match) => match[1] ?? '')
          .filter((body) => /color:/.test(body) && !body.includes('var(--color-text-muted)'))
          .map(() => basename(path))
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
