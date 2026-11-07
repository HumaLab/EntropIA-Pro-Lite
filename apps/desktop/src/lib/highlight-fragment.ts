import { mapRenderedText } from './rendered-text-map'

/**
 * Marks every rendered text-node segment covered by a citation's raw range.
 *
 * Citation offsets refer to the raw extraction, while this container holds the
 * rendered text. The rendered-text map translates that range without guessing;
 * the resulting visible offsets are then intersected with each DOM text node so
 * formatting elements remain intact.
 */
export function highlightCitationRange(
  container: HTMLElement | null,
  rawText: string,
  sourceRange: { start: number; end: number },
  markClass = 'citation-hit'
): boolean {
  if (!container) return false
  clearHighlight(container, markClass)

  const visibleRange = mapRenderedText(container.textContent ?? '', rawText).toVisible(
    sourceRange.start,
    sourceRange.end
  )
  if (!visibleRange) return false

  const walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT)
  const segments: Array<{ node: Text; start: number; end: number }> = []
  let offset = 0
  for (let current = walker.nextNode(); current; current = walker.nextNode()) {
    const node = current as Text
    const nodeEnd = offset + (node.textContent?.length ?? 0)
    const start = Math.max(visibleRange.start, offset)
    const end = Math.min(visibleRange.end, nodeEnd)
    if (start < end) {
      segments.push({ node, start: start - offset, end: end - offset })
    }
    offset = nodeEnd
  }
  if (segments.length === 0) return false

  let firstMark: HTMLElement | null = null
  for (let index = segments.length - 1; index >= 0; index -= 1) {
    const segment = segments[index]!
    firstMark = wrap(segment.node, segment.start, segment.end, markClass)
  }
  firstMark?.scrollIntoView({ block: 'center' })
  return true
}

export interface HighlightRange {
  start: number
  end: number
  role: 'passage' | 'subject' | 'predicate' | 'object'
}

/** Maps all raw ranges once and partitions overlaps without nested marks. */
export function highlightLabeledRanges(
  container: HTMLElement | null,
  rawText: string,
  ranges: readonly HighlightRange[]
): boolean {
  if (!container) return false
  clearHighlight(container, 'occurrence-hit')
  const mapping = mapRenderedText(container.textContent ?? '', rawText)
  const visible = ranges.flatMap((range) => {
    const mapped = mapping.toVisible(range.start, range.end)
    return mapped ? [{ ...mapped, role: range.role }] : []
  })
  const walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT)
  const segments: Array<{ node: Text; start: number; end: number; classes: string }> = []
  let offset = 0
  for (let current = walker.nextNode(); current; current = walker.nextNode()) {
    const node = current as Text
    const nodeEnd = offset + node.length
    const overlapping = visible.filter((range) => range.start < nodeEnd && range.end > offset)
    const boundaries = [
      ...new Set(
        overlapping.flatMap((range) => [
          Math.max(offset, range.start),
          Math.min(nodeEnd, range.end),
        ])
      ),
    ].sort((a, b) => a - b)
    for (let i = 1; i < boundaries.length; i++) {
      const start = boundaries[i - 1]!
      const end = boundaries[i]!
      const roles = overlapping
        .filter((range) => range.start < end && range.end > start)
        .map((range) => `occurrence-${range.role}`)
      if (roles.length) {
        segments.push({
          node,
          start: start - offset,
          end: end - offset,
          classes: `occurrence-hit ${[...new Set(roles)].join(' ')}`,
        })
      }
    }
    offset = nodeEnd
  }
  let first: HTMLElement | null = null
  for (let i = segments.length - 1; i >= 0; i--) {
    const segment = segments[i]!
    first = wrap(segment.node, segment.start, segment.end, segment.classes)
  }
  first?.scrollIntoView({ block: 'center' })
  return first !== null
}

/** Removes any mark a previous call left, so two visits do not stack. */
export function clearHighlight(container: HTMLElement | null, markClass = 'citation-hit'): void {
  if (!container) return
  for (const mark of [...container.querySelectorAll(`mark.${markClass}`)]) {
    const parent = mark.parentNode
    if (!parent) continue
    while (mark.firstChild) parent.insertBefore(mark.firstChild, mark)
    parent.removeChild(mark)
    parent.normalize()
  }
}

function wrap(node: Text, start: number, end: number, markClass: string): HTMLElement {
  const range = document.createRange()
  range.setStart(node, start)
  range.setEnd(node, end)
  const mark = document.createElement('mark')
  mark.className = markClass
  range.surroundContents(mark)
  return mark
}
