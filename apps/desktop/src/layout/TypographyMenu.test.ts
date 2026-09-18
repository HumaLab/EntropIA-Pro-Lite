import { fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it } from 'vitest'
import TypographyMenu from './TypographyMenu.svelte'
import { FONT_PRESETS, FONT_STORAGE_KEY } from '$lib/typography'

/**
 * The typography button in the top bar and the preset cards it opens.
 *
 * Which fonts a preset uses is tested against the stylesheet elsewhere
 * (`typography-tokens.test.ts`). This is about choosing one: that every preset
 * is offered, that choosing applies and remembers it at once, and that it can
 * be done from the keyboard.
 */

afterEach(() => {
  delete document.documentElement.dataset.font
  localStorage.clear()
})

function openMenu() {
  render(TypographyMenu)
  return fireEvent.click(screen.getByRole('button', { name: /Tipografía/ }))
}

describe('the button', () => {
  it('names the preset in use', () => {
    localStorage.setItem(FONT_STORAGE_KEY, 'editorial')
    render(TypographyMenu)

    expect(screen.getByRole('button', { name: 'Tipografía: Editorial' })).toBeTruthy()
  })

  /** Restoring belongs to start-up: the menu may never be opened. */
  it('restores the stored preset on mount', () => {
    localStorage.setItem(FONT_STORAGE_KEY, 'archive')
    render(TypographyMenu)

    expect(document.documentElement.dataset.font).toBe('archive')
  })

  it('opens and closes the presets', async () => {
    await openMenu()
    expect(screen.getByRole('radiogroup', { name: 'Tipografía' })).toBeTruthy()

    await fireEvent.keyDown(screen.getByRole('radiogroup'), { key: 'Escape' })
    expect(screen.queryByRole('radiogroup')).toBeNull()
  })
})

describe('the cards', () => {
  it('offers one per preset, generated from the registry', async () => {
    await openMenu()

    const radios = screen.getAllByRole('radio')
    expect(radios).toHaveLength(FONT_PRESETS.length)
    expect(radios.map((radio) => radio.getAttribute('value'))).toEqual(
      FONT_PRESETS.map((preset) => preset.id)
    )
  })

  /** A card previews its own preset, whatever the application is using. */
  it('render each preview in its own preset', async () => {
    await openMenu()

    for (const preset of FONT_PRESETS) {
      const preview = document.querySelector(`[data-font-preview='${preset.id}']`)
      expect(preview?.getAttribute('data-font'), preset.id).toBe(preset.id)
    }
  })

  it('marks the preset in use as checked, and only that one', async () => {
    await openMenu()

    const checked = screen
      .getAllByRole('radio')
      .filter((radio) => (radio as HTMLInputElement).checked)
    expect(checked.map((radio) => radio.getAttribute('value'))).toEqual(['academic'])
  })

  it('apply and remember a preset the moment it is chosen', async () => {
    await openMenu()

    await fireEvent.click(screen.getByRole('radio', { name: /Moderna/ }))

    expect(document.documentElement.dataset.font).toBe('modern')
    expect(localStorage.getItem(FONT_STORAGE_KEY)).toBe('modern')
    expect(screen.getByRole('button', { name: 'Tipografía: Moderna' })).toBeTruthy()
  })

  /** Native radios: arrow keys move the choice, with no key handling of our own. */
  it('are a real radio group, so the keyboard works', async () => {
    await openMenu()

    const names = new Set(screen.getAllByRole('radio').map((radio) => radio.getAttribute('name')))
    expect(names.size).toBe(1)
  })

  it('name the families of each preset for assistive technology', async () => {
    await openMenu()

    const academic = screen.getByRole('radio', { name: /Académica/ })
    expect(academic.getAttribute('aria-describedby')).toBeTruthy()
    const description = document.getElementById(academic.getAttribute('aria-describedby')!)
    expect(description?.textContent).toContain('Source Sans 3')
    expect(description?.textContent).toContain('Source Serif 4')
    expect(description?.textContent).toContain('JetBrains Mono')
  })
})
