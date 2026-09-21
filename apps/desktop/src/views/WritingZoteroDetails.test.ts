import { invoke } from '@tauri-apps/api/core'
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import WritingZoteroDetails, { fetchZoteroItemDetail } from './WritingZoteroDetails.svelte'
import type { LibraryEntry } from '$lib/writing-zotero'

const mockInvoke = vi.mocked(invoke)

const ENTRY: LibraryEntry = {
  key: 'DETAIL1',
  itemVersion: 3,
  libraryType: 'user',
  libraryId: '0',
  title: 'Los orígenes',
  authors: 'Moore',
  year: '1973',
  csl_json: JSON.stringify({
    id: 'moore1973',
    type: 'book',
    title: 'Los orígenes',
    author: [{ family: 'Moore', given: 'Barrington' }],
    issued: { 'date-parts': [[1973]] },
  }),
}

const CONFIRMED = {
  status: 'confirmed',
  verifiedAt: Date.UTC(2024, 4, 6, 12, 0, 0),
  item: {
    itemKey: 'DETAIL1',
    itemType: 'book',
    title: 'Los orígenes',
    creators: [{ creatorType: 'author', firstName: 'Barrington', lastName: 'Moore' }],
    publicationTitle: 'Prensa sintética',
    publisher: 'EntropIA',
    date: '1973',
    doi: '10.0000/sintetico',
    isbn: '978-0-00-000000-0',
    abstract: 'Un resumen sintético.',
    language: 'es',
    url: 'https://sintetico.invalido/detalle1',
    itemVersion: 3,
    collections: ['Alfa'],
    tags: ['tesis'],
    attachments: [
      {
        attachmentKey: 'PDF01',
        contentType: 'application/pdf',
        linkMode: 'linked_file',
        filename: 'capitulo.pdf',
        url: 'https://sintetico.invalido/capitulo.pdf',
      },
    ],
  },
} as const

beforeEach(() => {
  vi.clearAllMocks()
  mockInvoke.mockReset()
})

describe('E1c-3 work details (ficha), confirmed state', () => {
  it('renders the full ficha with a Sincronizado chip carrying the verifiedAt date', () => {
    render(WritingZoteroDetails, {
      props: { entry: ENTRY, detail: structuredClone(CONFIRMED) as never },
    })

    expect(screen.getByText('Los orígenes')).toBeInTheDocument()
    expect(screen.getByText(/Barrington/)).toBeInTheDocument()
    expect(screen.getByText('Prensa sintética')).toBeInTheDocument()
    expect(screen.getByText('EntropIA')).toBeInTheDocument()
    expect(screen.getByText('10.0000/sintetico')).toBeInTheDocument()
    expect(screen.getByText('978-0-00-000000-0')).toBeInTheDocument()
    expect(screen.getByText('Un resumen sintético.')).toBeInTheDocument()
    expect(screen.getByText('Alfa')).toBeInTheDocument()
    expect(screen.getByText('tesis')).toBeInTheDocument()
    // Attachment names are listed; files are never opened from the ficha.
    expect(screen.getByText('capitulo.pdf')).toBeInTheDocument()
    expect(screen.queryByRole('link')).not.toBeInTheDocument()

    const chip = screen.getByText(/Sincronizado/)
    expect(chip).toBeInTheDocument()
    expect(chip.textContent).toContain('2024-05-06')
  })
})

