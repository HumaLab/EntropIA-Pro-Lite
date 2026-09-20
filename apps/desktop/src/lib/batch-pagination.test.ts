import { describe, expect, it } from 'vitest'
import {
  pageWindow,
  splitIntoTables,
  tablesForWidth,
  TASKS_PER_PAGE,
  THREE_TABLES_FROM,
  TWO_TABLES_FROM,
} from './batch-pagination'

describe('how many tables fit', () => {
  it('adds a table only once the list is wide enough to hold another', () => {
    expect([
      tablesForWidth(0),
      tablesForWidth(TWO_TABLES_FROM - 1),
      tablesForWidth(TWO_TABLES_FROM),
      tablesForWidth(THREE_TABLES_FROM - 1),
      tablesForWidth(THREE_TABLES_FROM),
      tablesForWidth(4000),
    ]).toEqual([1, 1, 2, 2, 3, 3])
  })
})

describe('dealing one page among the tables', () => {
  const page = Array.from({ length: TASKS_PER_PAGE }, (_, index) => index)

  it('keeps every unit, once, in the original order', () => {
    for (const count of [1, 2, 3]) {
      const tables = splitIntoTables(page, count)

      // The three ways this goes wrong are all invisible in one table and all
      // fatal in two: a unit read twice, a unit lost between tables, and a
      // page that no longer runs in order.
      expect(tables.flat()).toEqual(page)
    }
  })

  it('splits the page evenly rather than multiplying it', () => {
    expect(splitIntoTables(page, 1).map((table) => table.length)).toEqual([48])
    expect(splitIntoTables(page, 2).map((table) => table.length)).toEqual([24, 24])
    expect(splitIntoTables(page, 3).map((table) => table.length)).toEqual([16, 16, 16])
  })

  it('gives each table consecutive units, never a round-robin deal', () => {
    const [first, second] = splitIntoTables(page, 2)

    expect(first?.at(-1)).toBe(23)
    expect(second?.at(0)).toBe(24)
  })

  it('divides a page that does not split evenly without dropping the remainder', () => {
    const fifty = Array.from({ length: 50 }, (_, index) => index)

    expect(splitIntoTables(fifty, 3).map((table) => table.length)).toEqual([17, 17, 16])
    expect(splitIntoTables(fifty, 3).flat()).toEqual(fifty)
  })

  it('draws no empty table when there are too few units to fill them', () => {
    expect(splitIntoTables([1, 2], 3)).toEqual([[1], [2]])
    expect(splitIntoTables([], 3)).toEqual([])
  })
})

describe('which page numbers the pager shows', () => {
  it('elides nothing while every page number fits', () => {
    expect(pageWindow(1, 7)).toEqual([1, 2, 3, 4, 5, 6, 7])
  })

  it('always offers the first and last page, however far away they are', () => {
    expect(pageWindow(5, 10)).toEqual([1, 'gap', 4, 5, 6, 'gap', 10])
  })

  it('opens a gap only on the side that has pages to hide', () => {
    expect(pageWindow(1, 10)).toEqual([1, 2, 'gap', 10])
    expect(pageWindow(10, 10)).toEqual([1, 'gap', 9, 10])
  })

  it('never repeats the first or last page as a neighbour', () => {
    for (let current = 1; current <= 10; current += 1) {
      const pages = pageWindow(current, 10).filter((page) => page !== 'gap')

      expect(new Set(pages).size).toBe(pages.length)
    }
  })
})
