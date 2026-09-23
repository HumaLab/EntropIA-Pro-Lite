/** @vitest-environment jsdom */

import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, describe, expect, it, vi } from 'vitest'
import ActiveProcessBand from './ActiveProcessBand.svelte'

describe('ActiveProcessBand', () => {
  afterEach(() => {
    cleanup()
  })

  it('renders the title, progress and open action from props', () => {
    render(ActiveProcessBand, {
      title: 'OCR',
      progress: '428 / 1.244 páginas · 34 %',
      openLabel: 'Ver lote →',
      onOpen: vi.fn(),
    })

    expect(screen.getByText('OCR')).toBeInTheDocument()
    expect(screen.getByText('428 / 1.244 páginas · 34 %')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Ver lote →' })).toBeInTheDocument()
  })

  it('calls onOpen when the open action is clicked', async () => {
    const onOpen = vi.fn()
    render(ActiveProcessBand, {
      title: 'Embeddings',
      progress: '10 / 20 páginas · 50 %',
      openLabel: 'Ver lote →',
      onOpen,
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Ver lote →' }))

    expect(onOpen).toHaveBeenCalledOnce()
  })
})
