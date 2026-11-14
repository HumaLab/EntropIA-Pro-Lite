import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { invoke } from '@tauri-apps/api/core'
import { locale } from '$lib/i18n'
import { HELP_BUBBLE_MAXIMIZED_KEY, HELP_BUBBLE_STORAGE_KEY } from '$lib/help-bubble'
import HelpBubble from './HelpBubble.svelte'

const mockInvoke = vi.mocked(invoke)

const { navigationRef } = vi.hoisted(() => ({
  navigationRef: { navigate: vi.fn(), openRootSection: vi.fn() },
}))

vi.mock('$lib/pane-context', () => ({
  getNavigation: () => navigationRef,
  getPaneId: () => 'pane-test',
}))

/** Bottom-right default for the current viewport (56px bubble, status bar clearance). */
function defaultPosition(): { x: number; y: number } {
  return {
    x: window.innerWidth - 56 - 24,
    y: window.innerHeight - 56 - 46,
  }
}

function bubbleButton(): HTMLElement {
  return screen.getByRole('button', { name: 'Ayuda' })
}

const PREVIEW_TEXT = 'Producto: EntropIA Lite\n- llaves: configurada / no configurada'

function backend(): void {
  mockInvoke.mockImplementation(async (command: string) => {
    if (command === 'help_status') return { hasOpenrouterKey: true }
    if (command === 'help_diagnostics_preview') return PREVIEW_TEXT
    if (command === 'help_faq_list') return []
    return undefined
  })
}

async function openPanel(): Promise<void> {
  await fireEvent.click(bubbleButton())
  await screen.findByRole('textbox', { name: 'Escribí tu pregunta…' })
}

function setWindowSize(width: number, height: number): void {
  Object.defineProperty(window, 'innerWidth', { value: width, configurable: true })
  Object.defineProperty(window, 'innerHeight', { value: height, configurable: true })
}

