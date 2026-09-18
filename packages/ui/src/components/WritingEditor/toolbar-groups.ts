/**
 * The writing toolbar as data.
 *
 * Each group is a run of tools between two separators, with a priority that
 * decides when it collapses into the overflow menu (see `toolbar-fit.ts`). A
 * new tool or group is a new entry in the list the editor builds; nothing here
 * or in the fitting logic has to change for it.
 */
import type { ActionIconName } from '../Button/ActionIcon.types'
import type { ToolbarMenuItem } from '../ToolbarMenu/ToolbarMenu.types'
import type { ToolbarGroupPriority } from './toolbar-fit'
import type { WritingColor } from './writing-colors'

export interface ToolbarTool {
  id: string
  label: string
  icon: ActionIconName
  /**
   * Whether the format is on. Present only on toggles: the row shows it as
   * pressed, the menu as checked. A plain command leaves it out.
   */
  active?: boolean
  disabled?: boolean
  variant?: 'ghost' | 'danger'
  /** Keeps a press from moving the focus, so the caret stays where it was. */
  keepFocus?: boolean
  /** A separator inside the group, before this tool. */
  separated?: boolean
  /**
   * The tool opens a menu of these instead of running a command. In the
   * overflow menu its entries stand in its place, since a menu inside a menu
   * is one more thing to steer with the arrow keys and nothing more to read.
   */
  menu?: ToolbarMenuItem[]
  /**
   * In the overflow menu, the entries of `menu` go under a small heading with
   * the tool's label, for a menu whose entries do not name it themselves
   * (line spacing's bare numbers).
   */
  menuHeading?: boolean
  /**
   * One of a set of mutually exclusive choices (the alignments): the row
   * shows it as pressed, the overflow menu as a radio item. Needs `active`.
   */
  radio?: boolean
  /**
   * The tool opens a colour palette (ColorPalette.svelte). Nine entries would
   * double the overflow menu as a list, so there the palette is drawn whole,
   * after the listed tools, as the same compact grid.
   */
  palette?: ToolbarPalette
  run: () => void
}

export interface ToolbarPalette {
  /** Whose tokens the swatches are drawn with. */
  kind: 'text' | 'highlight'
  /** The colour the selection has, or null for none (or one this build does not know). */
  current: WritingColor | null
  /** Null takes the colour off. */
  apply: (name: WritingColor | null) => void
}

export interface ToolbarGroup {
  id: string
  /** `'pinned'` never collapses; lower numbers collapse first. */
  priority: ToolbarGroupPriority
  /** Sits against the previous group with no separator between them. */
  joined?: boolean
  tools: ToolbarTool[]
}

/**
 * One stretch of the overflow menu: a run of listed entries, or a colour
 * palette drawn whole.
 */
export type OverflowSection =
  | { kind: 'items'; id: string; items: ToolbarMenuItem[] }
  | { kind: 'palette'; tool: ToolbarTool & { palette: ToolbarPalette } }

function listed(tool: ToolbarTool): ToolbarMenuItem[] {
  if (tool.menu) {
    const entries: ToolbarMenuItem[] = tool.menuHeading
      ? [{ kind: 'heading', id: `${tool.id}-heading`, label: tool.label }]
      : []
    for (const entry of tool.menu) {
      if (entry.kind === 'separator' || entry.kind === 'heading') continue
      entries.push({ ...entry, disabled: tool.disabled || entry.disabled })
    }
    return entries
  }
  return [
    {
      id: tool.id,
      kind: tool.active === undefined ? 'action' : tool.radio ? 'radio' : 'checkbox',
      label: tool.label,
      icon: tool.icon,
      checked: tool.active,
      disabled: tool.disabled,
      onselect: tool.run,
    },
  ]
}

/**
 * The overflow menu's contents: every tool of the collapsed groups, in
 * toolbar order, with a separator between groups so they still read as
 * groups. A palette interrupts the list where its tool stands, so the order
 * holds whichever groups collapse.
 */
export function overflowSections(
  groups: readonly ToolbarGroup[],
  hidden: readonly string[]
): OverflowSection[] {
  const sections: OverflowSection[] = []
  let run: ToolbarMenuItem[] = []
  let started = false
  const flush = () => {
    if (run.length > 0) sections.push({ kind: 'items', id: `items-${run[0]!.id}`, items: run })
    run = []
  }
  for (const group of groups) {
    if (!hidden.includes(group.id)) continue
    if (started) run.push({ kind: 'separator', id: `${group.id}-separator` })
    started = true
    for (const tool of group.tools) {
      if (tool.palette) {
        // A palette is headed and ruled off by its own drawing.
        if (run.at(-1)?.kind === 'separator') run.pop()
        flush()
        sections.push({ kind: 'palette', tool: tool as ToolbarTool & { palette: ToolbarPalette } })
        continue
      }
      run.push(...listed(tool))
    }
  }
  flush()
  return sections
}
