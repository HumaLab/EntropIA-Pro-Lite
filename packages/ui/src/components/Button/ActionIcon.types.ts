/**
 * The icon contract for the whole app.
 *
 * Two closed sets govern every icon rendered anywhere: the catalogue of names
 * below, and the size scale. Both are declared as runtime arrays first and the
 * types are derived from them, so a test can iterate the real catalogue instead
 * of restating it and drifting.
 *
 * Rules that the rest of the system depends on:
 *  - One family (Lucide), one geometry: 24x24 grid, stroke 2, round caps/joins.
 *    Icons authored in-house follow the same metric.
 *  - Sizes are even. A stroke of 2 on a 24 viewBox scaled to an odd pixel size
 *    lands on a half pixel and renders paler than its neighbours.
 *  - Names describe the UI role ('delete'), not the Lucide export ('trash-2').
 *    Directions ('chevron-*') are the exception: their shape is their meaning.
 *  - The icon never carries the accessible name. ActionIcon marks every svg
 *    aria-hidden; the label belongs on the button that wraps it.
 */

export const ACTION_ICON_NAMES = [
  'add',
  'bell',
  'bold',
  'broom',
  'check',
  'check-check',
  'chevron-down',
  'chevron-left',
  'chevron-right',
  'chevron-up',
  'chevrons-left',
  'chevrons-right',
  'circle-check',
  'circle-help',
  'circle-play',
  'circle-x',
  'close',
  'code',
  'copy',
  'crop',
  'database',
  'delete',
  'download',
  'edit',
  'eraser',
  'expand',
  'external-link',
  'eye',
  'eye-off',
  'file',
  'file-audio',
  'file-braces',
  'file-image',
  'file-spreadsheet',
  'file-text',
  'file-up',
  'folder',
  'folder-plus',
  'hand',
  'heading-1',
  'heading-2',
  'heading-3',
  'italic',
  'languages',
  'link',
  'list',
  'list-ordered',
  'loader',
  'map-pin',
  'map-pin-pen',
  'message-circle',
  'message-circle-plus',
  'mic',
  'panel-left',
  'panel-left-close',
  'pause',
  'play',
  'rectangle',
  'redo',
  'refresh',
  'rotate-ccw',
  'rotate-cw',
  'rotate-fine-ccw',
  'rotate-fine-cw',
  'save',
  'search',
  'search-x',
  'send',
  'settings',
  'skip-back',
  'skip-forward',
  'text-quote',
  'theme',
  'triangle-alert',
  'underline',
  'undo',
  'unlink',
  'volume',
  'wrench',
  'zoom-in',
  'zoom-out',
] as const

export type ActionIconName = (typeof ACTION_ICON_NAMES)[number]

/**
 * The closed size scale, paired with its container in IconButton:
 *   12 -> 24 (chips, table cells)      14 -> 28 (row actions, dense panels)
 *   16 -> 32 (toolbars, buttons)       20 -> 40 (panel headers, statusbar)
 *   24 -> 44 (banners, dialogs)        40      (empty states, card placeholders)
 */
export const ACTION_ICON_SIZES = [12, 14, 16, 20, 24, 40] as const

export type ActionIconSize = (typeof ACTION_ICON_SIZES)[number]
