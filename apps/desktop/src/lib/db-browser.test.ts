import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { DB_BROWSER_EXPORT_PAGE_SIZE, queryAllDbBrowserRowsInChunks } from './db-browser'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))

const mockInvoke = vi.mocked(invoke)

describe('queryAllDbBrowserRowsInChunks', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('requests export rows in fixed-size chunks instead of all rows at once', async () => {
    mockInvoke
      .mockResolvedValueOnce({
        table: 'documents',
        page: 1,
        pageSize: DB_BROWSER_EXPORT_PAGE_SIZE,
        total: DB_BROWSER_EXPORT_PAGE_SIZE + 1,
        rows: Array.from({ length: DB_BROWSER_EXPORT_PAGE_SIZE }, (_, index) => ({
          id: index + 1,
        })),
      })
      .mockResolvedValueOnce({
        table: 'documents',
        page: 2,
        pageSize: DB_BROWSER_EXPORT_PAGE_SIZE,
        total: DB_BROWSER_EXPORT_PAGE_SIZE + 1,
        rows: [{ id: DB_BROWSER_EXPORT_PAGE_SIZE + 1 }],
      })

    const response = await queryAllDbBrowserRowsInChunks({
      table: 'documents',
      sortColumn: 'id',
      sortDirection: 'asc',
      search: undefined,
    })

    expect(mockInvoke).toHaveBeenCalledTimes(2)
    expect(mockInvoke).toHaveBeenNthCalledWith(1, 'db_browser_query_rows', {
      table: 'documents',
      page: 1,
      pageSize: DB_BROWSER_EXPORT_PAGE_SIZE,
      sortColumn: 'id',
      sortDirection: 'asc',
      search: undefined,
    })
    expect(mockInvoke).toHaveBeenNthCalledWith(2, 'db_browser_query_rows', {
      table: 'documents',
      page: 2,
      pageSize: DB_BROWSER_EXPORT_PAGE_SIZE,
      sortColumn: 'id',
      sortDirection: 'asc',
      search: undefined,
    })
    expect(response.total).toBe(DB_BROWSER_EXPORT_PAGE_SIZE + 1)
    expect(response.rows).toHaveLength(DB_BROWSER_EXPORT_PAGE_SIZE + 1)
  })

  it('stops after the first chunk when it contains all rows', async () => {
    mockInvoke.mockResolvedValueOnce({
      table: 'documents',
      page: 1,
      pageSize: DB_BROWSER_EXPORT_PAGE_SIZE,
      total: 2,
      rows: [{ id: 1 }, { id: 2 }],
    })

    const response = await queryAllDbBrowserRowsInChunks({
      table: 'documents',
      sortColumn: 'id',
      sortDirection: 'desc',
      search: 'acta',
    })

    expect(mockInvoke).toHaveBeenCalledTimes(1)
    expect(mockInvoke).toHaveBeenCalledWith('db_browser_query_rows', {
      table: 'documents',
      page: 1,
      pageSize: DB_BROWSER_EXPORT_PAGE_SIZE,
      sortColumn: 'id',
      sortDirection: 'desc',
      search: 'acta',
    })
    expect(response.rows).toEqual([{ id: 1 }, { id: 2 }])
  })
})
