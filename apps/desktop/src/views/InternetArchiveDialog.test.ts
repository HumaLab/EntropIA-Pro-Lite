/** @vitest-environment jsdom */

import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'
import { DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT } from '$lib/document-explorer'
import InternetArchiveDialog from './InternetArchiveDialog.svelte'

const { iaRef, importRef, storeRef } = vi.hoisted(() => ({
  iaRef: {
    iaSearch: vi.fn(),
    iaListCollection: vi.fn(),
    iaResolveIdentifier: vi.fn(),
    iaGetItem: vi.fn(),
    iaPrepareImport: vi.fn(),
    iaStartImport: vi.fn(),
    iaCancelImport: vi.fn(),
    iaDiscardDraft: vi.fn(),
    iaGetImportStatus: vi.fn(),
    iaListJobs: vi.fn(),
    onIaJobUpdated: vi.fn(),
  },
  importRef: {
    importVerifiedFiles: vi.fn(),
  },
  storeRef: {
    current: {
      collections: {
        findAll: vi.fn(),
        create: vi.fn(),
      },
    },
  },
}))

vi.mock('$lib/internet-archive', async (importOriginal) => {
  const actual = await importOriginal<typeof import('$lib/internet-archive')>()
  return {
    ...actual,
    iaSearch: iaRef.iaSearch,
    iaListCollection: iaRef.iaListCollection,
    iaResolveIdentifier: iaRef.iaResolveIdentifier,
    iaGetItem: iaRef.iaGetItem,
    iaPrepareImport: iaRef.iaPrepareImport,
    iaStartImport: iaRef.iaStartImport,
    iaCancelImport: iaRef.iaCancelImport,
    iaDiscardDraft: iaRef.iaDiscardDraft,
    iaGetImportStatus: iaRef.iaGetImportStatus,
    iaListJobs: iaRef.iaListJobs,
    onIaJobUpdated: iaRef.onIaJobUpdated,
  }
})

vi.mock('$lib/internet-archive-import', async (importOriginal) => {
  const actual = await importOriginal<typeof import('$lib/internet-archive-import')>()
  return {
    ...actual,
    importVerifiedFiles: importRef.importVerifiedFiles,
  }
})

vi.mock('$lib/db', () => ({ getStore: () => storeRef.current }))

type JobHandler = (jobId: string) => void

let jobHandler: JobHandler | null = null
const unlisten = vi.fn()

const collection = (id: string, name: string) => ({
  id,
  name,
  description: null,
  createdAt: 1,
  updatedAt: 1,
})

const searchItem = (patch: Record<string, unknown> = {}) => ({
  identifier: 'some-id',
  title: 'Some Title',
  creators: ['Some Author'],
  date: '1920',
  collections: ['opensource'],
  restricted: false,
  ...patch,
})

const pdf = (patch: Record<string, unknown> = {}) => ({
  name: 'doc_text.pdf',
  format: 'Text PDF',
  kind: 'pdf_text',
  source: 'original',
  size: 1024,
  mtime: null,
  md5: null,
  sha1: null,
  availability: 'available',
  ...patch,
})

const itemDetails = (patch: Record<string, unknown> = {}) => ({
  identifier: 'some-id',
  title: 'Some Title',
  creators: ['Some Author'],
  date: '1920',
  languages: [],
  collections: ['opensource'],
  rights: null,
  licenseUrl: null,
  mediatype: 'texts',
  restricted: false,
  pdfs: [pdf()],
  truncated: false,
  ...patch,
})

const prepareEntry = (patch: Record<string, unknown> = {}) => ({
  identifier: 'some-id',
  fileName: 'doc_text.pdf',
  availability: 'available',
  size: 1024,
  kind: 'pdf',
  duplicate: { status: 'new' },
  ...patch,
})

const prepareResult = (patch: Record<string, unknown> = {}) => ({
  jobId: 'job-1',
  summary: {
    totalFiles: 1,
    availableFiles: 1,
    restrictedFiles: 0,
    knownBytes: 1024,
    unknownSizeFiles: 0,
    newFiles: 1,
    alreadyInTarget: 0,
    inOtherCollection: 0,
    newVersion: 0,
    entries: [prepareEntry()],
  },
  rejected: [],
  ...patch,
})

const jobFile = (patch: Record<string, unknown> = {}) => ({
  index: 0,
  identifier: 'some-id',
  fileName: 'doc_text.pdf',
  displayFileName: 'doc_text.pdf',
  state: 'verified',
  attempts: 0,
  error: null,
  expectedSize: 1024,
  pageCount: null,
  ...patch,
})

const jobStatus = (patch: Record<string, unknown> = {}) => ({
  id: 'job-1',
  state: 'running',
  targetCollectionId: 'col-1',
  files: [jobFile()],
  running: true,
  ...patch,
})

const importOutcome = (patch: Record<string, unknown> = {}) => ({
  imported: [{ index: 0, itemId: 'doc-1' }],
  confirmedExisting: [],
  failed: [],
  notProcessed: [],
  cancelled: false,
  ...patch,
})

const onClose = vi.fn()
const onImported = vi.fn()

function open(props: Record<string, unknown> = {}) {
  return render(InternetArchiveDialog, { props: { onClose, onImported, ...props } })
}

function discoveryTabs(): HTMLElement {
  return screen.getByRole('group', { name: 'Descubrimiento' })
}

async function searchForResults(props: Record<string, unknown> = {}) {
  iaRef.iaSearch.mockResolvedValue({
    numFound: 2,
    items: [
      searchItem(),
      searchItem({
        identifier: 'other-id',
        title: null,
        creators: [],
        date: null,
        restricted: true,
      }),
    ],
  })
  open(props)
  await fireEvent.input(screen.getByLabelText('Título'), { target: { value: 'Chaplin' } })
  const tabs = discoveryTabs()
  await fireEvent.click(within(tabs).getByRole('button', { name: 'Buscar' }))
  await fireEvent.click(screen.getByRole('button', { name: 'Buscar en el archivo' }))
  await screen.findByText('Some Title')
}

async function openItem(
  props: Record<string, unknown> = {},
  details: Record<string, unknown> = {}
) {
  await searchForResults(props)
  iaRef.iaGetItem.mockResolvedValue(itemDetails(details))
  await fireEvent.click(screen.getAllByRole('button', { name: 'Ver archivos' })[0]!)
  await screen.findByText('Archivos')
}

