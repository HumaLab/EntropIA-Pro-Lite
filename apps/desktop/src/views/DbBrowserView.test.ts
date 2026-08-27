/** @vitest-environment happy-dom */

import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'
import DbBrowserView from './DbBrowserView.svelte'
import dbBrowserViewSource from './DbBrowserView.svelte?raw'

const {
  listTablesMock,
  describeTableMock,
  queryAllRowsMock,
  queryRowsMock,
  clipboardWriteTextMock,
  exportCollectionToJsonMock,
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
}))

vi.mock('@entropia/ui', async () => {
  const MockButton = (await import('./__mocks__/MockButton.svelte')).default
  return { Button: MockButton }
})

vi.mock('./DbBrowserView.svelte', async () => {
  const MockDbBrowserView = (await import('./__mocks__/MockDbBrowserView.svelte')).default
  return { default: MockDbBrowserView }
})

function flushPromises() {
  return new Promise((resolve) => setTimeout(resolve, 0))
}

describe('DbBrowserView', () => {
  beforeEach(() => {
    locale.set('es')

    listTablesMock.mockReset().mockResolvedValue([{ name: 'documents' }])
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
  })

  it('defines icon-only search, clear, and refresh actions with localized tooltips', () => {
    expect([...dbBrowserViewSource.matchAll(/\biconOnly\b/g)]).toHaveLength(3)
    expect(dbBrowserViewSource).toContain('<ActionIcon name="search" size={16} />')
    expect(dbBrowserViewSource).toContain('<ActionIcon name="broom" size={16} />')
    expect(dbBrowserViewSource).toContain('<ActionIcon name="rotate-cw" size={16} />')

    for (const key of [
      'dbBrowser.searchSubmit',
      'dbBrowser.searchClear',
      'dbBrowser.refresh',
    ]) {
      expect(dbBrowserViewSource).toContain(
        `aria-label={$currentLocale && translate('${key}')}`
      )
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
    })

    await flushPromises()
    await waitFor(() => {
      expect(screen.getByRole('button', { name: 'Descargar JSON' })).toBeInTheDocument()
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

  it('shows the empty table message without hiding the export action', async () => {
    await renderDbBrowserView()

    expect(screen.getByText('Esta tabla no tiene filas para mostrar.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Descargar JSON' })).toBeInTheDocument()
  })

  it('renders a visible table export button and exports the full table query', async () => {
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

    await fireEvent.click(screen.getByRole('button', { name: 'Descargar JSON' }))

    await waitFor(() => {
      expect(exportCollectionToJsonMock).toHaveBeenCalledTimes(1)
    })
    expect(queryAllRowsMock).toHaveBeenCalledWith({
      table: 'documents',
      sortColumn: '',
      sortDirection: 'asc',
      search: undefined,
    })
    expect(queryRowsMock).toHaveBeenCalledTimes(1)
    const [payload] = exportCollectionToJsonMock.mock.calls[0] ?? []
    expect(payload).toMatchObject({
      table: 'documents',
      scope: 'full_table',
      rows: [
        { id: 'row-1', body: jsonCellValue },
        { id: 'row-2', body: 'plain text' },
      ],
    })
  })
})
