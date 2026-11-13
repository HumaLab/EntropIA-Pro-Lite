import { invoke } from '@tauri-apps/api/core'

export const DB_BROWSER_EXPORT_PAGE_SIZE = 1000

export type DbBrowserSortDirection = 'asc' | 'desc'

export interface DbBrowserTable {
  name: string
}

export interface DbBrowserColumn {
  name: string
  dataType: string
  nullable: boolean
  isPrimaryKey: boolean
}

export interface DbBrowserQueryRequest {
  table: string
  page: number
  pageSize: number
  sortColumn?: string
  sortDirection?: DbBrowserSortDirection
  search?: string
}

export interface DbBrowserQueryResponse {
  table: string
  page: number
  pageSize: number
  total: number
  rows: Record<string, unknown>[]
}

export function listDbBrowserTables(): Promise<DbBrowserTable[]> {
  return invoke<DbBrowserTable[]>('db_browser_list_tables')
}

export function describeDbBrowserTable(table: string): Promise<DbBrowserColumn[]> {
  return invoke<DbBrowserColumn[]>('db_browser_describe_table', { table })
}

export function queryDbBrowserRows(
  request: DbBrowserQueryRequest
): Promise<DbBrowserQueryResponse> {
  return invoke<DbBrowserQueryResponse>('db_browser_query_rows', {
    table: request.table,
    page: request.page,
    pageSize: request.pageSize,
    sortColumn: request.sortColumn,
    sortDirection: request.sortDirection,
    search: request.search,
  })
}

export async function queryAllDbBrowserRowsInChunks(
  request: Omit<DbBrowserQueryRequest, 'page' | 'pageSize'>,
  chunkSize = DB_BROWSER_EXPORT_PAGE_SIZE
): Promise<DbBrowserQueryResponse> {
  const rows: Record<string, unknown>[] = []
  let total: number
  let page = 1

  do {
    const response = await queryDbBrowserRows({
      ...request,
      page,
      pageSize: chunkSize,
    })

    total = response.total
    rows.push(...response.rows)

    if (response.rows.length === 0) break
    page += 1
  } while (rows.length < total)

  return {
    table: request.table,
    page: 1,
    pageSize: chunkSize,
    total,
    rows,
  }
}
