/**
 * Finding a cited fragment inside rendered text (plan-editor.md §10.2 step 4).
 *
 * # Why this searches for the text rather than using the stored offsets
 *
 * The citation's anchor is a character range over `extractions.text_content`,
 * and that range is exact — it is what `resolveCitationTarget` verifies the
 * source against. But the pane a reader lands on does not show that raw text:
 * `OcrRichText` puts it through `renderOcrHtml`, which rewrites it into HTML.
 * The extraction also contains markup of its own, so an offset into the raw
 * text corresponds to no position in the rendered DOM.
 *
 * Mapping one to the other would be a second translation layer of exactly the
 * kind the G3 entry of Unit 4 declined to invent. Searching the rendered output
 * for the quoted text instead is exact, needs no mapping, and survives whatever
 * the renderer did — and when the fragment cannot be found, that is reported
 * rather than guessed at.
 *
 * The offsets keep their job: they are the durable anchor and the thing the
 * integrity check compares. This is only about pointing at it on screen.
 */

/** Marks the first occurrence and scrolls to it. Reports whether it was found. */
export function highlightFragment(
  container: HTMLElement | null,
  fragment: string,
  markClass = 'citation-hit'
): boolean {
  if (!container) return false
  const needle = fragment.trim()
  if (!needle) return false

  clearHighlight(container, markClass)

  const walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT)
  const nodes: Text[] = []
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    nodes.push(node as Text)
  }

  // A fragment that sits inside one text node is the common case and the only
  // one that can be wrapped without restructuring the rendered markup.
  for (const node of nodes) {
    const at = (node.textContent ?? '').indexOf(needle)
    if (at === -1) continue
    return wrap(node, at, at + needle.length, markClass)
  }

  // It spans elements, so there is no single node to wrap. Rather than
  // rebuilding the renderer's output, the first node holding the start of the
  // fragment is marked and scrolled to: the reader is put in the right place
  // without the page being rewritten underneath them.
  const partial = longestLeadingMatch(nodes, needle)
  if (!partial) return false
  return wrap(partial.node, partial.start, partial.end, markClass)
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

function wrap(node: Text, start: number, end: number, markClass: string): boolean {
  const range = document.createRange()
  range.setStart(node, start)
  range.setEnd(node, end)
  const mark = document.createElement('mark')
  mark.className = markClass
  try {
    range.surroundContents(mark)
  } catch {
    // `surroundContents` refuses a range that partially selects a node. The
    // reader is still taken to the right place, just without the mark.
    node.parentElement?.scrollIntoView({ block: 'center' })
    return false
  }
  mark.scrollIntoView({ block: 'center' })
  return true
}

/**
 * The longest run of the fragment's opening that lives in one text node.
 *
 * Used when the fragment crosses element boundaries. Requires a decent run so a
 * single shared word does not send the reader to the wrong paragraph.
 */
const MIN_PARTIAL = 12

function longestLeadingMatch(
  nodes: Text[],
  needle: string
): { node: Text; start: number; end: number } | null {
  for (let length = needle.length - 1; length >= MIN_PARTIAL; length -= 1) {
    const prefix = needle.slice(0, length)
    for (const node of nodes) {
      const at = (node.textContent ?? '').indexOf(prefix)
      if (at !== -1) return { node, start: at, end: at + prefix.length }
    }
  }
  return null
}
