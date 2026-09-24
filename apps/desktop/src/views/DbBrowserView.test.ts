/** @vitest-environment jsdom */

import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'
import type { DbBrowserQueryResponse } from '$lib/db-browser'
import {
  DOCUMENT_ASSET_DELETED_EVENT,
  DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT,
} from '$lib/document-explorer'
import DbBrowserView from './DbBrowserView.svelte'
import dbBrowserViewSource from './DbBrowserView.svelte?raw'

const {
  listTablesMock,
  describeTableMock,
  queryAllRowsMock,
  queryRowsMock,
  clipboardWriteTextMock,
  exportCollectionToJsonMock,
  exportCollectionToCsvMock,
  jsonCellValue,
  batchStoreMock,
  syncStoreMock,
} = vi.hoisted(() => {
  const jsonCellValue = '{"title":"Acta","meta":{"page":2}}'

  // Minimal fakes of the two module-level stores: `subscribe` captures the
  // callback and immediately pushes an initial snapshot (mirroring the real
  // stores), and `emit` lets a test drive a later change from outside.
  let batchSubscriber: ((summary: unknown) => void) | null = null
  let syncSubscriber: ((status: unknown) => void) | null = null

  return {
    listTablesMock: vi.fn(),
    describeTableMock: vi.fn(),
    queryAllRowsMock: vi.fn(),
    queryRowsMock: vi.fn(),
    clipboardWriteTextMock: vi.fn<(_: string) => Promise<void>>(),
    exportCollectionToJsonMock: vi.fn(),
    exportCollectionToCsvMock: vi.fn(),
    jsonCellValue,
    batchStoreMock: {
      initialize: vi.fn().mockResolvedValue(undefined),
      subscribe: vi.fn((run: (summary: unknown) => void) => {
        batchSubscriber = run
        run({ init: null, initError: null, active: [], recoveredBatches: 0 })
        return () => {
          batchSubscriber = null
        }
      }),
      emit: (summary: unknown) => batchSubscriber?.(summary),
    },
    syncStoreMock: {
      initialize: vi.fn().mockResolvedValue(undefined),
      subscribe: vi.fn((run: (status: unknown) => void) => {
        syncSubscriber = run
        run({ state: 'disabled', last_sync_at: null })
        return () => {
          syncSubscriber = null
        }
      }),
      emit: (status: unknown) => syncSubscriber?.(status),
    },
  }
})

vi.mock('$lib/db-browser', () => ({
  listDbBrowserTables: listTablesMock,
  describeDbBrowserTable: describeTableMock,
  queryAllDbBrowserRowsInChunks: queryAllRowsMock,
  queryDbBrowserRows: queryRowsMock,
}))

vi.mock('$lib/export', () => ({
  exportCollectionToJson: exportCollectionToJsonMock,
  exportCollectionToCsv: exportCollectionToCsvMock,
}))

vi.mock('$lib/batch-processing', () => ({
  batchStore: batchStoreMock,
}))

vi.mock('$lib/sync-store', () => ({
  syncStore: syncStoreMock,
}))

vi.mock('@entropia/ui', async () => {
  const actual = await vi.importActual<typeof import('@entropia/ui')>('@entropia/ui')
  const MockButton = (await import('./__mocks__/MockButton.svelte')).default
  const MockActionIcon = (await import('./__mocks__/MockActionIcon.svelte')).default

  return {
    ...actual,
    Button: MockButton,
    ActionIcon: MockActionIcon,
  }
})

function flushPromises() {
  return new Promise((resolve) => setTimeout(resolve, 0))
}

function createDeferred<T>() {
  let resolve!: (value: T) => void
  // Promise.withResolvers is unavailable under this project's TypeScript library target.
  const promise = new Promise<T>((next) => {
    resolve = next
  })
  return { promise, resolve }
}

