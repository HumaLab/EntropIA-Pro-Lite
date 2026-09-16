/**
 * Turning a reader's selection into a citation anchor (plan-editor.md §10.1).
 *
 * The anchor a citation stores is `[start_char, end_char)` into the page's
 * `extractions.text_content`, decided in §29.1 and scoped in the G3 entry of
 * Unit 4. This is the one place that converts what someone highlighted on
 * screen into those offsets.
 *
 * It works because the page text is rendered as a single interpolated text
 * node: the DOM offsets inside it *are* the character offsets into the
 * extraction. Rendering that text with any markup — a highlight, a line break
 * element, a search hit — would break that identity silently, so the
 * conversion is checked rather than assumed.
 */

export interface SourceRange {
  start: number
  end: number
  text: string
}

/**
 * The selected range inside `container`, or null when there is no usable one.
 *
 * Null rather than an empty range for: no selection, a collapsed caret, a
 * selection that started outside this element, or one that spans more than the
 * single text node the page text is rendered as. Each of those would otherwise
 * produce offsets that do not mean what the citation claims they mean.
 */
export function selectionRange(
  container: HTMLElement | null,
  selection: Selection | null
): SourceRange | null {
  if (!container || !selection || selection.rangeCount === 0) return null
  if (selection.isCollapsed) return null

  // The Range rather than anchor/focus: a Range is ordered by construction, so
  // a selection dragged backwards needs no special case, and there is no pair
  // of offsets to accidentally compare the wrong way round.
  const range = selection.getRangeAt(0)
  const { startContainer, endContainer, startOffset, endOffset } = range

  // Both ends must be the same text node. The page text is rendered as exactly
  // one, so anything else means the selection left it — and an offset into a
  // different node is not an offset into the extraction.
  if (startContainer !== endContainer) return null
  if (startContainer.nodeType !== Node.TEXT_NODE) return null
  if (!container.contains(startContainer)) return null
  if (endOffset <= startOffset) return null

  const whole = startContainer.textContent ?? ''
  const text = whole.slice(startOffset, endOffset)
  if (!text.trim()) return null

  return { start: startOffset, end: endOffset, text }
}

/**
 * A fingerprint of the quoted text, for noticing later that the source moved.
 *
 * SHA-256 of its UTF-8 bytes, hex encoded. Spike S2 measured that a content
 * hash is what discriminates correctly here, where a stored position does not:
 * a position still resolves after the source is edited, it just resolves to the
 * wrong words, and only comparing the content can tell.
 *
 * Anything that verifies this later — the `integrity_status` column is where it
 * would land — has to hash the same way: SHA-256 over the UTF-8 bytes of the
 * quoted text exactly as stored, with no trimming or normalisation.
 */
export async function hashSourceText(text: string): Promise<string | null> {
  const subtle = globalThis.crypto?.subtle
  // A missing SubtleCrypto is not a reason to refuse the citation. The hash is
  // an aid to noticing drift later, not part of the anchor: without it the
  // citation still resolves by asset and offsets.
  if (!subtle) return null
  const bytes = new TextEncoder().encode(text)
  const digest = await subtle.digest('SHA-256', bytes)
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, '0')).join('')
}