describe('HelpBubble (S13, D9)', () => {
  const realWidth = window.innerWidth
  const realHeight = window.innerHeight

  beforeEach(() => {
    locale.set('es')
    localStorage.clear()
    navigationRef.navigate.mockReset()
    navigationRef.openRootSection.mockReset()
    mockInvoke.mockReset()
    backend()
    setWindowSize(realWidth, realHeight)
  })

  it('renders one round button at the bottom-right with its accessible name and the logo', () => {
    // The bubble is portalled to the overlay root (fixed-overlays guard),
    // so it lives in the document, not in the render container.
    render(HelpBubble)

    const button = bubbleButton()
    expect(button.tagName).toBe('BUTTON')
    const expected = defaultPosition()
    expect(button.style.left).toBe(`${expected.x}px`)
    expect(button.style.top).toBe(`${expected.y}px`)

    // The EntropIA mark, decorative: the button's name carries the meaning.
    const logo = button.querySelector('img')
    expect(logo).not.toBeNull()
    expect(logo).toHaveAttribute('aria-hidden', 'true')
    expect(logo?.getAttribute('src')).toContain('entropia-mark')
    expect(document.querySelector('.help-bubble')).not.toBeNull()
  })

  it('is reachable by Tab: a native button, focusable without a tabindex', () => {
    render(HelpBubble)

    const button = bubbleButton()
    expect(button.hasAttribute('tabindex')).toBe(false)
    button.focus()
    expect(document.activeElement).toBe(button)
  })

  it('click toggles the help panel, which hosts the help chat', async () => {
    render(HelpBubble)

    expect(screen.queryByRole('textbox', { name: 'Escribí tu pregunta…' })).not.toBeInTheDocument()

    await openPanel()
    expect(screen.getByRole('dialog', { name: 'Ayuda' })).toBeInTheDocument()

    await fireEvent.click(bubbleButton())
    expect(screen.queryByRole('textbox', { name: 'Escribí tu pregunta…' })).not.toBeInTheDocument()
  })

  it('the close button shuts the panel', async () => {
    render(HelpBubble)
    await openPanel()

    await fireEvent.click(screen.getByRole('button', { name: 'Cerrar ayuda' }))

    expect(screen.queryByRole('textbox', { name: 'Escribí tu pregunta…' })).not.toBeInTheDocument()
  })

  it('Escape closes the panel and returns focus to the bubble', async () => {
    render(HelpBubble)
    await openPanel()

    const input = screen.getByRole('textbox', { name: 'Escribí tu pregunta…' })
    input.focus()
    expect(document.activeElement).toBe(input)

    await fireEvent.keyDown(document.body, { key: 'Escape' })

    expect(screen.queryByRole('textbox', { name: 'Escribí tu pregunta…' })).not.toBeInTheDocument()
    expect(document.activeElement).toBe(bubbleButton())
  })

  it('Escape with the panel shut does nothing to focus', async () => {
    render(HelpBubble)

    const other = document.createElement('button')
    other.textContent = 'other'
    document.body.appendChild(other)
    other.focus()
    try {
      await fireEvent.keyDown(document.body, { key: 'Escape' })
      expect(document.activeElement).toBe(other)
    } finally {
      other.remove()
    }
  })

  it('a drag beyond the threshold moves the bubble, saves offsets, and does not open the panel', async () => {
    render(HelpBubble)

    const button = bubbleButton()
    const from = defaultPosition()
    const startX = from.x + 28
    const startY = from.y + 28

    await fireEvent.pointerDown(button, { clientX: startX, clientY: startY, pointerId: 1 })
    await fireEvent.pointerMove(button, {
      clientX: startX - 100,
      clientY: startY - 50,
      pointerId: 1,
    })
    await fireEvent.pointerUp(button, { clientX: startX - 100, clientY: startY - 50, pointerId: 1 })
    // Browsers fire a click after the drag: it must not toggle the panel.
    await fireEvent.click(button)

    expect(button.style.left).toBe(`${from.x - 100}px`)
    expect(button.style.top).toBe(`${from.y - 50}px`)
    // Stored as distances to the right/bottom edges, not absolute left/top.
    expect(localStorage.getItem(HELP_BUBBLE_STORAGE_KEY)).toBe('{"right":124,"bottom":96}')
    expect(screen.queryByRole('textbox', { name: 'Escribí tu pregunta…' })).not.toBeInTheDocument()
  })

  it('a press under the threshold is a click: it opens the panel and saves nothing', async () => {
    render(HelpBubble)

    const button = bubbleButton()
    const from = defaultPosition()
    const startX = from.x + 28
    const startY = from.y + 28

    await fireEvent.pointerDown(button, { clientX: startX, clientY: startY, pointerId: 1 })
    await fireEvent.pointerMove(button, { clientX: startX + 2, clientY: startY, pointerId: 1 })
    await fireEvent.pointerUp(button, { clientX: startX + 2, clientY: startY, pointerId: 1 })
    await fireEvent.click(button)

    await screen.findByRole('textbox', { name: 'Escribí tu pregunta…' })
    expect(localStorage.getItem(HELP_BUBBLE_STORAGE_KEY)).toBeNull()
  })

  it('restores the stored offsets on mount', () => {
    localStorage.setItem(HELP_BUBBLE_STORAGE_KEY, '{"right":124,"bottom":96}')
    render(HelpBubble)

    const button = bubbleButton()
    const expected = defaultPosition()
    expect(button.style.left).toBe(`${expected.x - 100}px`)
    expect(button.style.top).toBe(`${expected.y - 50}px`)
  })

  it('migrates a legacy absolute position using the current window size', () => {
    localStorage.setItem(HELP_BUBBLE_STORAGE_KEY, '{"x":120,"y":200}')
    render(HelpBubble)

    // The bubble stays where the user left it: conversion, not a reset.
    const button = bubbleButton()
    expect(button.style.left).toBe('120px')
    expect(button.style.top).toBe('200px')
    // …and the legacy shape is rewritten once, in the new shape.
    expect(localStorage.getItem(HELP_BUBBLE_STORAGE_KEY)).toBe(
      `{"right":${window.innerWidth - 56 - 120},"bottom":${window.innerHeight - 56 - 200}}`
    )
  })

  it('clamps stored offsets that no longer fit the window', () => {
    localStorage.setItem(HELP_BUBBLE_STORAGE_KEY, '{"right":-1000,"bottom":-1000}')
    render(HelpBubble)

    const button = bubbleButton()
    expect(Number.parseFloat(button.style.left)).toBeLessThanOrEqual(window.innerWidth - 56)
    expect(Number.parseFloat(button.style.top)).toBeLessThanOrEqual(window.innerHeight - 56)
  })

  it('falls back to the default when the stored position is corrupt', () => {
    localStorage.setItem(HELP_BUBBLE_STORAGE_KEY, 'not json at all')
    render(HelpBubble)

    const expected = defaultPosition()
    const button = bubbleButton()
    expect(button.style.left).toBe(`${expected.x}px`)
    expect(button.style.top).toBe(`${expected.y}px`)
  })

  it('keeps the dragged offsets across remounts', async () => {
    const { unmount } = render(HelpBubble)

    const button = bubbleButton()
    const from = defaultPosition()
    await fireEvent.pointerDown(button, {
      clientX: from.x + 28,
      clientY: from.y + 28,
      pointerId: 1,
    })
    await fireEvent.pointerMove(button, {
      clientX: from.x - 22,
      clientY: from.y + 28,
      pointerId: 1,
    })
    await fireEvent.pointerUp(button, { clientX: from.x - 22, clientY: from.y + 28, pointerId: 1 })
    unmount()

    render(HelpBubble)
    const again = bubbleButton()
    expect(again.style.left).toBe(`${from.x - 50}px`)
  })

  it('the default stays in the bottom-right corner after a resize (1024x700 -> 1920x1080)', async () => {
    setWindowSize(1024, 700)
    try {
      render(HelpBubble)
      const button = bubbleButton()
      expect(button.style.left).toBe(`${1024 - 56 - 24}px`)
      expect(button.style.top).toBe(`${700 - 56 - 46}px`)

      // Maximizing the window moves the corner: the bubble follows it.
      setWindowSize(1920, 1080)
      window.dispatchEvent(new Event('resize'))
      await waitFor(() => {
        expect(button.style.left).toBe(`${1920 - 56 - 24}px`)
        expect(button.style.top).toBe(`${1080 - 56 - 46}px`)
      })
    } finally {
      setWindowSize(realWidth, realHeight)
    }
  })

  it('a dragged bubble keeps its distance to the corner after a resize', async () => {
    setWindowSize(1024, 768)
    try {
      render(HelpBubble)
      const button = bubbleButton()
      const from = { x: 1024 - 56 - 24, y: 768 - 56 - 46 }

      await fireEvent.pointerDown(button, {
        clientX: from.x + 28,
        clientY: from.y + 28,
        pointerId: 1,
      })
      await fireEvent.pointerMove(button, {
        clientX: from.x + 28 - 100,
        clientY: from.y + 28 - 50,
        pointerId: 1,
      })
      await fireEvent.pointerUp(button, {
        clientX: from.x + 28 - 100,
        clientY: from.y + 28 - 50,
        pointerId: 1,
      })
      expect(localStorage.getItem(HELP_BUBBLE_STORAGE_KEY)).toBe('{"right":124,"bottom":96}')

      setWindowSize(1920, 1080)
      window.dispatchEvent(new Event('resize'))
      await waitFor(() => {
        // Same offsets from the new corner.
        expect(button.style.left).toBe(`${1920 - 56 - 124}px`)
        expect(button.style.top).toBe(`${1080 - 56 - 96}px`)
      })
    } finally {
      setWindowSize(realWidth, realHeight)
    }
  })

  it('the panel opens toward the inside of the window, never off-screen', async () => {
    render(HelpBubble)
    await openPanel()

    // Default bottom-right: the panel grows leftwards and upwards.
    const panel = screen.getByRole('dialog', { name: 'Ayuda' })
    expect(panel).toHaveAttribute('data-side', 'left')
    expect(panel).toHaveAttribute('data-vertical', 'above')
  })

  it('the panel flips sides when the bubble sits at the top-left', async () => {
    localStorage.setItem(HELP_BUBBLE_STORAGE_KEY, '{"right":9000,"bottom":9000}')
    render(HelpBubble)
    await openPanel()

    const panel = screen.getByRole('dialog', { name: 'Ayuda' })
    expect(panel).toHaveAttribute('data-side', 'right')
    expect(panel).toHaveAttribute('data-vertical', 'below')
  })

  it('re-clamps the bubble when the window shrinks', async () => {
    localStorage.setItem(HELP_BUBBLE_STORAGE_KEY, '{"right":900,"bottom":600}')
    render(HelpBubble)
    const button = bubbleButton()

    setWindowSize(500, 400)
    try {
      window.dispatchEvent(new Event('resize'))
      await waitFor(() => {
        // Pushed back inside with 8px of air from the shrunken edges.
        expect(button.style.left).toBe('8px')
        expect(button.style.top).toBe('8px')
      })
    } finally {
      setWindowSize(realWidth, realHeight)
    }
  })

  it('the header offers a maximize control with its accessible name', async () => {
    render(HelpBubble)
    await openPanel()

    const maximize = screen.getByRole('button', { name: 'Maximizar' })
    expect(screen.queryByRole('button', { name: 'Restaurar' })).not.toBeInTheDocument()

    await fireEvent.click(maximize)

    expect(screen.queryByRole('button', { name: 'Maximizar' })).not.toBeInTheDocument()
    const restore = screen.getByRole('button', { name: 'Restaurar' })
    expect(restore).toBeInTheDocument()
    expect(screen.getByRole('dialog', { name: 'Ayuda' })).toHaveAttribute('data-maximized', 'true')

    await fireEvent.click(restore)

    expect(screen.getByRole('button', { name: 'Maximizar' })).toBeInTheDocument()
    expect(screen.getByRole('dialog', { name: 'Ayuda' })).toHaveAttribute('data-maximized', 'false')
  })

  it('a maximized panel fills the window with a small margin', async () => {
    render(HelpBubble)
    await openPanel()

    await fireEvent.click(screen.getByRole('button', { name: 'Maximizar' }))

    const panel = screen.getByRole('dialog', { name: 'Ayuda' })
    expect(panel.style.left).toBe('16px')
    expect(panel.style.top).toBe('16px')
    expect(panel.style.right).toBe('16px')
    // Above the status bar, like the bubble's own default clearance.
    expect(panel.style.bottom).toBe('46px')
  })

  it('the maximized state persists across close and reopen', async () => {
    render(HelpBubble)
    await openPanel()

    await fireEvent.click(screen.getByRole('button', { name: 'Maximizar' }))
    expect(localStorage.getItem(HELP_BUBBLE_MAXIMIZED_KEY)).toBe('true')
    await fireEvent.click(screen.getByRole('button', { name: 'Cerrar ayuda' }))
    expect(screen.queryByRole('textbox', { name: 'Escribí tu pregunta…' })).not.toBeInTheDocument()

    await openPanel()
    expect(screen.getByRole('button', { name: 'Restaurar' })).toBeInTheDocument()
    expect(screen.getByRole('dialog', { name: 'Ayuda' })).toHaveAttribute('data-maximized', 'true')

    await fireEvent.click(screen.getByRole('button', { name: 'Restaurar' }))
    expect(localStorage.getItem(HELP_BUBBLE_MAXIMIZED_KEY)).toBe('false')
  })

  it('the maximized state persists across a remount', async () => {
    const first = render(HelpBubble)
    await openPanel()
    await fireEvent.click(screen.getByRole('button', { name: 'Maximizar' }))
    first.unmount()

    render(HelpBubble)
    await openPanel()
    expect(screen.getByRole('button', { name: 'Restaurar' })).toBeInTheDocument()
  })

  it('toggling maximize keeps the conversation: a typed draft survives', async () => {
    render(HelpBubble)
    await openPanel()

    const input = screen.getByRole('textbox', { name: 'Escribí tu pregunta…' })
    await fireEvent.input(input, { target: { value: 'borrador sin enviar' } })
    expect(input).toHaveValue('borrador sin enviar')

    await fireEvent.click(screen.getByRole('button', { name: 'Maximizar' }))
    expect(screen.getByRole('textbox', { name: 'Escribí tu pregunta…' })).toHaveValue(
      'borrador sin enviar'
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Restaurar' }))
    expect(screen.getByRole('textbox', { name: 'Escribí tu pregunta…' })).toHaveValue(
      'borrador sin enviar'
    )
  })

  it('Escape still closes a maximized panel', async () => {
    render(HelpBubble)
    await openPanel()
    await fireEvent.click(screen.getByRole('button', { name: 'Maximizar' }))

    await fireEvent.keyDown(document.body, { key: 'Escape' })

    expect(screen.queryByRole('textbox', { name: 'Escribí tu pregunta…' })).not.toBeInTheDocument()
    // Reopening keeps the last maximized state.
    await openPanel()
    expect(screen.getByRole('button', { name: 'Restaurar' })).toBeInTheDocument()
  })

  describe('panel layout (T15)', () => {
    it('shows a real header row with the title, maximize and close before the chat', async () => {
      render(HelpBubble)
      await openPanel()

      const panel = screen.getByRole('dialog', { name: 'Ayuda' })
      const header = panel.querySelector('.help-bubble__header')
      expect(header).not.toBeNull()
      expect(header?.querySelector('.help-bubble__title')).toHaveTextContent('Ayuda')

      const maximize = screen.getByRole('button', { name: 'Maximizar' })
      const close = screen.getByRole('button', { name: 'Cerrar ayuda' })
      const input = screen.getByRole('textbox', { name: 'Escribí tu pregunta…' })
      expect(header).toContainElement(maximize)
      expect(header).toContainElement(close)
      // DOM order: title, maximize, close, then the chat content.
      const title = header?.querySelector('.help-bubble__title')
      expect(title?.compareDocumentPosition(maximize)).toBe(Node.DOCUMENT_POSITION_FOLLOWING)
      expect(maximize.compareDocumentPosition(close)).toBe(Node.DOCUMENT_POSITION_FOLLOWING)
      expect(close.compareDocumentPosition(input)).toBe(Node.DOCUMENT_POSITION_FOLLOWING)
    })

    it('embeds HelpView without its big header card', async () => {
      render(HelpBubble)
      await openPanel()

      const panel = screen.getByRole('dialog', { name: 'Ayuda' })
      expect(panel.querySelector('.help-view--embedded')).not.toBeNull()
      expect(panel.querySelector('.help-view__header')).toBeNull()
    })

    it('keeps the composer in the footer and the preview in the scrolling middle', async () => {
      render(HelpBubble)
      await openPanel()

      const input = screen.getByRole('textbox', { name: 'Escribí tu pregunta…' })
      expect(input.closest('.help-view__composer')).not.toBeNull()
      expect(
        screen.getByRole('button', { name: 'Enviar' }).closest('.help-view__composer')
      ).not.toBeNull()

      await fireEvent.click(screen.getByRole('button', { name: 'Ver qué se envía' }))
      const pre = await screen.findByText(
        (_content, element) => element?.textContent === PREVIEW_TEXT
      )
      expect(pre.closest('.help-view__messages')).not.toBeNull()
      expect(pre.closest('.help-view__composer')).toBeNull()
    })
  })

  describe('panel stylesheet contracts (T15)', () => {
    const bubbleStyles = readFileSync(resolve(import.meta.dirname, 'HelpBubble.svelte'), 'utf-8')
      .split('<style>')[1]
      ?.replace(/\/\*[\s\S]*?\*\//g, '')
    const helpStyles = readFileSync(
      resolve(import.meta.dirname, '../views/HelpView.svelte'),
      'utf-8'
    )
      .split('<style>')[1]
      ?.replace(/\/\*[\s\S]*?\*\//g, '')

    function styleRule(styles: string | undefined, selector: string): string {
      const match = new RegExp(
        `(^|\\n)\\s*${selector.replace(/[.]/g, '\\.')}\\s*\\{([^}]*)\\}`
      ).exec(styles ?? '')
      expect(match, `${selector} is no longer in the stylesheet`).not.toBeNull()
      return match?.[2] ?? ''
    }

    it('lays the header out as a real row, never absolutely positioned', () => {
      const header = styleRule(bubbleStyles, '.help-bubble__header')
      expect(header).toMatch(/display:\s*flex/)
      expect(header).not.toMatch(/position:\s*absolute/)
      expect(styleRule(bubbleStyles, '.help-bubble__actions')).not.toMatch(/position:\s*absolute/)
    })

    it('lets the body fill the panel without clipping controls', () => {
      const body = styleRule(bubbleStyles, '.help-bubble__body')
      expect(body).toMatch(/flex:\s*1/)
      expect(body).toMatch(/min-height:\s*0/)
      expect(body).not.toMatch(/overflow:\s*hidden/)
    })

    it('scrolls the messages middle and pins the composer footer', () => {
      const messages = styleRule(helpStyles, '.help-view__messages')
      expect(messages).toMatch(/overflow-y:\s*auto/)
      expect(messages).toMatch(/min-height:\s*0/)
      expect(styleRule(helpStyles, '.help-view__composer')).toMatch(/flex-shrink:\s*0/)
    })

    it('bounds the diagnostics preview with its own scroll', () => {
      const preview = styleRule(helpStyles, '.help-view__preview')
      expect(preview).toMatch(/max-height:/)
      expect(preview).toMatch(/overflow:\s*auto/)
    })
  })
})