async function selectFirstPdf(props: Record<string, unknown> = {}) {
  await openItem(props)
  await fireEvent.click(screen.getByRole('checkbox', { name: 'Seleccionar doc_text.pdf' }))
  await screen.findByText('Seleccionados: 1')
}

async function goToDestination(props: Record<string, unknown> = {}) {
  await selectFirstPdf(props)
  await fireEvent.click(screen.getByRole('button', { name: 'Continuar' }))
  await screen.findByText('Destino')
}

async function prepareIntoCollection(
  choice: 'col-1' | 'new' = 'col-1',
  props: Record<string, unknown> = {}
) {
  await goToDestination(props)
  iaRef.iaPrepareImport.mockResolvedValue(prepareResult())
  if (choice === 'new') {
    storeRef.current.collections.create.mockResolvedValue(collection('col-new', 'Archivo nuevo'))
    await fireEvent.click(screen.getByRole('radio', { name: /Nueva colección/ }))
    await fireEvent.input(screen.getByLabelText('Nombre de la nueva colección'), {
      target: { value: 'Archivo nuevo' },
    })
  } else {
    await fireEvent.click(screen.getByRole('radio', { name: /Voces/ }))
  }
  await fireEvent.click(screen.getByRole('button', { name: 'Preparar importación' }))
  await screen.findByText('Revisa la preparación')
}

beforeEach(() => {
  locale.set('es')
  vi.clearAllMocks()
  jobHandler = null
  storeRef.current = {
    collections: {
      findAll: vi
        .fn()
        .mockResolvedValue([collection('col-1', 'Voces'), collection('col-2', 'Obras')]),
      create: vi.fn(),
    },
  }
  iaRef.onIaJobUpdated.mockImplementation((handler: JobHandler) => {
    jobHandler = handler
    return Promise.resolve(unlisten)
  })
  importRef.importVerifiedFiles.mockResolvedValue(importOutcome())
  iaRef.iaDiscardDraft.mockResolvedValue(undefined)
  iaRef.iaListJobs.mockResolvedValue([])
  onClose.mockReset()
  onImported.mockReset()
})

