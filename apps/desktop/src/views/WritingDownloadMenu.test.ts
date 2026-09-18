import { fireEvent, render, screen } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import { t } from '$lib/i18n'
import WritingDownloadMenu from './WritingDownloadMenu.svelte'

/**
 * The download button in the writing bar. It asks one thing — the format — and
 * exports at once: no settings, no confirmation. The settings are the Export
 * tab's, and were chosen before.
 */

const FORMATS = [
  ['markdown', 'writing.downloadMarkdown', 'file-markdown'],
  ['html', 'writing.downloadHtml', 'file-html'],
  ['docx', 'writing.downloadDocx', 'file-docx'],
] as const

function renderMenu(busy = false) {
  const ondownload = vi.fn()
  render(WritingDownloadMenu, { props: { ondownload, busy } })
  return ondownload
}

async function openMenu() {
  await fireEvent.click(screen.getByRole('button', { name: t('writing.download') }))
  return screen.getByRole('menu', { name: t('writing.downloadFormats') })
}

describe('the download menu', () => {
  it('is closed until the button is pressed', () => {
    renderMenu()
    expect(screen.queryByRole('menu')).toBeNull()
    expect(screen.getByRole('button', { name: t('writing.download') })).toHaveAttribute(
      'aria-haspopup',
      'menu'
    )
  })

  it('offers exactly the three formats, as icons with an accessible name and a tooltip', async () => {
    renderMenu()
    const menu = await openMenu()

    const items = [...menu.querySelectorAll<HTMLElement>('[role="menuitem"]')]
    expect(items.map((item) => item.getAttribute('aria-label'))).toEqual(
      FORMATS.map(([, key]) => t(key))
    )
    for (const [index, [, key, icon]] of FORMATS.entries()) {
      const item = items[index]!
      expect(item.dataset.tooltip).toBe(t(key))
      expect(item.querySelector(`[data-action-icon="${icon}"]`)).not.toBeNull()
      // Icons only: the name is for assistive technology and the tooltip.
      expect(item.textContent?.trim()).toBe('')
    }
  })

  it.each(FORMATS)('downloads %s at once and closes', async (format, key) => {
    const ondownload = renderMenu()
    await openMenu()

    await fireEvent.click(screen.getByRole('menuitem', { name: t(key) }))

    expect(ondownload).toHaveBeenCalledExactlyOnceWith(format)
    expect(screen.queryByRole('menu')).toBeNull()
  })

  it('closes on Escape without downloading', async () => {
    const ondownload = renderMenu()
    const menu = await openMenu()

    await fireEvent.keyDown(menu, { key: 'Escape' })

    expect(screen.queryByRole('menu')).toBeNull()
    expect(ondownload).not.toHaveBeenCalled()
  })

  it('closes on a click outside without downloading', async () => {
    const ondownload = renderMenu()
    await openMenu()

    await fireEvent.pointerDown(document.body)

    expect(screen.queryByRole('menu')).toBeNull()
    expect(ondownload).not.toHaveBeenCalled()
  })

  it('moves between the formats with the left and right arrows', async () => {
    renderMenu()
    await openMenu()
    const [markdown, html] = screen.getAllByRole('menuitem')
    markdown!.focus()

    await fireEvent.keyDown(markdown!, { key: 'ArrowRight' })
    expect(document.activeElement).toBe(html)

    await fireEvent.keyDown(html!, { key: 'ArrowLeft' })
    expect(document.activeElement).toBe(markdown)
  })

  it('cannot start a second export while one is running', () => {
    renderMenu(true)
    expect(screen.getByRole('button', { name: t('writing.exportRunning') })).toBeDisabled()
  })
})
