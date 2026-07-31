/**
 * Minimal, safe Markdown-to-HTML renderer for untrusted LLM output.
 *
 * Supports the subset that the RAG/LLM answers actually produce: paragraphs,
 * headings (#, ##, ###), bulleted (-, *) and ordered (1.) lists, and inline
 * strong / emphasis / inline-code / links. HTML in the input is always escaped
 * FIRST, so the returned string is safe to bind with {@html} — the only tags
 * present are the ones this renderer emits.
 */

const INLINE_CODE = /`([^`\n]+)`/g
const STRONG = /\*\*([^*\n]+)\*\*|__([^_\n]+)__/g
const EMPHASIS = /\*([^*\n]+)\*/g
const LINK = /\[([^\]]+)\]\(((?:[^()]|\([^()]*\))*)\)/g
const HEADING = /^(#{1,3})\s+(.*)$/
const BULLET_ITEM = /^[-*]\s+(.*)$/
const ORDERED_ITEM = /^\d+[.)]\s+(.*)$/

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

function isBlockStart(lines: string[], index: number): boolean {
  const trimmed = lineAt(lines, index).trim()
  return (
    trimmed === '' ||
    HEADING.test(trimmed) ||
    BULLET_ITEM.test(trimmed) ||
    ORDERED_ITEM.test(trimmed)
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