describe('InternetArchiveDialog discovery', () => {
  it('searches and renders title, creator, date, identifier and the restricted badge', async () => {
    await searchForResults()

    expect(iaRef.iaSearch).toHaveBeenCalledWith(
      expect.objectContaining({ title: 'Chaplin', textsOnly: true, kinds: ['pdf'] }),
      1,
      50
    )
    expect(screen.getByText('Some Title')).toBeInTheDocument()
    expect(screen.getByText('Some Author')).toBeInTheDocument()
    expect(screen.getByText('1920')).toBeInTheDocument()
    expect(screen.getByText('some-id')).toBeInTheDocument()
    // Null titles fall back to the identifier, so it shows twice for that hit.
    expect(screen.getAllByText('other-id')).toHaveLength(2)
    expect(screen.getByText('restringido')).toBeInTheDocument()
    expect(screen.getByText('2 resultados')).toBeInTheDocument()
  })

  it('never renders remote HTML: markup in a title stays text', async () => {
    iaRef.iaSearch.mockResolvedValue({
      numFound: 1,
      items: [searchItem({ identifier: 'x-id', title: '<script>alert(1)</script>' })],
    })
    open()
    const tabs = discoveryTabs()
    await fireEvent.click(within(tabs).getByRole('button', { name: 'Buscar' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar en el archivo' }))

    expect(await screen.findByText('<script>alert(1)</script>')).toBeInTheDocument()
    expect(document.querySelector('script')).toBeNull()
  })

  it('shows an empty state when the search finds nothing', async () => {
    iaRef.iaSearch.mockResolvedValue({ numFound: 0, items: [] })
    open()
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar en el archivo' }))

    expect(await screen.findByText('Sin resultados para esta búsqueda.')).toBeInTheDocument()
  })

  it('pages through search results', async () => {
    iaRef.iaSearch.mockResolvedValue({ numFound: 125, items: [searchItem()] })
    open()
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar en el archivo' }))
    await screen.findByText('Página 1 de 3')

    iaRef.iaSearch.mockResolvedValue({ numFound: 125, items: [searchItem()] })
    await fireEvent.click(screen.getByRole('button', { name: 'Siguiente' }))
    await screen.findByText('Página 2 de 3')

    expect(iaRef.iaSearch).toHaveBeenLastCalledWith(expect.anything(), 2, 50)
  })

  it('filters by file kind: PDF by default, any combination of the three', async () => {
    iaRef.iaSearch.mockResolvedValue({ numFound: 0, items: [] })
    open()
    const kinds = screen.getByRole('group', { name: 'Tipo de archivo' })
    const pdfKind = within(kinds).getByRole('button', { name: 'PDF' })
    const textKind = within(kinds).getByRole('button', { name: 'PDF + texto' })
    const imageKind = within(kinds).getByRole('button', { name: 'Imágenes' })
    expect(pdfKind).toHaveAttribute('aria-pressed', 'true')
    expect(textKind).toHaveAttribute('aria-pressed', 'false')
    expect(imageKind).toHaveAttribute('aria-pressed', 'false')

    await fireEvent.click(imageKind)
    await fireEvent.click(textKind)
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar en el archivo' }))

    expect(iaRef.iaSearch).toHaveBeenLastCalledWith(
      expect.objectContaining({ kinds: ['pdf', 'pdf_text', 'image'] }),
      1,
      50
    )
  })

  it('refuses to search with no file kind and says why', async () => {
    open()
    const kinds = screen.getByRole('group', { name: 'Tipo de archivo' })
    await fireEvent.click(within(kinds).getByRole('button', { name: 'PDF' }))

    expect(screen.getByText('Elegí al menos un tipo de archivo.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Buscar en el archivo' })).toBeDisabled()
  })

  it('shows only the record files of the chosen kinds', async () => {
    await searchForResults()
    iaRef.iaGetItem.mockResolvedValue(
      itemDetails({
        pdfs: [pdf(), pdf({ name: 'scan.pdf', format: 'Image Container PDF', kind: 'pdf' })],
      })
    )
    const kinds = screen.getByRole('group', { name: 'Tipo de archivo' })
    await fireEvent.click(within(kinds).getByRole('button', { name: 'PDF + texto' }))
    await fireEvent.click(within(kinds).getByRole('button', { name: 'PDF' }))
    await fireEvent.click(screen.getAllByRole('button', { name: 'Ver archivos' })[0]!)
    await screen.findByText('Archivos')

    expect(screen.getByRole('checkbox', { name: 'Seleccionar doc_text.pdf' })).toBeInTheDocument()
    expect(screen.queryByRole('checkbox', { name: 'Seleccionar scan.pdf' })).toBeNull()

    // PDF covers every PDF, Text PDF included.
    await fireEvent.click(within(kinds).getByRole('button', { name: 'PDF' }))
    expect(screen.getByRole('checkbox', { name: 'Seleccionar scan.pdf' })).toBeInTheDocument()
  })

  async function searchPage(items: Array<Record<string, unknown>>) {
    iaRef.iaSearch.mockResolvedValue({ numFound: items.length, items: items.map(searchItem) })
    open()
    await fireEvent.input(screen.getByLabelText('Título'), { target: { value: 'Chaplin' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar en el archivo' }))
    await screen.findAllByRole('button', { name: 'Ver archivos' })
  }

  const recordFiles: Record<string, ReturnType<typeof itemDetails>> = {
    'some-id': itemDetails({
      pdfs: [
        pdf(),
        pdf({ name: 'scan.pdf', format: 'Image Container PDF', kind: 'pdf' }),
        pdf({ name: 'locked.pdf', availability: 'encrypted' }),
        pdf({ name: 'photo.jpg', format: 'JPEG', kind: 'image' }),
      ],
    }),
    'third-id': itemDetails({ identifier: 'third-id', title: 'Third', pdfs: [pdf()] }),
  }

  it('selects every matching file of a record from its row, without opening it', async () => {
    iaRef.iaGetItem.mockImplementation(async (id: string) => recordFiles[id])
    await searchPage([{}])

    await fireEvent.click(screen.getByRole('checkbox', { name: 'Seleccionar registro Some Title' }))

    await screen.findByText('Seleccionados: 2')
    expect(iaRef.iaGetItem).toHaveBeenCalledWith('some-id')
    expect(screen.getByRole('checkbox', { name: 'Seleccionar registro Some Title' })).toBeChecked()

    await fireEvent.click(screen.getByRole('checkbox', { name: 'Seleccionar registro Some Title' }))
    expect(screen.queryByText('Seleccionados: 2')).toBeNull()
    expect(
      screen.getByRole('checkbox', { name: 'Seleccionar registro Some Title' })
    ).not.toBeChecked()
  })

  it('selects and deselects the whole page from the header, skipping restricted records', async () => {
    iaRef.iaGetItem.mockImplementation(async (id: string) => recordFiles[id])
    await searchPage([
      {},
      { identifier: 'other-id', title: 'Locked', restricted: true },
      { identifier: 'third-id', title: 'Third' },
    ])
    expect(screen.getByRole('checkbox', { name: 'Seleccionar registro Locked' })).toBeDisabled()
    const all = screen.getByRole('checkbox', {
      name: 'Seleccionar todos los registros de la página',
    })

    await fireEvent.click(all)

    await screen.findByText('Seleccionados: 3')
    expect(iaRef.iaGetItem).not.toHaveBeenCalledWith('other-id')
    expect(all).toBeChecked()

    await fireEvent.click(screen.getByRole('checkbox', { name: 'Seleccionar registro Third' }))
    expect(all).toBePartiallyChecked()

    await fireEvent.click(all)
    await screen.findByText('Seleccionados: 3')
    await fireEvent.click(all)
    expect(screen.queryByText(/Seleccionados:/)).toBeNull()
    expect(all).not.toBeChecked()
  })

  it('fetches record files a few at a time', async () => {
    const pending: Array<() => void> = []
    iaRef.iaGetItem.mockImplementation(
      (id: string) =>
        new Promise((resolve) => {
          pending.push(() => resolve(itemDetails({ identifier: id, pdfs: [pdf()] })))
        })
    )
    await searchPage(
      Array.from({ length: 10 }, (_, i) => ({ identifier: `id-${i}`, title: `Record ${i}` }))
    )

    await fireEvent.click(
      screen.getByRole('checkbox', { name: 'Seleccionar todos los registros de la página' })
    )

    expect(iaRef.iaGetItem).toHaveBeenCalledTimes(4)
    while (pending.length > 0) {
      pending.shift()!()
      await waitFor(() => undefined)
    }
    await screen.findByText('Seleccionados: 10')
    expect(iaRef.iaGetItem).toHaveBeenCalledTimes(10)
  })

  function pagedSearch(total: number) {
    iaRef.iaSearch.mockImplementation(async (_query: unknown, page: number, rows: number) => ({
      numFound: total,
      items: Array.from(
        { length: Math.max(0, Math.min(rows, total - (page - 1) * rows)) },
        (_, i) =>
          searchItem({
            identifier: `id-${(page - 1) * rows + i}`,
            title: `R${(page - 1) * rows + i}`,
          })
      ),
    }))
    iaRef.iaGetItem.mockImplementation(async (id: string) =>
      itemDetails({ identifier: id, pdfs: [pdf()] })
    )
  }

  async function searchAndSelectPage() {
    open()
    await fireEvent.input(screen.getByLabelText('Título'), { target: { value: 'Chaplin' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar en el archivo' }))
    await screen.findAllByRole('button', { name: 'Ver archivos' })
    await fireEvent.click(
      screen.getByRole('checkbox', { name: 'Seleccionar todos los registros de la página' })
    )
    await screen.findByText('Seleccionados: 50')
  }

  it('offers every result once the page is selected, and selects them all', async () => {
    pagedSearch(120)
    await searchAndSelectPage()

    expect(
      screen.getByText('Se van a revisar los archivos de 120 registros; puede tardar.')
    ).toBeInTheDocument()
    await fireEvent.click(screen.getByRole('button', { name: 'Seleccionar los 120 resultados' }))

    await screen.findByText('Seleccionados: 120', undefined, { timeout: 3000 })
    expect(iaRef.iaSearch).toHaveBeenCalledWith(
      expect.objectContaining({ title: 'Chaplin', kinds: ['pdf'] }),
      2,
      100
    )
    expect(iaRef.iaGetItem).toHaveBeenCalledTimes(120)
  })

  it('caps the all-results selection and says how many it takes', async () => {
    pagedSearch(2000)
    await searchAndSelectPage()

    expect(
      screen.getByRole('button', { name: 'Seleccionar los 500 resultados' })
    ).toBeInTheDocument()
    expect(
      screen.getByText(/Se seleccionan los primeros 500 de 2.?000 resultados\./)
    ).toBeInTheDocument()
  })

  it('does not offer every result while the page is not fully selected', async () => {
    pagedSearch(120)
    open()
    await fireEvent.input(screen.getByLabelText('Título'), { target: { value: 'Chaplin' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar en el archivo' }))
    await screen.findAllByRole('button', { name: 'Ver archivos' })
    expect(screen.queryByRole('button', { name: /Seleccionar los \d+ resultados/ })).toBeNull()
  })

  it('opens in the wide layout', () => {
    open()

    expect(screen.getByRole('dialog')).toHaveClass('confirm-dialog--wide')
  })

  it('changes the page size and searches again from the first page', async () => {
    iaRef.iaSearch.mockResolvedValue({ numFound: 125, items: [searchItem()] })
    open()
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar en el archivo' }))
    await screen.findByText('Página 1 de 3')
    const sizes = screen.getByRole('group', { name: 'Resultados por página' })
    expect(within(sizes).getByRole('button', { name: '50' })).toHaveAttribute(
      'aria-pressed',
      'true'
    )

    await fireEvent.click(within(sizes).getByRole('button', { name: '100' }))
    await screen.findByText('Página 1 de 2')

    expect(iaRef.iaSearch).toHaveBeenLastCalledWith(expect.anything(), 1, 100)
    expect(within(sizes).getByRole('button', { name: '100' })).toHaveAttribute(
      'aria-pressed',
      'true'
    )
  })

  it('never offers a page past the 10 000-result paging limit', async () => {
    iaRef.iaSearch.mockResolvedValue({ numFound: 4_151_546, items: [searchItem()] })
    open()
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar en el archivo' }))

    // 10 000 / 50 rows = 200 reachable pages, however many hits exist.
    expect(await screen.findByText('Página 1 de 200')).toBeInTheDocument()
  })

  it('keeps the results visible beside the item files', async () => {
    await openItem()

    const results = screen.getByRole('region', { name: 'Resultados' })
    expect(within(results).getByText('Some Title')).toBeInTheDocument()
    expect(screen.getAllByRole('button', { name: 'Ver archivos' })).toHaveLength(2)
    expect(screen.getByRole('checkbox', { name: 'Seleccionar doc_text.pdf' })).toBeInTheDocument()
  })

  it('explores a collection by id through the Colección tab', async () => {
    iaRef.iaListCollection.mockResolvedValue({ numFound: 1, items: [searchItem()] })
    open()
    await fireEvent.click(within(discoveryTabs()).getByRole('button', { name: 'Colección' }))
    await fireEvent.input(screen.getByLabelText('Identificador de la colección'), {
      target: { value: 'opensource' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Explorar' }))

    expect(iaRef.iaListCollection).toHaveBeenCalledWith('opensource', 1, 50, ['pdf'])
    expect(await screen.findByText('Some Title')).toBeInTheDocument()
  })

  it('resolves a link and opens the item through the Enlace tab', async () => {
    iaRef.iaResolveIdentifier.mockResolvedValue('some-id')
    iaRef.iaGetItem.mockResolvedValue(itemDetails())
    open()
    await fireEvent.click(within(discoveryTabs()).getByRole('button', { name: 'Enlace' }))
    await fireEvent.input(screen.getByLabelText('Identificador o enlace'), {
      target: { value: 'https://archive.org/details/some-id' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Abrir' }))
    expect(await screen.findByText('Archivos')).toBeInTheDocument()

    expect(iaRef.iaResolveIdentifier).toHaveBeenCalledWith('https://archive.org/details/some-id')
    expect(iaRef.iaGetItem).toHaveBeenCalledWith('some-id')
  })

  it('shows translated messages instead of raw error codes', async () => {
    iaRef.iaSearch.mockRejectedValue(new Error('network: connection reset'))
    open()
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar en el archivo' }))

    expect(await screen.findByText('Error de red al contactar Internet Archive.'))
    expect(screen.queryByText(/network: connection reset/)).toBeNull()
  })

  it('renders nothing when closed through the open prop', () => {
    open({ open: false })

    expect(screen.queryByRole('dialog')).toBeNull()
  })
})

describe('InternetArchiveDialog item and selection', () => {
  it('disables non-available files and shows their reasons', async () => {
    await searchForResults()
    iaRef.iaGetItem.mockResolvedValue(
      itemDetails({
        pdfs: [
          pdf(),
          pdf({ name: 'locked.pdf', availability: 'encrypted', size: null }),
          pdf({ name: 'secret.pdf', availability: 'private', size: 2048 }),
        ],
      })
    )
    await fireEvent.click(screen.getAllByRole('button', { name: 'Ver archivos' })[0]!)
    await screen.findByText('Archivos')

    expect(screen.getByRole('checkbox', { name: 'Seleccionar doc_text.pdf' })).toBeEnabled()
    expect(screen.getByRole('checkbox', { name: 'Seleccionar locked.pdf' })).toBeDisabled()
    expect(screen.getByRole('checkbox', { name: 'Seleccionar secret.pdf' })).toBeDisabled()
    expect(screen.getByText(/Cifrado/)).toBeInTheDocument()
    expect(screen.getByText(/Privado/)).toBeInTheDocument()
    expect(screen.getByText('Tamaño: desconocido')).toBeInTheDocument()
    expect(screen.getByText('Tamaño: 1 KB')).toBeInTheDocument()
  })

  it('says sin PDF when the record offers no PDF', async () => {
    await searchForResults()
    iaRef.iaGetItem.mockResolvedValue(itemDetails({ pdfs: [] }))
    await fireEvent.click(screen.getAllByRole('button', { name: 'Ver archivos' })[0]!)

    expect(await screen.findByText(/Sin PDF/)).toBeInTheDocument()
  })

  it('offers to browse a collection record in the Colección tab instead', async () => {
    await searchForResults()
    iaRef.iaGetItem.mockResolvedValue(itemDetails({ mediatype: 'collection', pdfs: [] }))
    await fireEvent.click(screen.getAllByRole('button', { name: 'Ver archivos' })[0]!)
    await screen.findByText(/es una colección/)

    iaRef.iaListCollection.mockResolvedValue({ numFound: 0, items: [] })
    await fireEvent.click(screen.getByRole('button', { name: 'Explorar en la pestaña Colección' }))
    await screen.findByText('Esta colección no tiene resultados en esta página.')

    expect(screen.getByLabelText('Identificador de la colección')).toHaveValue('some-id')
    expect(iaRef.iaListCollection).toHaveBeenCalledWith('some-id', 1, 50, ['pdf'])
  })

  it('accumulates the selection across items and lets files be removed', async () => {
    iaRef.iaSearch.mockResolvedValue({ numFound: 1, items: [searchItem()] })
    iaRef.iaGetItem.mockResolvedValue(itemDetails())
    open()
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar en el archivo' }))
    await fireEvent.click(await screen.findByRole('button', { name: 'Ver archivos' }))
    await screen.findByText('Archivos')
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Seleccionar doc_text.pdf' }))
    await screen.findByText('Seleccionados: 1')

    await fireEvent.click(screen.getByRole('button', { name: 'Ver selección' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Quitar doc_text.pdf' }))

    expect(screen.queryByText('Seleccionados: 1')).toBeNull()
    expect(screen.getByText(/no elegiste ningún archivo/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Continuar' })).toBeDisabled()
  })
})

describe('InternetArchiveDialog destination and preparation', () => {
  it('prepares with { identifier, fileNames } selections and the chosen collection', async () => {
    await prepareIntoCollection('col-1')

    expect(iaRef.iaPrepareImport).toHaveBeenCalledWith('col-1', [
      { identifier: 'some-id', fileNames: ['doc_text.pdf'] },
    ])
  })

  it('preselects the destination when opened from a collection', async () => {
    await goToDestination({ initialCollectionId: 'col-2' })

    expect(screen.getByRole('radio', { name: /Obras/ })).toBeChecked()
    expect(screen.getByRole('button', { name: 'Preparar importación' })).toBeEnabled()
  })

  it('creates a new collection with the store before preparing', async () => {
    await prepareIntoCollection('new')

    expect(storeRef.current.collections.create).toHaveBeenCalledWith({
      name: 'Archivo nuevo',
      description: null,
    })
    expect(iaRef.iaPrepareImport).toHaveBeenCalledWith(
      'col-new',
      expect.arrayContaining([
        expect.objectContaining({ identifier: 'some-id', fileNames: ['doc_text.pdf'] }),
      ])
    )
  })

  it('needs a name before preparing into a new collection', async () => {
    await goToDestination()
    await fireEvent.click(screen.getByRole('radio', { name: /Nueva colección/ }))

    expect(screen.getByRole('button', { name: 'Preparar importación' })).toBeDisabled()
  })

  it('counts a PDF twice and an image once in the synced volume', async () => {
    await goToDestination()
    iaRef.iaPrepareImport.mockResolvedValue(
      prepareResult({
        summary: {
          totalFiles: 2,
          availableFiles: 2,
          restrictedFiles: 0,
          knownBytes: 2 * 1024 * 1024,
          unknownSizeFiles: 0,
          newFiles: 2,
          alreadyInTarget: 0,
          inOtherCollection: 0,
          newVersion: 0,
          entries: [
            prepareEntry({ fileName: 'doc.pdf', size: 1024 * 1024 }),
            prepareEntry({ fileName: 'plate.png', size: 1024 * 1024, kind: 'image' }),
          ],
        },
      })
    )
    await fireEvent.click(screen.getByRole('radio', { name: /Voces/ }))
    await fireEvent.click(screen.getByRole('button', { name: 'Preparar importación' }))
    await screen.findByText('Revisa la preparación')

    expect(screen.getByText('Volumen sincronizado estimado: 3 MB')).toBeInTheDocument()
  })

  it('asks to tick files, not only PDFs, while nothing is selected', async () => {
    open()
    expect(
      await screen.findByText(
        'Todavía no elegiste ningún archivo. Marca los archivos que quieras importar.'
      )
    ).toBeInTheDocument()
  })

  it('shows duplicate statuses, rejected files and the synced-volume warning', async () => {
    await goToDestination()
    iaRef.iaPrepareImport.mockResolvedValue(
      prepareResult({
        summary: {
          totalFiles: 3,
          availableFiles: 3,
          restrictedFiles: 0,
          knownBytes: 1_200_000_000,
          unknownSizeFiles: 1,
          newFiles: 1,
          alreadyInTarget: 1,
          inOtherCollection: 1,
          newVersion: 0,
          entries: [
            prepareEntry({ fileName: 'new.pdf', size: 600_000_000 }),
            prepareEntry({
              fileName: 'old.pdf',
              size: 600_000_000,
              duplicate: { status: 'already_in_target', itemId: 'doc-1' },
            }),
            prepareEntry({
              fileName: 'elsewhere.pdf',
              size: null,
              duplicate: {
                status: 'in_other_collection',
                itemId: 'doc-2',
                collectionId: 'col-9',
              },
            }),
          ],
        },
        rejected: [{ identifier: 'some-id', fileName: 'locked.pdf', reason: 'not_found' }],
      })
    )
    await fireEvent.click(screen.getByRole('radio', { name: /Voces/ }))
    await fireEvent.click(screen.getByRole('button', { name: 'Preparar importación' }))
    await screen.findByText('Revisa la preparación')

    expect(screen.getByText('Ya importado en esta colección.')).toBeInTheDocument()
    expect(screen.getByText('Existe en otra colección.')).toBeInTheDocument()
    expect(screen.getByText(/1 archivo sin tamaño conocido/)).toBeInTheDocument()
    expect(screen.getByText('locked.pdf')).toBeInTheDocument()
    expect(screen.getByText(/supera 1 GB/)).toBeInTheDocument()
    // The duplicate set to omit by default is not counted as to-import.
    expect(screen.getByText('Archivos a importar: 1')).toBeInTheDocument()
  })

  it('importing an independent copy counts it back as to-import', async () => {
    await goToDestination()
    iaRef.iaPrepareImport.mockResolvedValue(
      prepareResult({
        summary: {
          totalFiles: 2,
          availableFiles: 2,
          restrictedFiles: 0,
          knownBytes: 2048,
          unknownSizeFiles: 0,
          newFiles: 1,
          alreadyInTarget: 0,
          inOtherCollection: 0,
          newVersion: 1,
          entries: [
            prepareEntry({ fileName: 'doc_text.pdf' }),
            prepareEntry({
              fileName: 'v2.pdf',
              duplicate: { status: 'new_version', previousItemId: 'doc-0' },
            }),
          ],
        },
        rejected: [],
      })
    )
    await fireEvent.click(screen.getByRole('radio', { name: /Voces/ }))
    await fireEvent.click(screen.getByRole('button', { name: 'Preparar importación' }))
    await screen.findByText('Hay una versión nueva.')

    expect(screen.getByText('Archivos a importar: 1')).toBeInTheDocument()
    await fireEvent.click(screen.getByRole('radio', { name: 'Importar una copia independiente' }))

    expect(screen.getByText('Archivos a importar: 2')).toBeInTheDocument()
  })
})

describe('InternetArchiveDialog running and done', () => {
  // The backend always answers the CURRENT status, no matter how many times
  // the dialog refreshes: a mutable answer keeps the tests independent of
  // the order in which internal refreshes and job events interleave.
  let currentStatus: Record<string, unknown> = jobStatus()

  async function startJob(status: Record<string, unknown> = jobStatus()) {
    currentStatus = status
    iaRef.iaGetImportStatus.mockImplementation(async () => currentStatus)
    await prepareIntoCollection('col-1')
    iaRef.iaStartImport.mockResolvedValue(undefined)
    await fireEvent.click(screen.getByRole('button', { name: 'Iniciar importación' }))
    await screen.findByText('Importando…')
  }

  it('starts the prepared job, refreshes status on job-updated and imports verified files', async () => {
    await startJob()

    expect(iaRef.iaStartImport).toHaveBeenCalledWith('job-1')
    expect(iaRef.onIaJobUpdated).toHaveBeenCalled()
    expect(iaRef.iaGetImportStatus).toHaveBeenCalledWith('job-1')

    jobHandler?.('job-1')
    await waitFor(() =>
      expect(importRef.importVerifiedFiles).toHaveBeenCalledWith(
        'job-1',
        expect.objectContaining({ shouldCancel: expect.any(Function) })
      )
    )
    expect(screen.getByText('0 de 1 archivos')).toBeInTheDocument()
    expect(screen.getByText('Verificado')).toBeInTheDocument()
    // Starting never discards: the job is live, not an abandoned draft.
    expect(iaRef.iaDiscardDraft).not.toHaveBeenCalled()
  })

  it('runs the import again when new files become verified', async () => {
    await startJob()
    jobHandler?.('job-1')
    await waitFor(() => expect(importRef.importVerifiedFiles).toHaveBeenCalledTimes(1))

    currentStatus = jobStatus({
      state: 'running',
      files: [
        jobFile({ index: 0, state: 'imported' }),
        jobFile({ index: 1, fileName: 'other.pdf', state: 'verified' }),
      ],
    })
    jobHandler?.('job-1')
    await waitFor(() => expect(importRef.importVerifiedFiles).toHaveBeenCalledTimes(2))
  })

  it('ignores job-updated events for other jobs', async () => {
    await startJob()
    const calls = importRef.importVerifiedFiles.mock.calls.length

    jobHandler?.('job-other')

    await Promise.resolve()
    expect(importRef.importVerifiedFiles.mock.calls.length).toBe(calls)
  })

  it('cancels through iaCancelImport and reports verified file codes translated', async () => {
    await startJob()

    const cancelButtons = screen.getAllByRole('button', { name: 'Cancelar' })
    const active = cancelButtons.find((button) => (button as HTMLButtonElement).disabled === false)!
    await fireEvent.click(active)

    expect(iaRef.iaCancelImport).toHaveBeenCalledWith('job-1')
    currentStatus = jobStatus({
      state: 'cancelled',
      files: [jobFile({ state: 'cancelled', error: 'cancelled' })],
    })
    jobHandler?.('job-1')
    expect(await screen.findByText('Importación terminada')).toBeInTheDocument()
    expect(screen.getByText('Cancelado.')).toBeInTheDocument()
    expect(screen.queryByText(/cancelled/)).toBeNull()
  })

  it('shows imported, present and failed counts with translated reasons, then opens the collection', async () => {
    const changed = vi.fn()
    window.addEventListener(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, changed)
    importRef.importVerifiedFiles.mockResolvedValue(
      importOutcome({
        imported: [{ index: 0, itemId: 'doc-1' }],
        confirmedExisting: [{ index: 2, itemId: 'doc-2' }],
        failed: [{ index: 1, code: 'verify_failed' }],
      })
    )
    await startJob()
    currentStatus = jobStatus({
      state: 'partial',
      files: [
        jobFile({ index: 0, state: 'imported' }),
        jobFile({ index: 1, fileName: 'bad.pdf', state: 'failed', error: 'verify_failed' }),
      ],
    })
    jobHandler?.('job-1')

    expect(await screen.findByText('Importación terminada')).toBeInTheDocument()
    expect(screen.getByText('Importados: 1')).toBeInTheDocument()
    expect(screen.getByText('Ya estaban en la colección: 1')).toBeInTheDocument()
    expect(screen.getByText('Con error: 1')).toBeInTheDocument()
    expect(screen.getByText('Verificación fallida.')).toBeInTheDocument()
    expect(screen.queryByText(/verify_failed/)).toBeNull()
    expect(changed).toHaveBeenCalled()

    await fireEvent.click(screen.getByRole('button', { name: 'Abrir colección' }))
    expect(onImported).toHaveBeenCalledWith('col-1')
    expect(onClose).toHaveBeenCalled()
    // Leaving from done never discards: the job already ran.
    expect(iaRef.iaDiscardDraft).not.toHaveBeenCalled()
    window.removeEventListener(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, changed)
  })

  it('re-prepares without omitted duplicates before starting', async () => {
    iaRef.iaSearch.mockResolvedValue({ numFound: 1, items: [searchItem()] })
    iaRef.iaGetItem.mockResolvedValue(
      itemDetails({ pdfs: [pdf(), pdf({ name: 'elsewhere.pdf' })] })
    )
    open()
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar en el archivo' }))
    await fireEvent.click(await screen.findByRole('button', { name: 'Ver archivos' }))
    await screen.findByText('Archivos')
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Seleccionar doc_text.pdf' }))
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Seleccionar elsewhere.pdf' }))
    await screen.findByText('Seleccionados: 2')
    await fireEvent.click(screen.getByRole('button', { name: 'Continuar' }))
    await screen.findByText('Destino')
    iaRef.iaPrepareImport
      .mockResolvedValueOnce(
        prepareResult({
          summary: {
            totalFiles: 2,
            availableFiles: 2,
            restrictedFiles: 0,
            knownBytes: 2048,
            unknownSizeFiles: 0,
            newFiles: 1,
            alreadyInTarget: 0,
            inOtherCollection: 1,
            newVersion: 0,
            entries: [
              prepareEntry({ fileName: 'doc_text.pdf' }),
              prepareEntry({
                fileName: 'elsewhere.pdf',
                duplicate: {
                  status: 'in_other_collection',
                  itemId: 'doc-2',
                  collectionId: 'col-9',
                },
              }),
            ],
          },
          rejected: [],
        })
      )
      .mockResolvedValueOnce(
        prepareResult({
          jobId: 'job-2',
          summary: {
            totalFiles: 1,
            availableFiles: 1,
            restrictedFiles: 0,
            knownBytes: 1024,
            unknownSizeFiles: 0,
            newFiles: 1,
            alreadyInTarget: 0,
            inOtherCollection: 0,
            newVersion: 0,
            entries: [prepareEntry({ fileName: 'doc_text.pdf' })],
          },
          rejected: [],
        })
      )
    await fireEvent.click(screen.getByRole('radio', { name: /Voces/ }))
    await fireEvent.click(screen.getByRole('button', { name: 'Preparar importación' }))
    await screen.findByText('Existe en otra colección.')

    iaRef.iaStartImport.mockResolvedValue(undefined)
    iaRef.iaGetImportStatus.mockResolvedValue(jobStatus({ id: 'job-2' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Iniciar importación' }))
    await screen.findByText('Importando…')

    expect(iaRef.iaPrepareImport).toHaveBeenCalledTimes(2)
    expect(iaRef.iaPrepareImport).toHaveBeenLastCalledWith('col-1', [
      { identifier: 'some-id', fileNames: ['doc_text.pdf'] },
    ])
    expect(iaRef.iaStartImport).toHaveBeenCalledWith('job-2')
    expect(iaRef.iaDiscardDraft).toHaveBeenCalledWith('job-1')
  })

  it('ignores a failed discard of the abandoned draft and still starts', async () => {
    iaRef.iaSearch.mockResolvedValue({ numFound: 1, items: [searchItem()] })
    iaRef.iaGetItem.mockResolvedValue(
      itemDetails({ pdfs: [pdf(), pdf({ name: 'elsewhere.pdf' })] })
    )
    iaRef.iaPrepareImport
      .mockResolvedValueOnce(
        prepareResult({
          summary: {
            totalFiles: 2,
            availableFiles: 2,
            restrictedFiles: 0,
            knownBytes: 2048,
            unknownSizeFiles: 0,
            newFiles: 1,
            alreadyInTarget: 0,
            inOtherCollection: 1,
            newVersion: 0,
            entries: [
              prepareEntry({ fileName: 'doc_text.pdf' }),
              prepareEntry({
                fileName: 'elsewhere.pdf',
                duplicate: {
                  status: 'in_other_collection',
                  itemId: 'doc-2',
                  collectionId: 'col-9',
                },
              }),
            ],
          },
          rejected: [],
        })
      )
      .mockResolvedValueOnce(prepareResult({ jobId: 'job-2' }))
    iaRef.iaDiscardDraft.mockRejectedValueOnce(new Error('io: gone'))
    open()
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar en el archivo' }))
    await fireEvent.click(await screen.findByRole('button', { name: 'Ver archivos' }))
    await screen.findByText('Archivos')
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Seleccionar doc_text.pdf' }))
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Seleccionar elsewhere.pdf' }))
    await screen.findByText('Seleccionados: 2')
    await fireEvent.click(screen.getByRole('button', { name: 'Continuar' }))
    await screen.findByText('Destino')
    await fireEvent.click(screen.getByRole('radio', { name: /Voces/ }))
    await fireEvent.click(screen.getByRole('button', { name: 'Preparar importación' }))
    await screen.findByText('Existe en otra colección.')
    iaRef.iaStartImport.mockResolvedValue(undefined)
    iaRef.iaGetImportStatus.mockResolvedValue(jobStatus({ id: 'job-2' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Iniciar importación' }))
    await screen.findByText('Importando…')

    expect(iaRef.iaStartImport).toHaveBeenCalledWith('job-2')
    expect(iaRef.iaDiscardDraft).toHaveBeenCalledWith('job-1')
  })

  it('discards the draft when closing from the summary without starting', async () => {
    await prepareIntoCollection('col-1')
    expect(iaRef.iaDiscardDraft).not.toHaveBeenCalled()

    await fireEvent.click(screen.getByRole('button', { name: 'Cancelar' }))

    expect(iaRef.iaDiscardDraft).toHaveBeenCalledWith('job-1')
    expect(onClose).toHaveBeenCalled()
  })

  it('unsubscribes the job listener on close', async () => {
    await prepareIntoCollection('col-1')

    await fireEvent.click(screen.getByRole('button', { name: 'Cancelar' }))

    expect(unlisten).toHaveBeenCalled()
    expect(onClose).toHaveBeenCalled()
  })
})

describe('InternetArchiveDialog resume (26b)', () => {
  const resumableJob = (patch: Record<string, unknown> = {}) => ({
    id: 'job-9',
    state: 'running',
    targetCollectionId: 'col-1',
    totalFiles: 2,
    verifiedFiles: 1,
    ...patch,
  })

  it('lists a started job with a resume action at the top of the discover phase', async () => {
    iaRef.iaListJobs.mockResolvedValue([resumableJob()])
    open()

    expect(await screen.findByText('Importaciones en curso')).toBeInTheDocument()
    expect(screen.getByText(/En curso/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Retomar importación' })).toBeInTheDocument()
  })

  it('does not offer draft or terminal jobs', async () => {
    iaRef.iaListJobs.mockResolvedValue([
      resumableJob({ id: 'job-draft', state: 'draft' }),
      resumableJob({ id: 'job-done', state: 'completed' }),
    ])
    open()

    await waitFor(() => expect(iaRef.iaListJobs).toHaveBeenCalled())
    await screen.findByRole('group', { name: 'Descubrimiento' })
    expect(screen.queryByText('Importaciones en curso')).toBeNull()
    expect(screen.queryByRole('button', { name: 'Retomar importación' })).toBeNull()
  })

  it('hides the notice and still discovers when listing jobs fails', async () => {
    iaRef.iaListJobs.mockRejectedValue(new Error('io: gone'))
    iaRef.iaSearch.mockResolvedValue({ numFound: 0, items: [] })
    open()

    await fireEvent.click(screen.getByRole('button', { name: 'Buscar en el archivo' }))
    expect(await screen.findByText('Sin resultados para esta búsqueda.')).toBeInTheDocument()
    expect(screen.queryByText('Importaciones en curso')).toBeNull()
  })

  it('resumes into the running phase reading everything from the backend status', async () => {
    iaRef.iaListJobs.mockResolvedValue([resumableJob()])
    iaRef.iaGetImportStatus.mockResolvedValue(jobStatus({ id: 'job-9', state: 'running' }))
    open()

    await fireEvent.click(await screen.findByRole('button', { name: 'Retomar importación' }))

    expect(await screen.findByText('Importando…')).toBeInTheDocument()
    expect(iaRef.iaGetImportStatus).toHaveBeenCalledWith('job-9')
    expect(importRef.importVerifiedFiles).toHaveBeenCalledWith(
      'job-9',
      expect.objectContaining({ shouldCancel: expect.any(Function) })
    )
    expect(iaRef.iaPrepareImport).not.toHaveBeenCalled()
  })

  it('does not restart a job whose runner is still active', async () => {
    iaRef.iaListJobs.mockResolvedValue([resumableJob()])
    iaRef.iaGetImportStatus.mockResolvedValue(jobStatus({ id: 'job-9', running: true }))
    open()

    await fireEvent.click(await screen.findByRole('button', { name: 'Retomar importación' }))

    expect(await screen.findByText('Importando…')).toBeInTheDocument()
    expect(iaRef.iaStartImport).not.toHaveBeenCalled()
  })

  it.each(['ready', 'running'])(
    'restarts the downloads of a %s job left without a runner',
    async (state) => {
      iaRef.iaListJobs.mockResolvedValue([resumableJob({ state })])
      iaRef.iaGetImportStatus.mockResolvedValue(jobStatus({ id: 'job-9', state, running: false }))
      open()

      await fireEvent.click(await screen.findByRole('button', { name: 'Retomar importación' }))

      await waitFor(() => expect(iaRef.iaStartImport).toHaveBeenCalledWith('job-9'))
      expect(await screen.findByText('Importando…')).toBeInTheDocument()
    }
  )
})

describe('InternetArchiveDialog stale responses (28)', () => {
  it('shows the second item when the first iaGetItem resolves last', async () => {
    iaRef.iaSearch.mockResolvedValue({
      numFound: 2,
      items: [
        searchItem(),
        searchItem({
          identifier: 'other-id',
          title: 'Other Title',
          creators: [],
          date: null,
          restricted: false,
        }),
      ],
    })
    open()
    await fireEvent.click(screen.getByRole('button', { name: 'Buscar en el archivo' }))
    await screen.findByText('Some Title')
    let resolveFirst!: (value: unknown) => void
    let resolveSecond!: (value: unknown) => void
    const firstGate = new Promise((resolve) => {
      resolveFirst = resolve
    })
    const secondGate = new Promise((resolve) => {
      resolveSecond = resolve
    })
    iaRef.iaGetItem.mockImplementation((identifier: string) => {
      if (identifier === 'some-id') return firstGate as Promise<never>
      return secondGate as Promise<never>
    })
    const buttons = screen.getAllByRole('button', { name: 'Ver archivos' })
    await fireEvent.click(buttons[0]!)
    // The first item is still loading: open the second one from the list
    // that stays beside it, so both `iaGetItem` calls overlap with the first
    // resolving last.
    await fireEvent.click(buttons[1]!)
    const detail = screen.getByRole('region', { name: 'Registro seleccionado' })
    resolveSecond(itemDetails({ identifier: 'other-id', title: 'Other Title' }))
    await within(detail).findByText('Other Title')
    resolveFirst(itemDetails({ identifier: 'some-id', title: 'Some Title' }))
    // Let the stale first response settle.
    await waitFor(() => expect(iaRef.iaGetItem).toHaveBeenCalledTimes(2))
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(within(detail).getByText('Other Title')).toBeInTheDocument()
    expect(within(detail).queryByText('some-id')).toBeNull()
  })

  it('does not let an older status overwrite a newer one', async () => {
    const currentStatus: Record<string, unknown> = jobStatus()
    iaRef.iaGetImportStatus.mockImplementation(async () => currentStatus)
    await prepareIntoCollection('col-1')
    iaRef.iaStartImport.mockResolvedValue(undefined)
    await fireEvent.click(screen.getByRole('button', { name: 'Iniciar importación' }))
    await screen.findByText('Importando…')
    // Two overlapping refreshes: the first is slow with first.pdf, the
    // second is fast with second.pdf. Resolving the slow one last must not
    // overwrite the newer status.
    let resolveSlow!: (value: unknown) => void
    let resolveFast!: (value: unknown) => void
    const slowGate = new Promise((resolve) => {
      resolveSlow = resolve
    })
    const fastGate = new Promise((resolve) => {
      resolveFast = resolve
    })
    const slowStatus = jobStatus({
      state: 'running',
      files: [jobFile({ displayFileName: 'first.pdf', fileName: 'first.pdf' })],
    })
    const fastStatus = jobStatus({
      state: 'running',
      files: [jobFile({ displayFileName: 'second.pdf', fileName: 'second.pdf' })],
    })
    iaRef.iaGetImportStatus
      .mockImplementationOnce(() => slowGate as Promise<never>)
      .mockImplementationOnce(() => fastGate as Promise<never>)
      .mockImplementation(async () => fastStatus)
    jobHandler?.('job-1')
    jobHandler?.('job-1')
    resolveFast(fastStatus)
    await screen.findByText('second.pdf')
    resolveSlow(slowStatus)
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(screen.getByText('second.pdf')).toBeInTheDocument()
    expect(screen.queryByText('first.pdf')).toBeNull()
  })
})

describe('InternetArchiveDialog in English', () => {
  it('renders the tabs, phases and duplicate statuses in English', async () => {
    locale.set('en')
    iaRef.iaSearch.mockResolvedValue({ numFound: 0, items: [] })
    open()

    expect(screen.getByRole('group', { name: 'Discovery' })).toBeInTheDocument()
    await fireEvent.click(screen.getByRole('button', { name: 'Search the archive' }))
    expect(await screen.findByText('No results for this search.')).toBeInTheDocument()
  })
})
