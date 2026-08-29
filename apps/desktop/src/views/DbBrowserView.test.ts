/** @vitest-environment jsdom */


import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'
import type { DbBrowserQueryResponse } from '$lib/db-browser'
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
} = vi.hoisted(() => {
  const jsonCellValue = '{"title":"Acta","meta":{"page":2}}'

  return {
    listTablesMock: vi.fn(),
    describeTableMock: vi.fn(),
    queryAllRowsMock: vi.fn(),
    queryRowsMock: vi.fn(),
    clipboardWriteTextMock: vi.fn<(_: string) => Promise<void>>(),
    exportCollectionToJsonMock: vi.fn(),
    exportCollectionToCsvMock: vi.fn(),
    jsonCellValue,
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

    Object.defineProperty(globalThis.navigator, 'clipboard', {
      configurable: true,
      value: { writeText: clipboardWriteTextMock },
    })
    clipboardWriteTextMock.mockReset().mockResolvedValue(undefined)
    exportCollectionToJsonMock.mockReset().mockResolvedValue('documents.json')
    exportCollectionToCsvMock.mockReset().mockResolvedValue('documents.csv')
  })

  it('defines icon-only submit and refresh actions plus the shared clear control', () => {
    expect([...dbBrowserViewSource.matchAll(/\biconOnly\b/g)]).toHaveLength(2)
    expect(dbBrowserViewSource).toContain('SearchClearButton')
    expect(dbBrowserViewSource).toContain(
      "label={$currentLocale && translate('dbBrowser.searchClear')}"
    )
    expect(dbBrowserViewSource).toContain('<ActionIcon name="search" size={16} />')
    expect(dbBrowserViewSource).toContain('<ActionIcon name="rotate-cw" size={16} />')
    expect(dbBrowserViewSource).not.toContain('<ActionIcon name="broom"')
    expect(dbBrowserViewSource).toContain(
      '.db-browser-toolbar__input-wrap {\n    position: relative;\n    width: 100%;\n  }'
    )
    expect(dbBrowserViewSource).toContain(
      '#db-browser-search {\n    width: 100%;\n    padding-right: calc(var(--space-3) + 24px + var(--space-2));\n  }'
    )
    expect(dbBrowserViewSource).toContain('padding: 0 var(--space-3);')

    for (const key of ['dbBrowser.searchSubmit', 'dbBrowser.refresh']) {
      expect(dbBrowserViewSource).toContain(`aria-label={$currentLocale && translate('${key}')}`)
      expect(dbBrowserViewSource).toContain(`title={$currentLocale && translate('${key}')}`)
    }
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
    expect(closeButton).toHaveAttribute('title', 'Cerrar')
    expect(closeButton).toHaveClass('db-browser-table__cell-action', 'db-browser-modal__icon-action')
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

    expect(jsonButton).toHaveAttribute('title', 'Exportar JSON')
    expect(csvButton).toHaveAttribute('title', 'Exportar CSV')
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
})
