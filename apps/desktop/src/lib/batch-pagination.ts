/**
 * The arithmetic behind the unit list in Configuración → Lotes: how many
 * tables fit, how one page of units is dealt among them, and which page
 * numbers the pager shows.
 *
 * It lives apart from the component because none of it can be checked through
 * one. The test environment performs no layout, so every panel it renders is
 * zero pixels wide and every split collapses to a single table — the case that
 * was never in doubt. Here the same rules are ordinary functions, and the
 * invariants that matter (order kept, nothing repeated, nothing dropped) are
 * things a test can actually state.
 */

/** Divides evenly by 1, 2 and 3, so no table is left a row short. */
export const TASKS_PER_PAGE = 48

/**
 * How wide the list has to be before another table fits beside the first. One
 * table's four columns come to about 22.5rem, and below that they start
 * colliding, so these are two and three of those plus the gaps between them.
 */
export const TWO_TABLES_FROM = 740
export const THREE_TABLES_FROM = 1120

/** How many pages a result set of this size needs. Always at least one. */
export function pageCountFor(total: number): number {
  return Math.max(1, Math.ceil(total / TASKS_PER_PAGE))
}

/**
 * The size of the result set the pager is walking. The snapshot already counts
 * every unit by state, so this costs no query — filtered or not.
 */
export function countForFilter(
  countsByState: readonly { name: string; count: number }[],
  filter: string
): number {
  if (!filter) return countsByState.reduce((sum, entry) => sum + entry.count, 0)
  return countsByState.find((entry) => entry.name === filter)?.count ?? 0
}

/** Measured on the list's own box — never the window's. */
export function tablesForWidth(width: number): number {
  if (width >= THREE_TABLES_FROM) return 3
  if (width >= TWO_TABLES_FROM) return 2
  return 1
}

/**
 * Consecutive slices in the original order: every item lands in exactly one
 * table, and reading down the first and on into the second reads the page in
 * order. A round-robin deal would scatter neighbours across columns.
 *
 * Fewer tables than asked for is the honest answer when there are not enough
 * items to fill them — three columns for four rows would draw an empty one.
 */
export function splitIntoTables<T>(items: readonly T[], count: number): T[][] {
  if (items.length === 0) return []
  if (count <= 1) return [[...items]]
  const perTable = Math.ceil(items.length / count)
  const tables: T[][] = []
  for (let at = 0; at < items.length; at += perTable) {
    tables.push(items.slice(at, at + perTable))
  }
  return tables
}

/**
 * First, last, the current page and its neighbours; a gap stands for the runs
 * left out. Up to seven pages every number fits, so nothing is elided.
 */
export function pageWindow(current: number, total: number): (number | 'gap')[] {
  if (total <= 7) return Array.from({ length: total }, (_, index) => index + 1)
  const pages: (number | 'gap')[] = [1]
  const from = Math.max(2, current - 1)
  const to = Math.min(total - 1, current + 1)
  if (from > 2) pages.push('gap')
  for (let page = from; page <= to; page += 1) pages.push(page)
  if (to < total - 1) pages.push('gap')
  pages.push(total)
  return pages
}
