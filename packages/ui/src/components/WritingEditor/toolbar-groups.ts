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
 * The overflow menu's entries: every tool of the collapsed groups, in toolbar
 * order, with a separator between groups so they still read as groups.
 */
export function overflowMenuItems(
  groups: readonly ToolbarGroup[],
  hidden: readonly string[]
): ToolbarMenuItem[] {
  const items: ToolbarMenuItem[] = []
  for (const group of groups) {
    if (!hidden.includes(group.id)) continue
    if (items.length > 0) items.push({ kind: 'separator', id: `${group.id}-separator` })
    for (const tool of group.tools) {
      if (tool.palette) continue
      if (tool.menu) {
        for (const entry of tool.menu) {
          if (entry.kind === 'separator') continue
          items.push({ ...entry, disabled: tool.disabled || entry.disabled })
        }
        continue
      }
      items.push({
        id: tool.id,
        kind: tool.active === undefined ? 'action' : 'checkbox',
        label: tool.label,
        icon: tool.icon,
        checked: tool.active,
        disabled: tool.disabled,
        onselect: tool.run,
      })
    }
  }
  return items
}

/**
 * The palettes of the collapsed groups, drawn after the listed entries. That
 * keeps toolbar order only while the palettes belong to the last group that
 * can collapse, which is typography — a test holds it there.
 */
export function overflowPalettes(
  groups: readonly ToolbarGroup[],
  hidden: readonly string[]
): ToolbarTool[] {
  return groups
    .filter((group) => hidden.includes(group.id))
    .flatMap((group) => group.tools.filter((tool) => tool.palette))
}