afterEach(() => {
  cleanup()
})
describe('DbBrowserView fixed header on scroll', () => {
  // Only the rows scroll: the page header, the table toolbar and the column
  // names stay in view. The view fills the content area exactly, the card
  // takes what is left, and the table wrapper scrolls itself, which is what
  // makes the (already sticky) column headers stick.
  function rule(selector: string): string {
    const start = dbBrowserViewSource.indexOf(`${selector} {`)
    expect(start, `${selector} rule is missing`).toBeGreaterThan(-1)
    return dbBrowserViewSource.slice(start, dbBrowserViewSource.indexOf('}', start))
  }

  it('makes the view fill the content area instead of growing with the rows', () => {
    expect(rule('.db-browser-view')).toMatch(/(?<!min-)height:\s*100%;/)
  })

  it('lets the table card take the remaining height', () => {
    const card = rule('.db-browser-card')
    expect(card).toMatch(/flex:\s*1;/)
    expect(card).toMatch(/min-height:\s*0;/)
  })

  it('scrolls the rows inside the table wrapper, under sticky column names', () => {
    const wrap = rule('.db-browser-table-wrap')
    expect(wrap).toMatch(/flex:\s*1;/)
    expect(wrap).toMatch(/overflow:\s*auto;/)
    expect(rule('.db-browser-table thead th')).toMatch(/position:\s*sticky;/)
  })
})

