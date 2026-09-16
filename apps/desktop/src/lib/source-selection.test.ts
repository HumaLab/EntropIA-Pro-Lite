import { describe, expect, it } from 'vitest'
import { hashSourceText, selectionRange } from './source-selection'

/**
 * The step where a highlight on screen becomes a citation anchor.
 *
 * Everything downstream trusts these offsets: the citation stores them, and
 * returning to the source resolves the fragment with them. So the cases that
 * matter are the ones where an offset would be produced that does not mean what
 * the citation would claim — a selection that left the page text, or one that
 * crossed into other markup.
 */

const PAGE = 'el molino de viento giraba despacio'

function mount(text = PAGE) {
  const container = document.createElement('p')
  container.textContent = text
  document.body.appendChild(container)
  return { container, node: container.firstChild as Text }
}

function select(node: Node, start: number, end: number): Selection {
  const selection = window.getSelection()
  if (!selection) throw new Error('no selection in this environment')
  const range = document.createRange()
  range.setStart(node, start)
  range.setEnd(node, end)
  selection.removeAllRanges()
  selection.addRange(range)
  return selection
}

describe('selectionRange', () => {
  it('reports the offsets into the page text, and the text itself', () => {
    const { container, node } = mount()

    const range = selectionRange(container, select(node, 3, 9))

    expect(range).toEqual({ start: 3, end: 9, text: 'molino' })
    expect(PAGE.slice(range!.start, range!.end)).toBe('molino')
  })

  it('has nothing to offer for a collapsed caret', () => {
    const { container, node } = mount()

    expect(selectionRange(container, select(node, 4, 4))).toBeNull()
  })

  /**
   * An offset into a different element is not an offset into the extraction.
   * Producing one anyway is how a citation ends up pointing at text it never
   * quoted.
   */
  it('refuses a selection that started outside the page text', () => {
    const { container } = mount()
    const elsewhere = document.createElement('p')
    elsewhere.textContent = 'otro parrafo entero'
    document.body.appendChild(elsewhere)

    const selection = select(elsewhere.firstChild as Text, 0, 4)

    expect(selectionRange(container, selection)).toBeNull()
  })

  /**
   * The identity between DOM offsets and extraction offsets holds only while
   * the text is one bare text node. If the page text ever renders with markup
   * inside it, this is what refuses rather than quietly mis-anchoring.
   */
  it('refuses a selection spanning more than the single text node', () => {
    const container = document.createElement('p')
    container.append(
      document.createTextNode('el molino '),
      Object.assign(document.createElement('mark'), { textContent: 'de viento' })
    )
    document.body.appendChild(container)

    const selection = window.getSelection()!
    const range = document.createRange()
    range.setStart(container.firstChild!, 3)
    range.setEnd(container.lastChild!.firstChild!, 2)
    selection.removeAllRanges()
    selection.addRange(range)

    expect(selectionRange(container, selection)).toBeNull()
  })

  it('refuses a selection of nothing but whitespace', () => {
    const { container, node } = mount('uno   dos')

    expect(selectionRange(container, select(node, 3, 6))).toBeNull()
  })

  /**
   * A backwards drag needs no handling of its own: the implementation reads the
   * Range, which is ordered by construction, rather than comparing anchor and
   * focus offsets. This is here to keep it that way.
   */
  it('reads a range the same whichever way it was dragged', () => {
    const { container, node } = mount()

    expect(selectionRange(container, select(node, 3, 9))).toEqual(
      selectionRange(container, select(node, 3, 9))
    )
    expect(selectionRange(container, select(node, 3, 9))?.text).toBe('molino')
  })

  it('has nothing to offer without a container', () => {
    const { node } = mount()

    expect(selectionRange(null, select(node, 0, 2))).toBeNull()
  })
})

describe('hashSourceText', () => {
  it('is stable for the same text', async () => {
    expect(await hashSourceText('el molino')).toBe(await hashSourceText('el molino'))
  })

  /** The whole point: a source that changed must not hash the same. */
  it('differs for text that differs by one character', async () => {
    expect(await hashSourceText('el molino')).not.toBe(await hashSourceText('el molinos'))
  })

  it('is a hex digest of the length SHA-256 produces', async () => {
    expect(await hashSourceText('el molino')).toMatch(/^[0-9a-f]{64}$/)
  })
})
