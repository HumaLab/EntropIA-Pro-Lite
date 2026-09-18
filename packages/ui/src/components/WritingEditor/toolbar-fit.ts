/**
 * Which toolbar groups fit on one row, and which collapse into the overflow
 * menu.
 *
 * Pure arithmetic over measured widths, so it can be held exactly by a test and
 * so the component that measures cannot feed its own output back into the
 * decision: nothing here reads the DOM, and a group's width does not depend on
 * which other groups are showing.
 *
 * The row is a flex row with a gap. Between two consecutive units there is
 * either a bare gap (the later unit is `joined` to the one before it, as the
 * formatting marks are) or a separator with a gap on each side.
 */

/** `'pinned'` never collapses; otherwise lower numbers collapse first. */
export type ToolbarGroupPriority = number | 'pinned'

export interface ToolbarFitGroup {
  id: string
  /** Natural width of the group's own box, in CSS pixels. */
  width: number
  priority: ToolbarGroupPriority
  /** Sits against the previous unit with no separator between them. */
  joined?: boolean
}

export interface ToolbarFitInput {
  /** In display order. */
  groups: readonly ToolbarFitGroup[]
  /** Content-box width of the row. */
  available: number
  /** The row's flex gap. */
  gap: number
  /** A separator's outer width, margins included. */
  separatorWidth: number
  /** The overflow trigger's width. */
  overflowWidth: number
  /**
   * The group the trigger is placed in front of. When that group is not
   * visible, or none is named, the trigger goes last.
   */
  overflowBefore?: string
}

export interface ToolbarFit {
  /** Ids of the groups on the row, in display order. */
  visible: string[]
  /** Ids of the collapsed groups, in the order they collapsed. */
  hidden: string[]
  /** Whether the overflow trigger is on the row. */
  overflow: boolean
}

export interface RowUnit {
  width: number
  joined?: boolean
}

/** The width a flex row of these units takes, separators and gaps included. */
export function rowWidth(units: readonly RowUnit[], gap: number, separatorWidth: number): number {
  return units.reduce((total, unit, index) => {
    if (index === 0) return unit.width
    const boundary = unit.joined ? gap : gap + separatorWidth + gap
    return total + boundary + unit.width
  }, 0)
}

/** The units of a row with `hidden` collapsed, the trigger included when needed. */
function unitsOf(input: ToolbarFitInput, hidden: ReadonlySet<string>): RowUnit[] {
  const visible = input.groups.filter((group) => !hidden.has(group.id))
  if (hidden.size === 0) return visible
  const trigger: RowUnit = { width: input.overflowWidth, joined: false }
  const at = visible.findIndex((group) => group.id === input.overflowBefore)
  const units: RowUnit[] = [...visible]
  units.splice(at === -1 ? units.length : at, 0, trigger)
  return units
}

/**
 * Collapse order: ascending priority, and among equals the rightmost first, so
 * a row shrinks from its end. The hidden set is always a prefix of this order,
 * which is what keeps a group from reappearing while a narrower one vanishes as
 * the width moves by a pixel.
 */
function collapseOrder(groups: readonly ToolbarFitGroup[]): ToolbarFitGroup[] {
  return groups
    .map((group, index) => ({ group, index }))
    .filter(({ group }) => group.priority !== 'pinned')
    .sort(
      (a, b) => (a.group.priority as number) - (b.group.priority as number) || b.index - a.index
    )
    .map(({ group }) => group)
}

export function fitToolbar(input: ToolbarFitInput): ToolbarFit {
  const all = input.groups.map((group) => group.id)
  const fits = (hidden: ReadonlySet<string>) =>
    rowWidth(unitsOf(input, hidden), input.gap, input.separatorWidth) <= input.available

  // No width yet means no layout yet, not a zero-width toolbar.
  if (input.available <= 0 || fits(new Set())) {
    return { visible: all, hidden: [], overflow: false }
  }

  const order = collapseOrder(input.groups)
  const hidden = new Set<string>()
  for (const group of order) {
    hidden.add(group.id)
    if (fits(hidden)) break
  }

  return {
    visible: all.filter((id) => !hidden.has(id)),
    hidden: order.filter((group) => hidden.has(group.id)).map((group) => group.id),
    overflow: hidden.size > 0,
  }
}
