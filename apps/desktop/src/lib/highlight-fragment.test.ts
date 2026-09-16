import { beforeEach, describe, expect, it, vi } from 'vitest'
import { clearHighlight, highlightFragment } from './highlight-fragment'

/**
 * Pointing at a cited fragment in rendered OCR (plan-editor.md §10.2 step 4).
 *
 * The fragment is found by its text, not by the citation's stored offsets: the
 * pane shows HTML that `renderOcrHtml` produced from the raw extraction, so an
 * offset into that raw text names no position here. What matters is that the
 * mark lands on the fragment, that the rendered markup is not rewritten around
 * it, and that "not found" is reported rather than guessed.
 */

beforeEach(() => {
  document.body.innerHTML = ''
  // jsdom has no layout and therefore no scrollIntoView.
  Element.prototype.scrollIntoView = vi.fn()
})

function render(html: string): HTMLElement {
  const container = document.createElement('div')
  container.innerHTML = html
  document.body.appendChild(container)
  return container
}

describe('highlightFragment', () => {
  it('marks the fragment where it sits', () => {
    const container = render('<p>el molino de viento giraba despacio</p>')

    expect(highlightFragment(container, 'molino de viento')).toBe(true)

    const mark = container.querySelector('mark.citation-hit')
    expect(mark?.textContent).toBe('molino de viento')
  })

  it('leaves the rest of the rendered markup exactly as it was', () => {
    const container = render('<p>antes <em>enfasis</em> el molino despues</p>')

    highlightFragment(container, 'el molino')

    expect(container.querySelector('em')?.textContent).toBe('enfasis')
    expect(container.textContent).toBe('antes enfasis el molino despues')
  })

  it('marks only the first occurrence', () => {
    const container = render('<p>molino y molino</p>')

    highlightFragment(container, 'molino')

    expect(container.querySelectorAll('mark.citation-hit')).toHaveLength(1)
  })

  it('finds a fragment in a later paragraph', () => {
    const container = render('<p>uno</p><p>dos</p><p>el molino de viento</p>')

    expect(highlightFragment(container, 'el molino de viento')).toBe(true)
    expect(container.querySelectorAll('p')[2]?.querySelector('mark')).not.toBeNull()
  })

  /**
   * A fragment broken by the renderer's own markup has no single node to wrap.
   * The reader is still put in the right paragraph rather than left where they
   * were, but only on a run long enough that a shared word cannot mislead.
   */
  it('falls back to the opening run when the fragment crosses elements', () => {
    const container = render('<p>el molino de viento <em>giraba despacio</em> sobre la loma</p>')

    expect(highlightFragment(container, 'el molino de viento giraba despacio')).toBe(true)
    expect(container.querySelector('mark.citation-hit')?.textContent).toContain('el molino de')
  })

  it('reports not finding a fragment that is not there', () => {
    const container = render('<p>otra cosa completamente</p>')

    expect(highlightFragment(container, 'el molino de viento')).toBe(false)
    expect(container.querySelector('mark.citation-hit')).toBeNull()
  })

  /** A single shared word must not send the reader to the wrong paragraph. */
  it('does not settle for a run too short to mean anything', () => {
    const container = render('<p>de</p>')

    expect(highlightFragment(container, 'de viento giraba despacio')).toBe(false)
  })

  it('has nothing to do without a fragment or a container', () => {
    const container = render('<p>texto</p>')

    expect(highlightFragment(container, '   ')).toBe(false)
    expect(highlightFragment(null, 'texto')).toBe(false)
  })
})

describe('clearHighlight', () => {
  it('puts the text back as it was, so two visits do not stack', () => {
    const container = render('<p>el molino de viento</p>')
    highlightFragment(container, 'molino')

    clearHighlight(container)

    expect(container.querySelector('mark.citation-hit')).toBeNull()
    expect(container.innerHTML).toBe('<p>el molino de viento</p>')
  })

  it('is what a second call relies on', () => {
    const container = render('<p>el molino y el viento</p>')
    highlightFragment(container, 'molino')

    highlightFragment(container, 'viento')

    const marks = container.querySelectorAll('mark.citation-hit')
    expect(marks).toHaveLength(1)
    expect(marks[0]?.textContent).toBe('viento')
  })
})
