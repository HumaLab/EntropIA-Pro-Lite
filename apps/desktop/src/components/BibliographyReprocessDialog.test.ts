import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { locale } from '$lib/i18n'
import BibliographyReprocessDialog from './BibliographyReprocessDialog.svelte'

// Mocks are set up in test-setup.ts:
//   @tauri-apps/api/core → invoke vi.fn()
//   @tauri-apps/api/event → listen vi.fn() returning Promise<vi.fn()>
const { invoke } = await import('@tauri-apps/api/core')
const { listen } = await import('@tauri-apps/api/event')

const mockInvoke = vi.mocked(invoke)
const mockListen = vi.mocked(listen)

function candidate(over: Record<string, unknown> = {}) {
  return {
    attachmentId: 'att-1',
    itemId: 'item-1',
    title: 'El oficio de historiador',
    filename: 'oficio.pdf',
    reasons: ['garbled_stored_pages'],
    flaggedPages: 3,
    ...over,
  }
}

function previewAttachment(over: Record<string, unknown> = {}) {
  return {
    attachmentId: 'att-1',
    itemId: 'item-1',
    title: 'El oficio de historiador',
    filename: 'oficio.pdf',
    pageCount: 100,
    ocrPages: 8,
    reusedOcrPages: 2,
    fixedWithoutOcr: 1,
    planHash: 'hash-1',
    busy: false,
    unreadable: null,
    ...over,
  }
}

function previewAnswer(
  attachments: Array<Record<string, unknown>>,
  totals: Record<string, unknown> = {}
) {
  return {
    attachments,
    totals: {
      attachments: attachments.length,
      pages: 100,
      ocrPages: 8,
      reusedOcrPages: 2,
      fixedWithoutOcr: 1,
      estimatedUsd: 0.24,
      ...totals,
    },
    cancelled: false,
  }
}

/** Answers each command on its own, as the real backend does. */
function backend(
  options: {
    candidates?: unknown[]
    preview?: unknown | (() => Promise<unknown>)
    confirm?: unknown
  } = {}
) {
  mockInvoke.mockImplementation(async (command: string, payload?: unknown) => {
    switch (command) {
      case 'bibliography_reprocess_candidates':
        return options.candidates ?? [candidate()]
      case 'bibliography_reprocess_preview': {
        if (typeof options.preview === 'function') return options.preview()
        return options.preview ?? previewAnswer([previewAttachment()])
      }
      case 'bibliography_reprocess_preview_cancel':
        return undefined
      case 'bibliography_reprocess_confirm': {
        void payload
        return (
          options.confirm ?? {
            batchId: 'batch-1',
            results: [{ attachmentId: 'att-1', status: 'queued' }],
          }
        )
      }
      default:
        throw new Error(`unexpected command: ${command}`)
    }
  })
}

function callsFor(command: string) {
  return mockInvoke.mock.calls.filter(([name]) => name === command)
}

/** The value shown beside a summary label in the definition list. */
function summaryValue(label: string): string | null | undefined {
  return screen.getByText(label).parentElement?.querySelector('dd')?.textContent
}

function progressHandler(): (event: { payload: { done: number; total: number } }) => void {
  const handler = mockListen.mock.calls.at(-1)?.[1]
  expect(typeof handler).toBe('function')
  return handler as unknown as (event: { payload: { done: number; total: number } }) => void
}

beforeEach(() => {
  vi.clearAllMocks()
  locale.set('es')
  mockListen.mockResolvedValue(vi.fn() as unknown as UnlistenFn)
})

