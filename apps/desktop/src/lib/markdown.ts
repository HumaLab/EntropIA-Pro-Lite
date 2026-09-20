/**
 * Minimal, safe Markdown-to-HTML renderer for untrusted LLM output.
 *
 * Supports the subset that the RAG/LLM answers actually produce: paragraphs,
 * headings (#, ##, ###), bulleted (-, *) and ordered (1.) lists, GFM pipe
 * tables, blockquotes, and inline strong / emphasis / inline-code / links.
 * HTML in the input is always escaped FIRST, so the returned string is safe to
 * bind with {@html} — the only tags present are the ones this renderer emits.
 *
 * Tables and blockquotes are here because the research engine writes both: the
 * coverage of a report is a pipe table and its warnings are a quote. Without
 * them a table arrives as one paragraph of pipes, which is what a reader of an
 * exported report actually sees.
 */

const INLINE_CODE = /`([^`\n]+)`/g
const STRONG = /\*\*([^*\n]+)\*\*|__([^_\n]+)__/g
const EMPHASIS = /\*([^*\n]+)\*/g
const LINK = /\[([^\]]+)\]\(((?:[^()]|\([^()]*\))*)\)/g
const HEADING = /^(#{1,3})\s+(.*)$/
const BULLET_ITEM = /^[-*]\s+(.*)$/
const ORDERED_ITEM = /^\d+[.)]\s+(.*)$/
const QUOTE_LINE = /^>\s?(.*)$/
/* A row must open with a pipe. Prose can hold a pipe; a line that starts with
   one is a table row and nothing else, which keeps the detection cheap and
   keeps a sentence from being read as a header. */
const TABLE_ROW = /^\|/
const TABLE_ALIGNMENT = /^:?-+:?$/

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
}

function isSafeUrl(url: string): boolean {
  return /^(https?:\/\/|mailto:)/i.test(url.trim())
}

/** Apply inline Markdown formatting to an already-HTML-escaped string. */
function renderInline(escaped: string): string {
  return escaped
    .replace(INLINE_CODE, (_m, code: string) => `<code>${code}</code>`)
    .replace(STRONG, (_m, a: string, b: string) => `<strong>${a ?? b}</strong>`)
    .replace(EMPHASIS, (_m, text: string) => `<em>${text}</em>`)
    .replace(LINK, (_m, text: string, url: string) =>
      isSafeUrl(url)
        ? `<a href="${url.trim()}" rel="noopener noreferrer" target="_blank">${text}</a>`
        : text
    )
}

function lineAt(lines: string[], index: number): string {
  return lines[index] ?? ''
}

/** The cells of one pipe row, with the optional outer pipes dropped. */
function tableCells(line: string): string[] {
  return line
    .trim()
    .replace(/^\|/, '')
    .replace(/\|$/, '')
    .split('|')
    .map((cell) => cell.trim())
}

/**
 * The column alignments a divider row declares, or null when the line is not a
 * divider. `null` for a column is the default, left, and writes no style.
 */
function tableAlignments(line: string): (string | null)[] | null {
  if (!TABLE_ROW.test(line.trim())) return null
  const cells = tableCells(line)
  if (cells.length === 0 || !cells.every((cell) => TABLE_ALIGNMENT.test(cell))) return null
  return cells.map((cell) => {
    const left = cell.startsWith(':')
    const right = cell.endsWith(':')
    if (left && right) return 'center'
    if (right) return 'right'
    return null
  })
}

/**
 * A table needs its header AND its divider, so this is asked in exactly two
 * places — the block loop and `isBlockStart` — and must answer the same in
 * both. A lone pipe line that `isBlockStart` called a block but the loop did
 * not would end a paragraph with nothing to put in its place, and the
 * paragraph branch would spin without ever advancing.
 */
function isTableStart(lines: string[], index: number): boolean {
  return (
    TABLE_ROW.test(lineAt(lines, index).trim()) &&
    tableAlignments(lineAt(lines, index + 1)) !== null
  )
}

function isBlockStart(lines: string[], index: number): boolean {
  const trimmed = lineAt(lines, index).trim()
  return (
    trimmed === '' ||
    HEADING.test(trimmed) ||
    BULLET_ITEM.test(trimmed) ||
    ORDERED_ITEM.test(trimmed) ||
    QUOTE_LINE.test(trimmed) ||
    isTableStart(lines, index)
  )
}

/**
 * Render a Markdown string to a safe HTML fragment. Returns '' for empty input.
 */
export function renderMarkdown(input: string): string {
  if (!input) return ''
  const normalized = input.replace(/\r\n?/g, '\n')
  const lines = normalized.split('\n')
  const blocks: string[] = []
  let i = 0

  while (i < lines.length) {
    const trimmed = lineAt(lines, i).trim()

    if (trimmed === '') {
      i++
      continue
    }

    const heading = HEADING.exec(trimmed)
    if (heading) {
      const level = heading[1]?.length ?? 1
      const text = heading[2] ?? ''
      blocks.push(`<h${level}>${renderInline(escapeHtml(text))}</h${level}>`)
      i++
      continue
    }

    // Before the lists: a quoted list opens with `>`, not with `-`.
    if (QUOTE_LINE.test(trimmed)) {
      const quoted: string[] = []
      while (i < lines.length) {
        const match = QUOTE_LINE.exec(lineAt(lines, i).trim())
        if (!match) break
        quoted.push(match[1] ?? '')
        i++
      }
      // Recursive: a quote holds blocks, so a list or a table inside one is
      // rendered by the same rules as outside it.
      blocks.push(`<blockquote>${renderMarkdown(quoted.join('\n'))}</blockquote>`)
      continue
    }

    if (isTableStart(lines, i)) {
      const headers = tableCells(lineAt(lines, i))
      const alignments = tableAlignments(lineAt(lines, i + 1)) ?? []
      i += 2

      const cellStyle = (column: number): string => {
        const alignment = alignments[column]
        return alignment ? ` style="text-align: ${alignment}"` : ''
      }

      const head = headers
        .map((header, column) => `<th${cellStyle(column)}>${renderInline(escapeHtml(header))}</th>`)
        .join('')

      const rows: string[] = []
      while (i < lines.length && TABLE_ROW.test(lineAt(lines, i).trim())) {
        const cells = tableCells(lineAt(lines, i))
        // Padded to the header: a short row that emitted fewer cells would
        // pull every value after it one column to the left.
        const body = headers
          .map(
            (_header, column) =>
              `<td${cellStyle(column)}>${renderInline(escapeHtml(cells[column] ?? ''))}</td>`
          )
          .join('')
        rows.push(`<tr>${body}</tr>`)
        i++
      }

      const bodyHtml = rows.length > 0 ? `<tbody>${rows.join('')}</tbody>` : ''
      blocks.push(`<table><thead><tr>${head}</tr></thead>${bodyHtml}</table>`)
      continue
    }

    if (BULLET_ITEM.test(trimmed)) {
      const items: string[] = []
      while (i < lines.length) {
        const current = lineAt(lines, i).trim()
        if (current === '') {
          i++
          continue
        }
        const match = BULLET_ITEM.exec(current)
        if (!match) break
        items.push(`<li>${renderInline(escapeHtml(match[1] ?? ''))}</li>`)
        i++
      }
      blocks.push(`<ul>${items.join('')}</ul>`)
      continue
    }

    if (ORDERED_ITEM.test(trimmed)) {
      const items: string[] = []
      while (i < lines.length) {
        const current = lineAt(lines, i).trim()
        if (current === '') {
          i++
          continue
        }
        const match = ORDERED_ITEM.exec(current)
        if (!match) break
        items.push(`<li>${renderInline(escapeHtml(match[1] ?? ''))}</li>`)
        i++
      }
      blocks.push(`<ol>${items.join('')}</ol>`)
      continue
    }

    const paragraphLines: string[] = []
    while (i < lines.length && !isBlockStart(lines, i)) {
      paragraphLines.push(lineAt(lines, i).trim())
      i++
    }
    blocks.push(`<p>${renderInline(escapeHtml(paragraphLines.join(' ')))}</p>`)
  }

  return blocks.join('')
}
