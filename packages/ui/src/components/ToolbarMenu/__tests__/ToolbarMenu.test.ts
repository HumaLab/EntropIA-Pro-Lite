import { fireEvent, render, screen } from '@testing-library/svelte'
import { tick } from 'svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type { ToolbarMenuItem } from '../ToolbarMenu.types'
import ToolbarMenuTestHost from './ToolbarMenuTestHost.svelte'

function itemsWith(spies: Record<string, () => void> = {}): ToolbarMenuItem[] {
  const noop = () => {}
  return [
    {
      id: 'bold',
      kind: 'checkbox',
      label: 'Bold',
      icon: 'bold',
      checked: true,
      onselect: spies.bold ?? noop,
    },
    {
      id: 'italic',
      kind: 'checkbox',
      label: 'Italic',
      icon: 'italic',
      checked: false,
      onselect: spies.italic ?? noop,
    },
    { id: 'sep', kind: 'separator' },
    { id: 'table', label: 'Table', icon: 'table', disabled: true, onselect: spies.table ?? noop },
    { id: 'footnote', label: 'Footnote', icon: 'footnote', onselect: spies.footnote ?? noop },
  ]
}

const trigger = () => screen.getByRole('button', { name: 'Open menu' })
const menu = () => screen.queryByRole('menu', { name: 'More tools' })
const focused = () => document.activeElement as HTMLElement | null

async function openMenu() {
  await fireEvent.click(trigger())
  await tick()
}