describe('BibliographyReprocessDialog', () => {
  it('shows the totals and the estimated USD of the preview', async () => {
    backend()
    render(BibliographyReprocessDialog, { mode: 'library', onclose: vi.fn() })

    expect(await screen.findByText('Adjuntos')).toBeInTheDocument()
    expect(summaryValue('Adjuntos')).toBe('1')
    expect(summaryValue('Páginas')).toBe('100')
    expect(summaryValue('Páginas para GLM-OCR')).toBe('8')
    expect(summaryValue('Páginas de OCR reutilizadas')).toBe('2')
    expect(summaryValue('Páginas corregidas sin OCR')).toBe('1')
    expect(screen.getByText('≈ USD 0,24 (estimado)')).toBeInTheDocument()
    expect(
      screen.getByText(/GLM-OCR: USD 0,03 por millón de tokens · supuesto: ~3000 tokens por página\./)
    ).toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: 'Reprocesar (≈ USD 0,24)' })
    ).toBeInTheDocument()
  })

  it('confirms exactly the confirmable entries and reports queued and busy', async () => {
    backend({
      preview: previewAnswer([
        previewAttachment(),
        previewAttachment({ attachmentId: 'att-2', planHash: 'hash-2', busy: true }),
        previewAttachment({
          attachmentId: 'att-3',
          planHash: null,
          busy: false,
          unreadable: 'file_missing',
        }),
      ]),
    })
    render(BibliographyReprocessDialog, { mode: 'library', onclose: vi.fn() })

    await screen.findByText('No se encolan')
    expect(screen.getByText(/en proceso/)).toBeInTheDocument()
    expect(screen.getByText(/archivo no disponible/)).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Reprocesar (≈ USD 0,24)' }))

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('bibliography_reprocess_confirm', {
        entries: [{ attachmentId: 'att-1', planHash: 'hash-1' }],
      })
    )
    expect(await screen.findByText('Encolados: 1 adjuntos.')).toBeInTheDocument()
    expect(screen.getByText('En proceso, no encolados: 1 adjuntos.')).toBeInTheDocument()
    expect(
      screen.getByText(
        'El avance, la pausa y la cancelación están en la pestaña «Lotes» de Configuración.'
      )
    ).toBeInTheDocument()
  })

  it('cancelling during the preview stops the preview', async () => {
    backend({ preview: () => new Promise(() => {}) })
    const onclose = vi.fn()
    render(BibliographyReprocessDialog, {
      mode: 'work',
      attachmentIds: ['att-1'],
      onclose,
    })

    expect(await screen.findByText('Leyendo 0 de 1 adjuntos')).toBeInTheDocument()
    // No confirm action while the preview runs.
    expect(screen.queryByRole('button', { name: /Reprocesar/ })).toBeNull()

    await fireEvent.click(screen.getByRole('button', { name: 'Cancelar' }))

    expect(mockInvoke).toHaveBeenCalledWith('bibliography_reprocess_preview_cancel')
    expect(onclose).toHaveBeenCalledOnce()
  })

  it('reports preview progress from the progress event', async () => {
    backend({ preview: () => new Promise(() => {}) })
    render(BibliographyReprocessDialog, {
      mode: 'work',
      attachmentIds: ['att-1', 'att-2', 'att-3', 'att-4', 'att-5'],
      onclose: vi.fn(),
    })

    await waitFor(() => expect(callsFor('bibliography_reprocess_preview').length).toBe(1))
    progressHandler()({ payload: { done: 2, total: 5 } })

    expect(await screen.findByText('Leyendo 2 de 5 adjuntos')).toBeInTheDocument()
  })

  it('says so and offers no confirm button when there are no candidates', async () => {
    backend({ candidates: [] })
    render(BibliographyReprocessDialog, { mode: 'library', onclose: vi.fn() })

    expect(await screen.findByText('No hay obras con texto dañado.')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /Reprocesar/ })).toBeNull()
    expect(within(screen.getByRole('dialog')).getAllByRole('button')).toHaveLength(1)
    expect(callsFor('bibliography_reprocess_preview')).toHaveLength(0)
  })

  it('shows no OCR cost when there are no pages to OCR', async () => {
    backend({
      preview: previewAnswer([previewAttachment({ ocrPages: 0 })], {
        ocrPages: 0,
        estimatedUsd: 0,
      }),
    })
    render(BibliographyReprocessDialog, { mode: 'library', onclose: vi.fn() })

    expect(await screen.findByText('sin costo de OCR')).toBeInTheDocument()
    expect(screen.queryByText('≈ USD 0,00 (estimado)')).toBeNull()
    expect(
      screen.getByRole('button', { name: 'Reprocesar (≈ USD 0,00)' })
    ).toBeInTheDocument()
  })
})
