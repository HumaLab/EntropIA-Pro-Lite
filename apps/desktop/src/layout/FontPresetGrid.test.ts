import { fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it } from 'vitest'
import FontPresetGrid from './FontPresetGrid.svelte'
import { FONT_PRESETS, FONT_STORAGE_KEY } from '$lib/typography'

/**
 * The typography preset cards, embedded in the Apariencia settings tab.
 *
 * Extracted from TypographyMenu.svelte's popover — same cards, same
 * behaviour, minus the trigger button and open/close chrome the tab does not
 * need. Which fonts a preset uses is tested against the stylesheet elsewhere
 * (`typography-tokens.test.ts`); this is about choosing one.
 */

afterEach(() => {
  delete document.documentElement.dataset.font
  localStorage.clear()
})

describe('mounting', () => {
  /** Restoring belongs to start-up in appearance.ts; this only re-applies it
   *  so the grid always reflects what is actually on screen. */
  it('restores the stored preset on mount', () => {
    localStorage.setItem(FONT_STORAGE_KEY, 'archive')
    render(FontPresetGrid)

    expect(document.documentElement.dataset.font).toBe('archive')
  })

  it('is a radiogroup with no trigger button', () => {
    render(FontPresetGrid)

    expect(screen.getByRole('radiogroup', { name: 'Tipografía' })).toBeTruthy()
    expect(screen.queryByRole('button')).toBeNull()
  })
})

describe('the cards', () => {
  it('offers one per preset, generated from the registry', () => {
    render(FontPresetGrid)

    const radios = screen.getAllByRole('radio')
    expect(radios).toHaveLength(FONT_PRESETS.length)
    expect(radios.map((radio) => radio.getAttribute('value'))).toEqual(
      FONT_PRESETS.map((preset) => preset.id)
    )
  })

  /** A card previews its own preset, whatever the application is using. */
  it('render each preview in its own preset', () => {
    render(FontPresetGrid)

    for (const preset of FONT_PRESETS) {
      const preview = document.querySelector(`[data-font-preview='${preset.id}']`)
      expect(preview?.getAttribute('data-font'), preset.id).toBe(preset.id)
    }
  })

  it('marks the preset in use as checked, and only that one', () => {
    render(FontPresetGrid)

    const checked = screen
      .getAllByRole('radio')
      .filter((radio) => (radio as HTMLInputElement).checked)
    expect(checked.map((radio) => radio.getAttribute('value'))).toEqual(['academic'])
  })

  it('apply and remember a preset the moment it is chosen', async () => {
    render(FontPresetGrid)

    await fireEvent.click(screen.getByRole('radio', { name: /Moderna/ }))

    expect(document.documentElement.dataset.font).toBe('modern')
    expect(localStorage.getItem(FONT_STORAGE_KEY)).toBe('modern')
  })

  /** Native radios: arrow keys move the choice, with no key handling of our own. */
  it('are a real radio group, so the keyboard works', () => {
    render(FontPresetGrid)

    const names = new Set(screen.getAllByRole('radio').map((radio) => radio.getAttribute('name')))
    expect(names.size).toBe(1)
  })

  it('name the families of each preset for assistive technology', () => {
    render(FontPresetGrid)

    const academic = screen.getByRole('radio', { name: /Académica/ })
    expect(academic.getAttribute('aria-describedby')).toBeTruthy()
    const description = document.getElementById(academic.getAttribute('aria-describedby')!)
    expect(description?.textContent).toContain('Source Sans 3')
    expect(description?.textContent).toContain('Source Serif 4')
    expect(description?.textContent).toContain('JetBrains Mono')
  })
})
