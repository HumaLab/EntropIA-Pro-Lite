import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { locale } from '$lib/i18n'
import { HELP_BUBBLE_STORAGE_KEY } from '$lib/help-bubble'
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

function backend(): void {
  mockInvoke.mockImplementation(async (command: string) => {
    if (command === 'help_status') return { hasOpenrouterKey: true }
    if (command === 'help_faq_list') return []
    return undefined
  })
}

async function openPanel(): Promise<void> {
  await fireEvent.click(bubbleButton())
  await screen.findByRole('textbox', { name: 'Escribí tu pregunta…' })
}

describe('HelpBubble (S13, D9)', () => {
  beforeEach(() => {
    locale.set('es')
    localStorage.clear()
    navigationRef.navigate.mockReset()
    navigationRef.openRootSection.mockReset()
    mockInvoke.mockReset()
    backend()
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

  it('a drag beyond the threshold moves the bubble, saves the position, and does not open the panel', async () => {
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
    expect(localStorage.getItem(HELP_BUBBLE_STORAGE_KEY)).toBe(
      `{"x":${from.x - 100},"y":${from.y - 50}}`
    )
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

  it('restores the stored position on mount', () => {
    localStorage.setItem(HELP_BUBBLE_STORAGE_KEY, '{"x":120,"y":200}')
    render(HelpBubble)

    const button = bubbleButton()
    expect(button.style.left).toBe('120px')
    expect(button.style.top).toBe('200px')
  })

  it('clamps a stored position that no longer fits the window', () => {
    localStorage.setItem(HELP_BUBBLE_STORAGE_KEY, '{"x":5000,"y":5000}')
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

  it('keeps the dragged position across remounts', async () => {
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

  it('the panel opens toward the inside of the window, never off-screen', async () => {
    render(HelpBubble)
    await openPanel()

    // Default bottom-right: the panel grows leftwards and upwards.
    const panel = screen.getByRole('dialog', { name: 'Ayuda' })
    expect(panel).toHaveAttribute('data-side', 'left')
    expect(panel).toHaveAttribute('data-vertical', 'above')
  })

  it('the panel flips sides when the bubble sits at the top-left', async () => {
    localStorage.setItem(HELP_BUBBLE_STORAGE_KEY, '{"x":8,"y":8}')
    render(HelpBubble)
    await openPanel()

    const panel = screen.getByRole('dialog', { name: 'Ayuda' })
    expect(panel).toHaveAttribute('data-side', 'right')
    expect(panel).toHaveAttribute('data-vertical', 'below')
  })

  it('re-clamps the bubble when the window shrinks', async () => {
    localStorage.setItem(HELP_BUBBLE_STORAGE_KEY, '{"x":900,"y":700}')
    render(HelpBubble)
    const button = bubbleButton()
    expect(button.style.left).toBe('900px')

    const originalWidth = window.innerWidth
    const originalHeight = window.innerHeight
    Object.defineProperty(window, 'innerWidth', { value: 500, configurable: true })
    Object.defineProperty(window, 'innerHeight', { value: 400, configurable: true })
    try {
      window.dispatchEvent(new Event('resize'))
      await waitFor(() => {
        expect(Number.parseFloat(button.style.left)).toBeLessThanOrEqual(500 - 56)
        expect(Number.parseFloat(button.style.top)).toBeLessThanOrEqual(400 - 56)
      })
    } finally {
      Object.defineProperty(window, 'innerWidth', { value: originalWidth, configurable: true })
      Object.defineProperty(window, 'innerHeight', { value: originalHeight, configurable: true })
    }
  })
})