describe('E1c-3 work details (ficha), lost-link state', () => {
  it('shows the unavailable chip, the tombstone reason and the last metadata', () => {
    render(WritingZoteroDetails, {
      props: {
        entry: ENTRY,
        detail: {
          status: 'lost_link',
          tombstone: { observedAt: 7, remoteVersion: 4, reason: 'página de borrado remoto' },
          item: structuredClone(CONFIRMED.item),
        } as never,
      },
    })

    expect(screen.getByText('Ítem no disponible en Zotero')).toBeInTheDocument()
    expect(screen.getByText(/página de borrado remoto/)).toBeInTheDocument()
    // The last snapshot is still shown, not invented metadata.
    expect(screen.getByText('Los orígenes')).toBeInTheDocument()
    expect(screen.getByText('capitulo.pdf')).toBeInTheDocument()
  })

  it('shows the unavailable chip without inventing metadata when the snapshot is gone', () => {
    render(WritingZoteroDetails, {
      props: {
        entry: ENTRY,
        detail: {
          status: 'lost_link',
          tombstone: { observedAt: 7, remoteVersion: 4, reason: 'página de borrado remoto' },
          item: null,
        } as never,
      },
    })

    expect(screen.getByText('Ítem no disponible en Zotero')).toBeInTheDocument()
    expect(screen.queryByText('Los orígenes')).not.toBeInTheDocument()
    expect(screen.queryByText('capitulo.pdf')).not.toBeInTheDocument()
  })
})

describe('E1c-3 work details (ficha), offline states render the held CSL', () => {
  it.each(['not_in_catalog', 'catalog_unavailable'] as const)(
    '%s renders the held entry with a local-copy chip and never claims Zotero is closed',
    (status) => {
      render(WritingZoteroDetails, { props: { entry: ENTRY, detail: { status } as never } })

      expect(screen.getByText('Los orígenes')).toBeInTheDocument()
      expect(screen.getByText(/Moore/)).toBeInTheDocument()
      expect(screen.getByText(/1973/)).toBeInTheDocument()
      expect(screen.getByText('Copia local sin verificar ahora')).toBeInTheDocument()
      expect(document.body.textContent).not.toMatch(/cerrad/i)
      expect(document.body.textContent).not.toMatch(/not installed/i)
    }
  )
})

describe('E1c-3 work details (ficha), fetch entry point', () => {
  it('invokes writing_zotero_item_detail with camelCase library and item identity', async () => {
    mockInvoke.mockResolvedValue({ status: 'not_in_catalog' })

    await fetchZoteroItemDetail('user', '0', 'DETAIL1')

    expect(mockInvoke).toHaveBeenCalledWith('writing_zotero_item_detail', {
      libraryType: 'user',
      libraryId: '0',
      itemKey: 'DETAIL1',
    })
  })

  it('loads the ficha on mount and shows it once it answers', async () => {
    mockInvoke.mockResolvedValue(structuredClone(CONFIRMED))

    render(WritingZoteroDetails, { props: { entry: ENTRY } })

    expect(screen.getByText(/Leyendo la ficha/)).toBeInTheDocument()
    expect(await screen.findByText(/Sincronizado/)).toBeInTheDocument()
    expect(mockInvoke).toHaveBeenCalledWith('writing_zotero_item_detail', {
      libraryType: 'user',
      libraryId: '0',
      itemKey: 'DETAIL1',
    })
  })

  it('keeps an invoke failure honest with a retry instead of an invented state', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('la base está ocupada'))
    mockInvoke.mockResolvedValueOnce(structuredClone(CONFIRMED))

    render(WritingZoteroDetails, { props: { entry: ENTRY } })

    expect(await screen.findByRole('button', { name: /Reintentar/ })).toBeInTheDocument()
    // No state chip is claimed while nothing answered.
    expect(screen.queryByText(/Sincronizado/)).not.toBeInTheDocument()
    expect(screen.queryByText('Ítem no disponible en Zotero')).not.toBeInTheDocument()
    expect(screen.queryByText('Copia local sin verificar ahora')).not.toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: /Reintentar/ }))

    expect(await screen.findByText(/Sincronizado/)).toBeInTheDocument()
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledTimes(2))
  })

  it('closes back to the list without touching the library', async () => {
    const onclose = vi.fn()
    render(WritingZoteroDetails, {
      props: { entry: ENTRY, detail: { status: 'not_in_catalog' } as never, onclose },
    })

    await fireEvent.click(screen.getByRole('button', { name: /Volver/ }))

    expect(onclose).toHaveBeenCalledOnce()
    expect(mockInvoke).not.toHaveBeenCalled()
  })
})