describe('DbBrowserView', () => {
  beforeEach(() => {
    locale.set('es')

    listTablesMock.mockReset().mockResolvedValue([{ name: 'documents' }, { name: 'archives' }])
    describeTableMock.mockReset().mockResolvedValue([
      {
        name: 'body',
        dataType: 'TEXT',
        nullable: true,
        isPrimaryKey: false,
      },
    ])
    queryRowsMock.mockReset().mockResolvedValue({
      table: 'documents',
      page: 1,
      pageSize: 25,
      total: 0,
      rows: [],
    })
    queryAllRowsMock.mockReset().mockResolvedValue({
      table: 'documents',
      page: 1,
      pageSize: 1000,
      total: 0,
      rows: [],
    })

    batchStoreMock.initialize.mockClear().mockResolvedValue(undefined)
    batchStoreMock.subscribe.mockClear()
    syncStoreMock.initialize.mockClear().mockResolvedValue(undefined)
    syncStoreMock.subscribe.mockClear()

    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: { writeText: clipboardWriteTextMock },
    })
    clipboardWriteTextMock.mockReset().mockResolvedValue(undefined)
    exportCollectionToJsonMock.mockReset().mockResolvedValue('documents.json')
    exportCollectionToCsvMock.mockReset().mockResolvedValue('documents.csv')
  })

  it('opens on the extractions table when it is available', async () => {
    listTablesMock.mockResolvedValue([
      { name: 'assets' },
      { name: 'extractions' },
      { name: 'items' },
    ])
    render(DbBrowserView)

    await waitFor(() => expect(describeTableMock).toHaveBeenCalledWith('extractions'))
    expect(describeTableMock).not.toHaveBeenCalledWith('assets')
  })

  it('renders no search submit or refresh buttons', async () => {
    await renderDbBrowserView()

    expect(screen.queryByRole('button', { name: 'Buscar' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Recargar' })).not.toBeInTheDocument()
    // The filter field itself, and its clear control, still exist.
    expect(screen.getByRole('searchbox', { name: 'Filtro simple' })).toBeInTheDocument()
  })

  it('drops the removed toolbar actions and their i18n keys from the source', () => {
    expect(dbBrowserViewSource).not.toContain('db-browser-toolbar__actions')
    expect(dbBrowserViewSource).not.toMatch(/dbBrowser\.searchSubmit\b/)
    expect(dbBrowserViewSource).not.toMatch(/dbBrowser\.refresh\b/)
    expect(dbBrowserViewSource).toContain('SearchClearButton')
    expect(dbBrowserViewSource).toContain(
      "label={$currentLocale && translate('dbBrowser.searchClear')}"
    )
    expect(dbBrowserViewSource).toContain(
      '.db-browser-toolbar__input-wrap {\n    position: relative;\n    width: 100%;\n  }'
    )
    expect(dbBrowserViewSource).toContain(
      '#db-browser-search {\n    width: 100%;\n    padding-right: calc(var(--space-3) + 24px + var(--space-2));\n  }'
    )
    // The leading inset is the app-wide one, so the gap between the magnifier
    // and the text matches every other search field.
    expect(dbBrowserViewSource).toContain('padding: 0 var(--space-3) 0 var(--search-field-inset);')
    expect(dbBrowserViewSource).toContain('<span class="search-field__icon" aria-hidden="true">')
  })

  async function renderDbBrowserView() {
    render(DbBrowserView)

    await flushPromises()
    await flushPromises()

    await waitFor(() => {
      expect(listTablesMock).toHaveBeenCalledTimes(1)
      expect(describeTableMock).toHaveBeenCalledWith('documents')
      expect(queryRowsMock).toHaveBeenCalledTimes(1)
      expect(screen.getByRole('button', { name: 'Exportar JSON' })).toBeInTheDocument()
      expect(screen.getByRole('button', { name: 'Exportar CSV' })).toBeInTheDocument()
    })
  }

  async function renderPagedView(total: number) {
    queryRowsMock.mockReset().mockResolvedValue({
      table: 'documents',
      page: 1,
      pageSize: 25,
      total,
      rows: [{ body: 'Acta' }],
    })
    await renderDbBrowserView()
    return screen.getByRole('group', { name: 'Paginación de la tabla' })
  }

  it('navigates the table with an icon-only pagination group', async () => {
    // 130 rows over a page size of 25 -> 6 pages.
    const group = await renderPagedView(130)

    expect(within(group).getByRole('button', { name: 'Primera' })).toBeDisabled()
    expect(within(group).getByRole('button', { name: 'Anterior' })).toBeDisabled()
    expect(within(group).getByRole('button', { name: 'Siguiente' })).toBeEnabled()
    expect(within(group).getByRole('button', { name: 'Última' })).toBeEnabled()

    await fireEvent.click(within(group).getByRole('button', { name: 'Siguiente' }))
    await waitFor(() => {
      expect(screen.getByText('Página 2 de 6')).toBeInTheDocument()
    })
    expect(within(group).getByRole('button', { name: 'Anterior' })).toBeEnabled()
  })

  it('jumps to the last page in one click instead of five', async () => {
    const group = await renderPagedView(130)

    await fireEvent.click(within(group).getByRole('button', { name: 'Última' }))

    await waitFor(() => {
      expect(screen.getByText('Página 6 de 6')).toBeInTheDocument()
    })
    expect(within(group).getByRole('button', { name: 'Siguiente' })).toBeDisabled()
    expect(within(group).getByRole('button', { name: 'Última' })).toBeDisabled()
    expect(within(group).getByRole('button', { name: 'Primera' })).toBeEnabled()
  })

  it('hides the pagination group when there is only one page', async () => {
    queryRowsMock.mockReset().mockResolvedValue({
      table: 'documents',
      page: 1,
      pageSize: 25,
      total: 3,
      rows: [{ body: 'Acta' }],
    })
    await renderDbBrowserView()

    expect(screen.queryByRole('group', { name: 'Paginación de la tabla' })).not.toBeInTheDocument()
  })

  it('keeps the page labels as accessible names rather than visible copy', async () => {
    const group = await renderPagedView(130)

    // The words survive for screen readers and tooltips; they just stop taking
    // up 200px of a dense toolbar.
    expect(within(group).getByRole('button', { name: 'Anterior' })).toHaveAttribute(
      'data-tooltip',
      'Anterior'
    )
    expect(within(group).queryByText('Anterior')).not.toBeInTheDocument()
    expect(within(group).queryByText('Siguiente')).not.toBeInTheDocument()
  })

  it('clears the DB search with the real shared control and reloads the current table state', async () => {
    await renderDbBrowserView()

    const searchInput = screen.getByRole('searchbox', { name: 'Filtro simple' })
    await fireEvent.input(searchInput, { target: { value: 'acta' } })

    const clearButton = screen.getByRole('button', { name: 'Limpiar búsqueda' })
    await fireEvent.click(clearButton)

    await waitFor(() => {
      expect(searchInput).toHaveValue('')
      expect(queryRowsMock).toHaveBeenLastCalledWith({
        table: 'documents',
        page: 1,
        pageSize: 25,
        sortColumn: 'body',
        sortDirection: 'asc',
        search: undefined,
      })
    })
  })

  it('renders the database browser header and selected table metadata', async () => {
    await renderDbBrowserView()

    expect(screen.getByText('Base de datos')).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'Consulta DB' })).toBeInTheDocument()
    expect(screen.getByText('documents · 1 columnas')).toBeInTheDocument()
  })

  it('renders the selected table control after loading tables', async () => {
    await renderDbBrowserView()

    expect(screen.getByLabelText('Tabla')).toHaveValue('documents')
  })

  it('describes expanded embedding cells as Base64 with the row dimensions', async () => {
    describeTableMock.mockResolvedValue([
      { name: 'asset_id', dataType: 'TEXT', nullable: false, isPrimaryKey: true },
      { name: 'embedding', dataType: 'BLOB', nullable: false, isPrimaryKey: false },
      { name: 'dimensions', dataType: 'INTEGER', nullable: false, isPrimaryKey: false },
    ])
    queryRowsMock.mockResolvedValue({
      table: 'documents',
      page: 1,
      pageSize: 25,
      total: 2,
      rows: [
        { asset_id: 'asset-1', embedding: 'QUJDREVG'.repeat(30), dimensions: 1024 },
        { asset_id: 'asset-2', embedding: 'WFlaQUJD'.repeat(30), dimensions: 0 },
      ],
    })

    await renderDbBrowserView()

    const expandButtons = screen.getAllByRole('button', { name: 'Expandir valor de embedding' })
    expect(expandButtons).toHaveLength(2)

    await fireEvent.click(expandButtons[0]!)
    expect(
      screen.getByText('Representación binaria codificada en Base64 · 1024 dimensiones')
    ).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Cerrar' }))

    await fireEvent.click(
      screen.getAllByRole('button', { name: 'Expandir valor de embedding' })[1]!
    )
    expect(screen.getByText('Representación binaria codificada en Base64')).toBeInTheDocument()
    expect(screen.queryByText('Vista completa del contenido textual.')).not.toBeInTheDocument()
  })

  it('shows a BLOB as its size and copies the full Base64 value', async () => {
    const payload = 'QUJDREVG'.repeat(30)
    describeTableMock.mockResolvedValue([
      { name: 'id', dataType: 'TEXT', nullable: false, isPrimaryKey: true },
      { name: 'embedding', dataType: 'BLOB', nullable: false, isPrimaryKey: false },
    ])
    queryRowsMock.mockResolvedValue({
      table: 'documents',
      page: 1,
      pageSize: 25,
      total: 1,
      rows: [{ id: 'chunk-1', embedding: payload }],
    })

    await renderDbBrowserView()

    expect(screen.getByText('BLOB · 180 bytes')).toBeInTheDocument()
    expect(screen.queryByText(payload)).not.toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Copiar valor de embedding' }))
    await waitFor(() => {
      expect(clipboardWriteTextMock).toHaveBeenCalledWith(payload)
    })
  })

  it('renders the modal close action as an X icon button matching the copy action', async () => {
    queryRowsMock.mockResolvedValue({
      table: 'documents',
      page: 1,
      pageSize: 25,
      total: 1,
      rows: [{ body: 'Texto largo '.repeat(20).trim() }],
    })

    await renderDbBrowserView()
    await fireEvent.click(screen.getByRole('button', { name: 'Expandir valor de body' }))

    const copyButton = screen.getByRole('button', { name: 'Copiar valor completo de body' })
    const closeButton = screen.getByRole('button', { name: 'Cerrar' })

    expect(closeButton.textContent?.trim()).toBe('')
    expect(closeButton).toHaveAttribute('data-tooltip', 'Cerrar')
    expect(closeButton).toHaveClass(
      'db-browser-table__cell-action',
      'db-browser-modal__icon-action'
    )
    expect(copyButton).toHaveClass('db-browser-table__cell-action', 'db-browser-modal__icon-action')

    await fireEvent.click(closeButton)
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('keeps both export actions visible when the table is empty', async () => {
    queryRowsMock.mockResolvedValue({
      table: 'documents',
      page: 1,
      pageSize: 25,
      total: 0,
      rows: [],
    })

    await renderDbBrowserView()

    expect(screen.getByText('Esta tabla no tiene filas para mostrar.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Exportar JSON' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Exportar CSV' })).toBeInTheDocument()
  })

  it('renders equal export buttons with only icon and format text', async () => {
    await renderDbBrowserView()

    const jsonButton = screen.getByRole('button', { name: 'Exportar JSON' })
    const csvButton = screen.getByRole('button', { name: 'Exportar CSV' })

    expect(jsonButton).toHaveAttribute('data-tooltip', 'Exportar JSON')
    expect(csvButton).toHaveAttribute('data-tooltip', 'Exportar CSV')
    expect(jsonButton.textContent?.trim()).toBe('JSON')
    expect(csvButton.textContent?.trim()).toBe('CSV')
    expect(jsonButton.querySelector('svg')).not.toBeNull()
    expect(csvButton.querySelector('svg')).not.toBeNull()
    expect(jsonButton).toHaveClass('db-browser-export-button')
    expect(csvButton).toHaveClass('db-browser-export-button')
    expect([
      ...dbBrowserViewSource.matchAll(/<ActionIcon name="download" size=\{16\} \/>/g),
    ]).toHaveLength(2)
    expect(dbBrowserViewSource).toContain('grid-template-columns: repeat(2, minmax(0, 1fr));')
  })

  it('exports the full filtered and sorted table as JSON', async () => {
    queryAllRowsMock.mockResolvedValue({
      table: 'documents',
      page: 1,
      pageSize: 1000,
      total: 2,
      rows: [
        { id: 'row-1', body: jsonCellValue },
        { id: 'row-2', body: 'plain text' },
      ],
    })

    await renderDbBrowserView()
    await fireEvent.click(screen.getByRole('button', { name: 'Exportar JSON' }))

    await waitFor(() => {
      expect(exportCollectionToJsonMock).toHaveBeenCalledTimes(1)
    })
    expect(queryAllRowsMock).toHaveBeenCalledWith({
      table: 'documents',
      sortColumn: 'body',
      sortDirection: 'asc',
      search: undefined,
    })
    const [payload, defaultName] = exportCollectionToJsonMock.mock.calls[0] ?? []
    expect(payload).toMatchObject({
      table: 'documents',
      scope: 'full_table',
      rows: [
        { id: 'row-1', body: jsonCellValue },
        { id: 'row-2', body: 'plain text' },
      ],
    })
    expect(defaultName).toBe('documents.json')
    expect(exportCollectionToCsvMock).not.toHaveBeenCalled()
  })

  it('exports schema-ordered rows as CSV', async () => {
    const exportRows = [
      { id: 'row-1', body: jsonCellValue },
      { id: 'row-2', body: 'plain text' },
    ]
    queryAllRowsMock.mockResolvedValue({
      table: 'documents',
      page: 1,
      pageSize: 1000,
      total: exportRows.length,
      rows: exportRows,
    })

    await renderDbBrowserView()
    await fireEvent.click(screen.getByRole('button', { name: 'Exportar CSV' }))

    await waitFor(() => {
      expect(exportCollectionToCsvMock).toHaveBeenCalledTimes(1)
    })
    expect(queryAllRowsMock).toHaveBeenCalledWith({
      table: 'documents',
      sortColumn: 'body',
      sortDirection: 'asc',
      search: undefined,
    })
    expect(exportCollectionToCsvMock).toHaveBeenCalledWith(exportRows, ['body'], 'documents.csv')
    expect(exportCollectionToJsonMock).not.toHaveBeenCalled()
  })

  it('snapshots the table name and schema before collecting export rows', async () => {
    const pendingRows = createDeferred<DbBrowserQueryResponse>()
    queryAllRowsMock.mockReturnValue(pendingRows.promise)

    await renderDbBrowserView()
    await fireEvent.click(screen.getByRole('button', { name: 'Exportar CSV' }))
    await waitFor(() => {
      expect(queryAllRowsMock).toHaveBeenCalledTimes(1)
    })

    await fireEvent.change(screen.getByLabelText('Tabla'), { target: { value: 'archives' } })
    expect(screen.getByLabelText('Tabla')).toHaveValue('archives')
    await flushPromises()
    expect(screen.getByLabelText('Tabla')).toHaveValue('archives')
    pendingRows.resolve({
      table: 'documents',
      page: 1,
      pageSize: 1000,
      total: 1,
      rows: [{ body: 'Acta' }],
    })

    await waitFor(() => {
      expect(exportCollectionToCsvMock).toHaveBeenCalledWith(
        [{ body: 'Acta' }],
        ['body'],
        'documents.csv'
      )
    })
    const snapshotIndex = dbBrowserViewSource.indexOf('const exportTable = selectedTable')
    const queryIndex = dbBrowserViewSource.indexOf('await queryAllDbBrowserRowsInChunks')
    expect(snapshotIndex).toBeGreaterThan(-1)
    expect(snapshotIndex).toBeLessThan(queryIndex)
  })

  it('disables both export actions while either format is being prepared', async () => {
    // Keep the export pending; this project's TS target does not expose Promise.withResolvers.
    queryAllRowsMock.mockReturnValue(new Promise(() => {}))

    await renderDbBrowserView()

    const jsonButton = screen.getByRole('button', { name: 'Exportar JSON' })
    const csvButton = screen.getByRole('button', { name: 'Exportar CSV' })
    await fireEvent.click(jsonButton)

    await waitFor(() => {
      expect(jsonButton).toBeDisabled()
      expect(csvButton).toBeDisabled()
    })
  })

  describe('filter debounce and automatic reload', () => {
    beforeEach(() => {
      vi.useFakeTimers()
    })

    afterEach(() => {
      vi.useRealTimers()
    })

    /** Same wait as `renderDbBrowserView`, without `flushPromises` (a real
     *  `setTimeout(0)`, which never fires under fake timers). The mocked
     *  loads below resolve as microtasks, so `waitFor` alone is enough. */
    async function renderAndWaitForInitialLoad() {
      render(DbBrowserView)
      await waitFor(() => {
        expect(listTablesMock).toHaveBeenCalledTimes(1)
        expect(describeTableMock).toHaveBeenCalledWith('documents')
        expect(queryRowsMock).toHaveBeenCalledTimes(1)
      })
    }

    it('filters rows after the debounce and resets to page 1', async () => {
      await renderAndWaitForInitialLoad()
      queryRowsMock.mockClear()

      const input = screen.getByRole('searchbox', { name: 'Filtro simple' })
      await fireEvent.input(input, { target: { value: 'acta' } })

      expect(queryRowsMock).not.toHaveBeenCalled()
      await vi.advanceTimersByTimeAsync(299)
      expect(queryRowsMock).not.toHaveBeenCalled()

      await vi.advanceTimersByTimeAsync(1)
      await waitFor(() => {
        expect(queryRowsMock).toHaveBeenCalledWith({
          table: 'documents',
          page: 1,
          pageSize: 25,
          sortColumn: 'body',
          sortDirection: 'asc',
          search: 'acta',
        })
      })
    })

    it('coalesces fast keystrokes into a single query for the last value typed', async () => {
      await renderAndWaitForInitialLoad()
      queryRowsMock.mockClear()

      const input = screen.getByRole('searchbox', { name: 'Filtro simple' })
      await fireEvent.input(input, { target: { value: 'a' } })
      await vi.advanceTimersByTimeAsync(100)
      await fireEvent.input(input, { target: { value: 'ac' } })
      await vi.advanceTimersByTimeAsync(100)
      await fireEvent.input(input, { target: { value: 'acta' } })

      await vi.advanceTimersByTimeAsync(300)

      expect(queryRowsMock).toHaveBeenCalledTimes(1)
      expect(queryRowsMock).toHaveBeenCalledWith(expect.objectContaining({ search: 'acta' }))
    })

    it('applies the filter immediately on Enter, cancelling the pending debounce', async () => {
      await renderAndWaitForInitialLoad()
      queryRowsMock.mockClear()

      const input = screen.getByRole('searchbox', { name: 'Filtro simple' })
      const form = input.closest('form')
      if (!form) throw new Error('expected the filter input to sit inside a form')

      await fireEvent.input(input, { target: { value: 'acta' } })
      await fireEvent.submit(form)

      expect(queryRowsMock).toHaveBeenCalledTimes(1)
      expect(queryRowsMock).toHaveBeenCalledWith({
        table: 'documents',
        page: 1,
        pageSize: 25,
        sortColumn: 'body',
        sortDirection: 'asc',
        search: 'acta',
      })

      // The debounce that would have re-applied the same value was cancelled.
      await vi.advanceTimersByTimeAsync(300)
      expect(queryRowsMock).toHaveBeenCalledTimes(1)
    })

    it('ignores a stale filter response when a newer one already resolved', async () => {
      await renderAndWaitForInitialLoad()

      const firstQuery = createDeferred<DbBrowserQueryResponse>()
      const secondQuery = createDeferred<DbBrowserQueryResponse>()
      queryRowsMock
        .mockReset()
        .mockReturnValueOnce(firstQuery.promise)
        .mockReturnValueOnce(secondQuery.promise)

      const input = screen.getByRole('searchbox', { name: 'Filtro simple' })
      await fireEvent.input(input, { target: { value: 'acta' } })
      await vi.advanceTimersByTimeAsync(300)
      expect(queryRowsMock).toHaveBeenCalledTimes(1)

      await fireEvent.input(input, { target: { value: 'vigente' } })
      await vi.advanceTimersByTimeAsync(300)
      expect(queryRowsMock).toHaveBeenCalledTimes(2)

      secondQuery.resolve({
        table: 'documents',
        page: 1,
        pageSize: 25,
        total: 1,
        rows: [{ body: 'Acta vigente' }],
      })
      await waitFor(() => {
        expect(screen.getByText('Acta vigente')).toBeInTheDocument()
      })

      // The stale first response lands after the newer one already rendered.
      firstQuery.resolve({
        table: 'documents',
        page: 1,
        pageSize: 25,
        total: 1,
        rows: [{ body: 'Acta vieja' }],
      })
      await vi.advanceTimersByTimeAsync(0)

      expect(screen.getByText('Acta vigente')).toBeInTheDocument()
      expect(screen.queryByText('Acta vieja')).not.toBeInTheDocument()
    })

    it('does not reload merely from mounting (the stores’ initial snapshot push)', async () => {
      await renderAndWaitForInitialLoad()
      listTablesMock.mockClear()

      await vi.advanceTimersByTimeAsync(2000)

      expect(listTablesMock).not.toHaveBeenCalled()
    })

    it('coalesces a burst of batch-processing signals into one reload', async () => {
      await renderAndWaitForInitialLoad()
      listTablesMock.mockClear()

      for (let i = 0; i < 5; i++) {
        batchStoreMock.emit({ active: [{ id: `batch-${i}` }] })
        await vi.advanceTimersByTimeAsync(100)
      }
      expect(listTablesMock).not.toHaveBeenCalled()

      await vi.advanceTimersByTimeAsync(600)
      expect(listTablesMock).toHaveBeenCalledTimes(1)
    })

    it('coalesces a burst of sync-completed signals into one reload', async () => {
      await renderAndWaitForInitialLoad()
      listTablesMock.mockClear()

      for (let i = 1; i <= 5; i++) {
        syncStoreMock.emit({ state: 'idle', last_sync_at: i })
        await vi.advanceTimersByTimeAsync(100)
      }
      expect(listTablesMock).not.toHaveBeenCalled()

      await vi.advanceTimersByTimeAsync(600)
      expect(listTablesMock).toHaveBeenCalledTimes(1)
    })

    it('does not reload on a sync status tick that does not change last_sync_at', async () => {
      await renderAndWaitForInitialLoad()
      listTablesMock.mockClear()

      syncStoreMock.emit({ state: 'syncing', last_sync_at: null })
      syncStoreMock.emit({ state: 'idle', last_sync_at: null })
      await vi.advanceTimersByTimeAsync(1000)

      expect(listTablesMock).not.toHaveBeenCalled()
    })

    it('coalesces a burst of document import/change events into one reload', async () => {
      await renderAndWaitForInitialLoad()
      listTablesMock.mockClear()

      for (let i = 0; i < 5; i++) {
        window.dispatchEvent(
          new CustomEvent(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, {
            detail: { collectionId: 'col-1' },
          })
        )
        await vi.advanceTimersByTimeAsync(100)
      }
      expect(listTablesMock).not.toHaveBeenCalled()

      await vi.advanceTimersByTimeAsync(600)
      expect(listTablesMock).toHaveBeenCalledTimes(1)
    })

    it('coalesces a burst of page-deletion events into one reload', async () => {
      await renderAndWaitForInitialLoad()
      listTablesMock.mockClear()

      for (let i = 0; i < 5; i++) {
        window.dispatchEvent(
          new CustomEvent(DOCUMENT_ASSET_DELETED_EVENT, {
            detail: { itemId: 'item-1', assetId: `asset-${i}` },
          })
        )
        await vi.advanceTimersByTimeAsync(100)
      }
      expect(listTablesMock).not.toHaveBeenCalled()

      await vi.advanceTimersByTimeAsync(600)
      expect(listTablesMock).toHaveBeenCalledTimes(1)
    })

    it('keeps the page, sort, filter and selected table across an automatic reload', async () => {
      queryRowsMock.mockReset().mockResolvedValue({
        table: 'documents',
        page: 1,
        pageSize: 25,
        total: 130,
        rows: [{ body: 'Acta' }],
      })
      await renderAndWaitForInitialLoad()

      await fireEvent.change(screen.getByLabelText('Tabla'), { target: { value: 'archives' } })
      await waitFor(() => expect(describeTableMock).toHaveBeenLastCalledWith('archives'))

      // Sort descending on the only column.
      await fireEvent.click(screen.getByRole('button', { name: 'body' }))
      await waitFor(() => expect(screen.getByText('Página 1 de 6')).toBeInTheDocument())

      // Filter (applied immediately via Enter).
      const input = screen.getByRole('searchbox', { name: 'Filtro simple' })
      const form = input.closest('form')
      if (!form) throw new Error('expected the filter input to sit inside a form')
      await fireEvent.input(input, { target: { value: 'acta' } })
      await fireEvent.submit(form)

      // Page forward.
      const group = screen.getByRole('group', { name: 'Paginación de la tabla' })
      await fireEvent.click(within(group).getByRole('button', { name: 'Siguiente' }))
      await waitFor(() => expect(screen.getByText('Página 2 de 6')).toBeInTheDocument())

      queryRowsMock.mockClear()
      listTablesMock.mockClear()

      window.dispatchEvent(
        new CustomEvent(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, {
          detail: { collectionId: 'col-1' },
        })
      )
      await vi.advanceTimersByTimeAsync(600)

      await waitFor(() => {
        expect(listTablesMock).toHaveBeenCalledTimes(1)
        expect(describeTableMock).toHaveBeenLastCalledWith('archives')
      })
      expect(queryRowsMock).toHaveBeenLastCalledWith({
        table: 'archives',
        page: 2,
        pageSize: 25,
        sortColumn: 'body',
        sortDirection: 'desc',
        search: 'acta',
      })
      expect(screen.getByLabelText('Tabla')).toHaveValue('archives')
    })

    it('an automatic reload keeps the grid on screen instead of swapping to a loading page', async () => {
      await renderAndWaitForInitialLoad()

      const pendingDescribe =
        createDeferred<
          Array<{ name: string; dataType: string; nullable: boolean; isPrimaryKey: boolean }>
        >()
      describeTableMock.mockReturnValueOnce(pendingDescribe.promise)

      window.dispatchEvent(
        new CustomEvent(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, {
          detail: { collectionId: 'col-1' },
        })
      )
      await vi.advanceTimersByTimeAsync(600)

      // The schema re-read is in flight, but the table select (proof the
      // grid, not a loading message, is still on screen) is still there.
      expect(screen.getByLabelText('Tabla')).toBeInTheDocument()
      expect(screen.queryByText('Cargando tablas disponibles...')).not.toBeInTheDocument()

      pendingDescribe.resolve([
        { name: 'body', dataType: 'TEXT', nullable: true, isPrimaryKey: false },
      ])
      await vi.advanceTimersByTimeAsync(0)
    })

    it('an automatic reload re-reads the schema, lists new tables and keeps the selected one', async () => {
      await renderAndWaitForInitialLoad()

      await fireEvent.change(screen.getByLabelText('Tabla'), { target: { value: 'archives' } })
      await waitFor(() => {
        expect(describeTableMock).toHaveBeenLastCalledWith('archives')
      })

      listTablesMock.mockResolvedValue([
        { name: 'archives' },
        { name: 'documents' },
        { name: 'added_by_migration' },
      ])
      window.dispatchEvent(
        new CustomEvent(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, {
          detail: { collectionId: 'col-1' },
        })
      )
      await vi.advanceTimersByTimeAsync(600)

      await waitFor(() => {
        expect(listTablesMock).toHaveBeenCalledTimes(2)
        expect(screen.getByRole('option', { name: 'added_by_migration' })).toBeInTheDocument()
      })
      expect(describeTableMock).toHaveBeenLastCalledWith('archives')
      expect(screen.getByLabelText('Tabla')).toHaveValue('archives')
    })

    it('falls back to the first table when an automatic reload no longer finds the selected one', async () => {
      await renderAndWaitForInitialLoad()

      listTablesMock.mockResolvedValue([{ name: 'archives' }])
      window.dispatchEvent(
        new CustomEvent(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, {
          detail: { collectionId: 'col-1' },
        })
      )
      await vi.advanceTimersByTimeAsync(600)

      await waitFor(() => {
        expect(screen.getByLabelText('Tabla')).toHaveValue('archives')
      })
      expect(describeTableMock).toHaveBeenLastCalledWith('archives')
    })

    it('dispose (unmount) stops the store subscriptions, listeners and pending timer', async () => {
      const { unmount } = render(DbBrowserView)
      await waitFor(() => expect(listTablesMock).toHaveBeenCalledTimes(1))

      unmount()
      listTablesMock.mockClear()

      batchStoreMock.emit({ active: [{ id: 'batch-1' }] })
      syncStoreMock.emit({ state: 'idle', last_sync_at: 999 })
      window.dispatchEvent(
        new CustomEvent(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, {
          detail: { collectionId: 'col-1' },
        })
      )
      window.dispatchEvent(
        new CustomEvent(DOCUMENT_ASSET_DELETED_EVENT, {
          detail: { itemId: 'item-1', assetId: 'asset-1' },
        })
      )

      await vi.advanceTimersByTimeAsync(2000)

      expect(listTablesMock).not.toHaveBeenCalled()
    })
  })
})
