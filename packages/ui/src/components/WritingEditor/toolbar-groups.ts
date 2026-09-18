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
  run: () => void
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
