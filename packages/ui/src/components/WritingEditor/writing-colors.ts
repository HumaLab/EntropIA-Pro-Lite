/**
 * The manuscript's colours: text colour and highlight.
 *
 * # Names, not hex
 *
 * The app has four themes, two dark and two pale. A red picked on a white page
 * is unreadable on the dark one, and a highlight picked on the dark one glares
 * on the white. So a manuscript stores what the writer meant — "red" — and
 * every theme says what red is on its own surface, through the tokens
 * `--writing-text-<name>` and `--writing-highlight-<name>` in `tokens.css`.
 * `contrast-floor.test.ts` measures every one of them against its surface and
 * against each other, so no combination falls under the reading floor.
 *
 * # What a name the build does not know does
 *
 * Nothing. A hand-edited file or a newer build can carry a name missing here;
 * the document still loads, keeps the name, and draws the text uncoloured.
 *
 * # Printing
 *
 * An export is read on a white page, whatever theme it was written in, so each
 * name has one fixed print colour for text and one for highlight. The
 * exporters all read this table.
 */

export const WRITING_COLORS = [
  'gray',
  'red',
  'orange',
  'yellow',
  'green',
  'blue',
  'purple',
  'pink',
] as const

export type WritingColor = (typeof WRITING_COLORS)[number]

/** A stored or pasted name, or null for anything that is not one of ours. */
export function parseWritingColor(value: unknown): WritingColor | null {
  return typeof value === 'string' && (WRITING_COLORS as readonly string[]).includes(value)
    ? (value as WritingColor)
    : null
}

export function textColorVar(name: WritingColor): string {
  return `var(--writing-text-${name})`
}

export function highlightColorVar(name: WritingColor): string {
  return `var(--writing-highlight-${name})`
}

/**
 * The palette name an element was drawn with, read back from the forms this
 * editor writes: the data attribute, or failing that the token in its inline
 * style. A hex or a CSS colour name is someone else's colour and reads as none.
 */
function readDrawn(element: HTMLElement, attribute: string, property: string, token: string) {
  const named = parseWritingColor(element.getAttribute(attribute))
  if (named) return named
  const style = element.getAttribute('style') ?? ''
  const pattern = new RegExp(`(?:^|;)\\s*${property}\\s*:\\s*var\\(--writing-${token}-([a-z]+)\\)`)
  return parseWritingColor(pattern.exec(style)?.[1])
}

export function readTextColor(element: HTMLElement): WritingColor | null {
  return readDrawn(element, 'data-text-color', 'color', 'text')
}

export function readHighlightColor(element: HTMLElement): WritingColor | null {
  return readDrawn(element, 'data-highlight', 'background-color', 'highlight')
}

/**
 * On white paper: the light theme's own values, which already hold every text
 * colour at 4.5:1 or more on each highlight and on white.
 */
export const PRINT_COLORS: Record<WritingColor, { text: string; highlight: string }> = {
  gray: { text: '#4e535c', highlight: '#d2d4d9' },
  red: { text: '#9f211c', highlight: '#f5cbc9' },
  orange: { text: '#82410e', highlight: '#f6cdad' },
  yellow: { text: '#655008', highlight: '#f3d265' },
  green: { text: '#1f5d34', highlight: '#a6e1ba' },
  blue: { text: '#17519e', highlight: '#bfd6f6' },
  purple: { text: '#6c32ae', highlight: '#decdf1' },
  pink: { text: '#99205c', highlight: '#f3c8de' },
}

export type WritingColorLabelKey = `color${Capitalize<WritingColor>}`

/** The editor label that names a colour on screen: `red` → `colorRed`. */
export function colorLabelKey(name: WritingColor): WritingColorLabelKey {
  return `color${name[0]!.toUpperCase()}${name.slice(1)}` as WritingColorLabelKey
}