async function press(key: string, target: Element = focused() ?? document.body) {
  await fireEvent.keyDown(target, { key })
  await tick()
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('ToolbarMenu: the trigger', () => {
  it('announces a menu it controls, collapsed until opened', async () => {
    render(ToolbarMenuTestHost, { props: { items: itemsWith() } })

    expect(trigger()).toHaveAttribute('aria-haspopup', 'menu')
    expect(trigger()).toHaveAttribute('aria-expanded', 'false')
    expect(menu()).toBeNull()

    await openMenu()

    expect(trigger()).toHaveAttribute('aria-expanded', 'true')
    expect(trigger().getAttribute('aria-controls')).toBe(menu()?.id)
  })

  it('moves focus into the menu, onto the first item', async () => {
    render(ToolbarMenuTestHost, { props: { items: itemsWith() } })
    await openMenu()

    expect(focused()).toHaveTextContent('Bold')
  })

  it('opens on ArrowDown at the first item and on ArrowUp at the last', async () => {
    render(ToolbarMenuTestHost, { props: { items: itemsWith() } })

    await press('ArrowDown', trigger())
    expect(focused()).toHaveTextContent('Bold')

    await press('Escape')
    await press('ArrowUp', trigger())
    expect(focused()).toHaveTextContent('Footnote')
  })

  it('closes when clicked again', async () => {
    render(ToolbarMenuTestHost, { props: { items: itemsWith() } })
    await openMenu()
    await openMenu()

    expect(menu()).toBeNull()
  })
})

describe('ToolbarMenu: items', () => {
  it('gives each item the role its kind calls for, with its state', async () => {
    render(ToolbarMenuTestHost, { props: { items: itemsWith() } })
    await openMenu()

    expect(screen.getByRole('menuitemcheckbox', { name: 'Bold' })).toHaveAttribute(
      'aria-checked',
      'true'
    )
    expect(screen.getByRole('menuitemcheckbox', { name: 'Italic' })).toHaveAttribute(
      'aria-checked',
      'false'
    )
    expect(screen.getByRole('menuitem', { name: 'Table' })).toBeDisabled()
    expect(screen.getByRole('menuitem', { name: 'Footnote' })).toBeEnabled()
    expect(screen.getByRole('separator')).toBeInTheDocument()
  })

  it('draws the icon each item names', async () => {
    render(ToolbarMenuTestHost, { props: { items: itemsWith() } })
    await openMenu()

    const bold = screen.getByRole('menuitemcheckbox', { name: 'Bold' })
    expect(bold.querySelector('[data-action-icon="bold"]')).not.toBeNull()
  })

  it('runs the item and closes when it is clicked', async () => {
    const footnote = vi.fn()
    render(ToolbarMenuTestHost, { props: { items: itemsWith({ footnote }) } })
    await openMenu()

    await fireEvent.click(screen.getByRole('menuitem', { name: 'Footnote' }))
    await tick()

    expect(footnote).toHaveBeenCalledOnce()
    expect(menu()).toBeNull()
  })
})

describe('ToolbarMenu: keyboard', () => {
  it('moves through the items with the arrows, skipping the disabled, and wraps', async () => {
    render(ToolbarMenuTestHost, { props: { items: itemsWith() } })
    await openMenu()

    await press('ArrowDown')
    expect(focused()).toHaveTextContent('Italic')
    await press('ArrowDown')
    expect(focused()).toHaveTextContent('Footnote')
    await press('ArrowDown')
    expect(focused()).toHaveTextContent('Bold')
    await press('ArrowUp')
    expect(focused()).toHaveTextContent('Footnote')
  })

  it('jumps to the ends with Home and End', async () => {
    render(ToolbarMenuTestHost, { props: { items: itemsWith() } })
    await openMenu()

    await press('End')
    expect(focused()).toHaveTextContent('Footnote')
    await press('Home')
    expect(focused()).toHaveTextContent('Bold')
  })

  it('activates the focused item with Enter, and with Space', async () => {
    const bold = vi.fn()
    const italic = vi.fn()
    render(ToolbarMenuTestHost, { props: { items: itemsWith({ bold, italic }) } })

    await openMenu()
    await press('Enter')
    expect(bold).toHaveBeenCalledOnce()
    expect(menu()).toBeNull()

    await openMenu()
    await press('ArrowDown')
    await press(' ')
    expect(italic).toHaveBeenCalledOnce()
  })

  it('closes on Escape and gives the focus back to the trigger', async () => {
    const onclose = vi.fn()
    render(ToolbarMenuTestHost, { props: { items: itemsWith(), onclose } })
    await openMenu()

    await press('Escape')

    expect(menu()).toBeNull()
    expect(focused()).toBe(trigger())
    expect(onclose).toHaveBeenCalledWith('escape')
  })
})

describe('ToolbarMenu: closing from outside', () => {
  it('closes on a press outside, and not on one inside', async () => {
    render(ToolbarMenuTestHost, { props: { items: itemsWith() } })
    await openMenu()

    await fireEvent.pointerDown(menu()!)
    await tick()
    expect(menu()).not.toBeNull()

    await fireEvent.pointerDown(screen.getByRole('button', { name: 'After' }))
    await tick()
    expect(menu()).toBeNull()
  })

  it('closes when the focus leaves it', async () => {
    render(ToolbarMenuTestHost, { props: { items: itemsWith() } })
    await openMenu()

    screen.getByRole('button', { name: 'After' }).focus()
    await tick()

    expect(menu()).toBeNull()
  })
})

describe('ToolbarMenu: free content', () => {
  it('navigates and activates content that brings its own menu roles', async () => {
    const onswatch = vi.fn()
    render(ToolbarMenuTestHost, { props: { swatches: ['Red', 'Green', 'Blue'], onswatch } })
    await openMenu()

    expect(focused()).toHaveTextContent('Red')
    await press('ArrowDown')
    await press('Enter')

    expect(onswatch).toHaveBeenCalledWith('Green')
    expect(menu()).toBeNull()
  })
})

describe('ToolbarMenu: placement', () => {
  it('floats in the viewport and stays inside it at the right edge', async () => {
    vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (
      this: HTMLElement
    ) {
      const trigger =
        'menuTrigger' in this.dataset || this.getAttribute('aria-label') === 'Open menu'
      const box = trigger
        ? { left: window.innerWidth - 20, top: 40, width: 28, height: 28 }
        : { left: 0, top: 0, width: 180, height: 120 }
      return {
        ...box,
        right: box.left + box.width,
        bottom: box.top + box.height,
        x: 0,
        y: 0,
        toJSON() {},
      } as DOMRect
    })
    render(ToolbarMenuTestHost, { props: { items: itemsWith() } })
    await openMenu()

    const style = menu()!.style
    const left = Number.parseFloat(style.left)
    expect(style.position).toBe('fixed')
    expect(left + 180).toBeLessThanOrEqual(window.innerWidth)
    expect(left).toBeGreaterThanOrEqual(0)
    expect(style.top).toBe('72px')
  })
})
