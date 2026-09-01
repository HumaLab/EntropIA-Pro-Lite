/**
 * Virtual grid arithmetic.
 *
 * Deliberately free of the DOM. The component that uses this cannot be tested
 * in happy-dom, which reports no layout at all — so the part worth getting
 * right lives here, where it can be checked against exact numbers, and the
 * component stays a thin shell that measures and applies.
 *
 * The grid is uniform and fixed-height, which is the trivial case: one row
 * height, one column count, and two spacer divs standing in for the rows that
 * are not rendered.
 */

export type VirtualGridWindowInput = {
  /** Scroll position of the scrolling ancestor. */
  scrollTop: number
  /** Distance from the top of the scroll content to the top of the grid. */
  gridOffset: number
  rowHeight: number
  columns: number
  totalItems: number
  /** Extra rows kept mounted above and below, so scrolling does not race
   *  rendering and so a focused card survives small movements. */
  overscanRows: number
  viewportHeight: number
}

export type VirtualGridWindow = {
  /** First rendered item index, inclusive. */
  startIndex: number
  /** Last rendered item index, exclusive. */
  endIndex: number
  /** Height of the spacer standing in for the rows above the window. */
  beforeHeight: number
  /** Height of the spacer standing in for the rows below it. */
  afterHeight: number
}

/**
 * Which slice of the collection to keep mounted, and how tall the spacers that
 * hold the place of the rest must be.
 */
export function computeVirtualGridWindow(input: VirtualGridWindowInput): VirtualGridWindow {
  const columns = Math.max(1, input.columns)
  const totalRows = Math.ceil(input.totalItems / columns)

  if (input.totalItems <= 0) {
    return { startIndex: 0, endIndex: 0, beforeHeight: 0, afterHeight: 0 }
  }

  // Before the first paint — and in any test environment without layout — the
  // measured height is zero. Rendering an empty window there would leave the
  // grid permanently blank, so fall back to rendering everything and let the
  // first real measurement narrow it.
  if (input.rowHeight <= 0 || input.viewportHeight <= 0) {
    return { startIndex: 0, endIndex: input.totalItems, beforeHeight: 0, afterHeight: 0 }
  }

  const scrolledIntoGrid = Math.max(0, input.scrollTop - input.gridOffset)
  const firstVisibleRow = Math.floor(scrolledIntoGrid / input.rowHeight)
  const lastVisibleRow = Math.ceil((scrolledIntoGrid + input.viewportHeight) / input.rowHeight)

  const startRow = Math.max(0, Math.min(firstVisibleRow - input.overscanRows, totalRows - 1))
  const endRow = Math.min(totalRows, Math.max(startRow + 1, lastVisibleRow + input.overscanRows))

  return {
    startIndex: startRow * columns,
    endIndex: Math.min(input.totalItems, endRow * columns),
    beforeHeight: startRow * input.rowHeight,
    afterHeight: (totalRows - endRow) * input.rowHeight,
  }
}

/**
 * How many columns fit. Mirrors what `repeat(auto-fill, minmax(min, 1fr))`
 * resolves to, because the window math needs the number that CSS will actually
 * use rather than an assumption about it.
 */
export function resolveColumnCount(input: {
  containerWidth: number
  minColumnWidth: number
  gap: number
}): number {
  if (input.containerWidth <= 0 || input.minColumnWidth <= 0) return 1

  const fitted = Math.floor(
    (input.containerWidth + input.gap) / (input.minColumnWidth + input.gap)
  )
  return Math.max(1, fitted)
}

export type FocusTarget =
  | { kind: 'keep'; key: string | null }
  | { kind: 'handoff'; key: string }
  | { kind: 'container'; key: null }

/**
 * Where focus should go when the rendered window changes.
 *
 * Virtualization unmounts elements, and unmounting the focused element sends
 * focus to `<body>` — the user loses their place entirely and keyboard
 * navigation starts over from the top of the document. So the grid never lets
 * that happen silently: it hands focus to the nearest card still rendered,
 * searching forward in document order first and then backward, and only takes
 * focus itself when nothing at all survives.
 */
export function resolveFocusTarget(input: {
  focusedKey: string | null
  previousKeys: string[]
  nextKeys: string[]
}): FocusTarget {
  if (input.focusedKey === null) return { kind: 'keep', key: null }
  if (input.nextKeys.includes(input.focusedKey)) return { kind: 'keep', key: input.focusedKey }

  const survivors = new Set(input.nextKeys)
  const from = input.previousKeys.indexOf(input.focusedKey)
  if (from < 0) return { kind: 'container', key: null }

  for (let index = from + 1; index < input.previousKeys.length; index += 1) {
    const key = input.previousKeys[index]!
    if (survivors.has(key)) return { kind: 'handoff', key }
  }

  for (let index = from - 1; index >= 0; index -= 1) {
    const key = input.previousKeys[index]!
    if (survivors.has(key)) return { kind: 'handoff', key }
  }

  return { kind: 'container', key: null }
}
