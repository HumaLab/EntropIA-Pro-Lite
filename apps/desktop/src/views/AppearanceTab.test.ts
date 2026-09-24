import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import AppearanceTab from './AppearanceTab.svelte'
import { locale } from '$lib/i18n'
import { THEME_STORAGE_KEY } from '$lib/theme'
import { CONTRAST_STORAGE_KEY } from '$lib/contrast'
import { FONT_STORAGE_KEY } from '$lib/typography'
import { resetZoom, ZOOM_MAX, ZOOM_MIN, zoomIn, zoomOut } from '$lib/zoom'

/**
 * Apariencia: the five preferences the top bar used to scatter across its own
 * icons, now together in one Configuración tab (user decision, 2026-09-24).
 */

const { setZoomMock } = vi.hoisted(() => ({ setZoomMock: vi.fn() }))

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockResolvedValue(undefined) }))
// The zoom module itself is real — the point of these tests is that the
// control stays in step with it, including changes it never triggered.
vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ setZoom: setZoomMock }),
}))

const themeTrigger = () => screen.getByRole('button', { name: 'Tema Oscuro' })
const contrastTrigger = () => screen.getByRole('button', { name: 'Contraste Contraste normal' })
const languageTrigger = () => screen.getByRole('button', { name: 'Idioma Español' })
const zoomDecrease = () => screen.getByRole('button', { name: 'Reducir zoom' })
const zoomIncrease = () => screen.getByRole('button', { name: 'Aumentar zoom' })
const zoomReset = () => screen.getByRole('button', { name: 'Restablecer zoom' })
const zoomLevel = () => screen.getByTestId('appearance-zoom-level')

describe('AppearanceTab', () => {
  beforeEach(async () => {
    locale.set('es')
    localStorage.clear()
    delete document.documentElement.dataset.theme
    delete document.documentElement.dataset.contrast
    delete document.documentElement.dataset.font
    setZoomMock.mockReset().mockResolvedValue(undefined)
    await resetZoom()
  })

  afterEach(() => {
    delete document.documentElement.dataset.theme
    delete document.documentElement.dataset.contrast
    delete document.documentElement.dataset.font
    localStorage.clear()
  })

  describe('theme', () => {
    it('shows the current theme and offers every theme in the cycle', async () => {
      render(AppearanceTab)

      await fireEvent.click(themeTrigger())

      const options = screen.getAllByRole('menuitemradio')
      expect(options.map((option) => option.textContent?.trim())).toEqual([
        'Oscuro',
        'Cálido',
        'Claro',
        'Lite',
      ])
      expect(screen.getByRole('menuitemradio', { name: 'Oscuro' })).toHaveAttribute(
        'aria-checked',
        'true'
      )
    })

    it('applies and persists the chosen theme', async () => {
      render(AppearanceTab)
      await fireEvent.click(themeTrigger())

      await fireEvent.click(screen.getByRole('menuitemradio', { name: 'Cálido' }))

      expect(document.documentElement.dataset.theme).toBe('dim')
      expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe('dim')
      expect(screen.getByRole('button', { name: 'Tema Cálido' })).toBeInTheDocument()
    })
  })

  describe('contrast', () => {
    it('applies and persists the chosen contrast level', async () => {
      render(AppearanceTab)
      await fireEvent.click(contrastTrigger())

      await fireEvent.click(screen.getByRole('menuitemradio', { name: 'Contraste alto' }))

      expect(document.documentElement.dataset.contrast).toBe('high')
      expect(localStorage.getItem(CONTRAST_STORAGE_KEY)).toBe('high')
    })
  })

  describe('language', () => {
    it('changes the interface language', async () => {
      render(AppearanceTab)
      await fireEvent.click(languageTrigger())

      await fireEvent.click(screen.getByRole('menuitemradio', { name: 'English' }))

      await waitFor(() => {
        expect(screen.getByRole('button', { name: 'Language English' })).toBeInTheDocument()
      })
    })
  })

  describe('zoom', () => {
    it('shows the current level', () => {
      render(AppearanceTab)
      expect(zoomLevel()).toHaveTextContent('100%')
    })

    it('steps up and down by 5%', async () => {
      render(AppearanceTab)

      await fireEvent.click(zoomIncrease())
      await waitFor(() => expect(zoomLevel()).toHaveTextContent('105%'))

      await fireEvent.click(zoomDecrease())
      await fireEvent.click(zoomDecrease())
      await waitFor(() => expect(zoomLevel()).toHaveTextContent('95%'))
    })

    it('restores 100% from the reset action', async () => {
      render(AppearanceTab)
      await fireEvent.click(zoomIncrease())
      await waitFor(() => expect(zoomLevel()).toHaveTextContent('105%'))

      await fireEvent.click(zoomReset())
      await waitFor(() => expect(zoomLevel()).toHaveTextContent('100%'))
    })

    it('follows a zoom change it did not trigger, such as the keyboard shortcut', async () => {
      render(AppearanceTab)

      await zoomIn()
      await zoomIn()

      await waitFor(() => expect(zoomLevel()).toHaveTextContent('110%'))
    })

    it('disables the step that would leave the allowed range', async () => {
      render(AppearanceTab)

      while ((await zoomIn()) < ZOOM_MAX);
      await waitFor(() => expect(zoomIncrease()).toBeDisabled())
      expect(zoomDecrease()).toBeEnabled()

      while ((await zoomOut()) > ZOOM_MIN);
      await waitFor(() => expect(zoomDecrease()).toBeDisabled())
      expect(zoomIncrease()).toBeEnabled()
    })
  })

  describe('zoom', () => {
    it('does not print the keyboard shortcut hint', () => {
      render(AppearanceTab)

      expect(screen.queryByText('Ctrl + / Ctrl − / Ctrl 0')).not.toBeInTheDocument()
    })
  })

  describe('typography', () => {
    function fontTrigger() {
      return screen.getByRole('button', { name: /Tipografía/ })
    }

    it('is a dropdown like Tema: the trigger names the current preset and the cards stay closed', () => {
      render(AppearanceTab)

      expect(fontTrigger()).toHaveTextContent('Académica')
      expect(screen.queryByRole('radio', { name: /Moderna/ })).not.toBeInTheDocument()
    })

    it('opens the preset cards, applies and persists a choice, then closes', async () => {
      render(AppearanceTab)

      await fireEvent.click(fontTrigger())
      await fireEvent.click(screen.getByRole('radio', { name: /Moderna/ }))

      expect(document.documentElement.dataset.font).toBe('modern')
      expect(localStorage.getItem(FONT_STORAGE_KEY)).toBe('modern')
      await waitFor(() =>
        expect(screen.queryByRole('radio', { name: /Moderna/ })).not.toBeInTheDocument()
      )
      expect(fontTrigger()).toHaveTextContent('Moderna')
    })
  })
})
