import { render } from '@testing-library/svelte'
import { describe, expect, it } from 'vitest'
import ActionIcon from '../ActionIcon.svelte'
import source from '../ActionIcon.svelte?raw'

/** rotate-fine-cw and rotate-fine-ccw are drawn in ./icons, not imported. */
const IN_HOUSE_ICONS = 2
import { ACTION_ICON_NAMES, ACTION_ICON_SIZES } from '../ActionIcon.types'

describe('ActionIcon', () => {
  it('imports every glyph from its own module, never from the package barrel', () => {
    // ActionIcon sits in eslint.config.js's allowlist, where no-restricted-imports
    // is switched off so it can import icons at all. That leaves nothing stopping
    // this file from going back to `import { IconX } from '@tabler/...'` — which
    // costs nothing in a production build and hangs Tauri's WebView in dev, where
    // the barrel is 7.9 MB pulling 6184 .svelte modules. This is that stop.
    //
    // Comments are stripped first: the file explains the rule in prose that
    // names the barrel, and a check that reads its own documentation as a
    // violation is a check that cannot be written down.
    const code = source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/\/\/[^\n]*/g, '')

    const barrel = [...code.matchAll(/from '@tabler\/icons-svelte-runes'/g)]
    const perIcon = [...code.matchAll(/from '@tabler\/icons-svelte-runes\/icons\/[a-z0-9-]+'/g)]

    // Every name is either a Tabler module or one of the two drawn in ./icons.
    expect([barrel.length, perIcon.length + IN_HOUSE_ICONS]).toEqual([0, ACTION_ICON_NAMES.length])
  })

  it('renders an svg for every name in the catalogue', () => {
    const missing: string[] = []

    for (const name of ACTION_ICON_NAMES) {
      const { container, unmount } = render(ActionIcon, { props: { name } })
      if (container.querySelector('svg') === null) missing.push(name)
      unmount()
    }

    expect(missing).toEqual([])
  })

  it('hides every icon from the accessibility tree', () => {
    const exposed: string[] = []

    for (const name of ACTION_ICON_NAMES) {
      const { container, unmount } = render(ActionIcon, { props: { name } })
      const svg = container.querySelector('svg')
      if (svg?.getAttribute('aria-hidden') !== 'true') exposed.push(name)
      unmount()
    }

    expect(exposed).toEqual([])
  })

  it('draws every icon at the canonical stroke width', () => {
    const offContract: string[] = []

    for (const name of ACTION_ICON_NAMES) {
      const { container, unmount } = render(ActionIcon, { props: { name } })
      const stroke = container.querySelector('svg')?.getAttribute('stroke-width')
      if (stroke !== null && stroke !== '2') offContract.push(`${name}=${stroke}`)
      unmount()
    }

    expect(offContract).toEqual([])
  })

  it('stamps its own name on every icon, independent of the icon library', () => {
    // The family behind the gate has changed once already (Lucide -> Tabler).
    // A test that reaches for the library's class name (`svg.lucide-rotate-ccw`)
    // breaks on the next swap; this attribute is ours, so it does not.
    const unstamped: string[] = []

    for (const name of ACTION_ICON_NAMES) {
      const { container, unmount } = render(ActionIcon, { props: { name } })
      const svg = container.querySelector('svg')
      if (svg?.getAttribute('data-action-icon') !== name) {
        unstamped.push(`${name}=${svg?.getAttribute('data-action-icon')}`)
      }
      unmount()
    }

    expect(unstamped).toEqual([])
  })

  it('draws a distinct glyph for every name in the catalogue', () => {
    // Two names sharing one glyph is how a semantic remap goes wrong quietly:
    // the build passes, and two different controls grow the same affordance.
    const byGeometry = new Map<string, string[]>()

    for (const name of ACTION_ICON_NAMES) {
      const { container, unmount } = render(ActionIcon, { props: { name } })
      const geometry = container.querySelector('svg')?.innerHTML ?? ''
      byGeometry.set(geometry, [...(byGeometry.get(geometry) ?? []), name])
      unmount()
    }

    const collisions = [...byGeometry.values()].filter((names) => names.length > 1)

    expect(collisions).toEqual([])
  })

  it('draws the footnote apart from the superscript it used to borrow', () => {
    // The footnote button drew Tabler's superscript glyph while there was no
    // superscript in the editor. Now there is one, and two buttons with the
    // same x² would be two different commands wearing one face.
    const geometry = (name: string) => {
      const { container, unmount } = render(ActionIcon, {
        props: { name: name as (typeof ACTION_ICON_NAMES)[number] },
      })
      const svg = container.querySelector('svg')?.innerHTML ?? ''
      unmount()
      return svg
    }
    const typography = [
      'text-increase',
      'text-decrease',
      'letter-case',
      'subscript',
      'superscript',
      'clear-formatting',
    ]

    expect(
      typography.filter((name) => !(ACTION_ICON_NAMES as readonly string[]).includes(name))
    ).toEqual([])
    expect(geometry('footnote')).not.toBe(geometry('superscript'))
  })

  it('renders the notification bell that NotificationBell draws by hand today', () => {
    const { container } = render(ActionIcon, { props: { name: 'bell' } })

    expect(container.querySelector('svg')).not.toBeNull()
  })

  it('applies the requested size to both axes', () => {
    const { container } = render(ActionIcon, { props: { name: 'close', size: 20 } })
    const svg = container.querySelector('svg')

    expect(svg).toHaveAttribute('width', '20')
    expect(svg).toHaveAttribute('height', '20')
  })

  it('defaults to the md step when no size is given', () => {
    const { container } = render(ActionIcon, { props: { name: 'close' } })

    expect(container.querySelector('svg')).toHaveAttribute('width', '16')
  })

  it('exposes a closed size scale of even steps only', () => {
    expect(ACTION_ICON_SIZES).toEqual([12, 14, 16, 20, 24, 40])
    expect(ACTION_ICON_SIZES.every((size) => size % 2 === 0)).toBe(true)
  })

  it('no longer exposes the pencil alias of edit', () => {
    expect(ACTION_ICON_NAMES).not.toContain('pencil')
    expect(ACTION_ICON_NAMES).toContain('edit')
  })
})
