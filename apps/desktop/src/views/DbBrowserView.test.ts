/** @vitest-environment happy-dom */

import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'
import type { DbBrowserQueryResponse } from '$lib/db-browser'
import DbBrowserView from './DbBrowserView.svelte'
import dbBrowserViewSource from './DbBrowserView.svelte?raw'
import searchClearButtonSource from '../../../../packages/ui/src/components/SearchClearButton/SearchClearButton.svelte?raw'

const source = `${dbBrowserViewSource}\n${searchClearButtonSource}`

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
  const MockButton = (await import('./__mocks__/MockButton.svelte')).default
  const MockActionIcon = (await import('./__mocks__/MockActionIcon.svelte')).default
  return { Button: MockButton, ActionIcon: MockActionIcon }
})

vi.mock('./DbBrowserView.svelte', async () => {
  // Vitest hoists module mock factories, so this test double must be loaded inside the factory.
  const MockDbBrowserView = (await import('./__mocks__/MockDbBrowserView.svelte')).default
  return { default: MockDbBrowserView }
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
      total: 1,
      rows: [{ id: 'row-1', body: jsonCellValue }],
    })
    queryAllRowsMock.mockReset().mockResolvedValue({
      table: 'documents',
      page: 1,
      pageSize: 1000,
      total: 1,
      rows: [{ id: 'row-1', body: jsonCellValue }],
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
    expect([...source.matchAll(/\biconOnly\b/g)]).toHaveLength(2)
    expect(source).toContain('SearchClearButton')
    expect(source).toContain("label={translate('dbBrowser.searchClear')}")
    expect(source).toContain('<ActionIcon name="close" size={14} />')
    expect(source).toContain('<ActionIcon name="search" size={16} />')
    expect(source).toContain('<ActionIcon name="rotate-cw" size={16} />')
    expect(source).not.toContain('<ActionIcon name="broom"')

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
