import { beforeEach, describe, expect, it, vi } from 'vitest'
import { clearHighlight, highlightCitationRange } from './highlight-fragment'

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

const raw = 'Antes **molino** y *viento*.\n\nSegundo párrafo termina.'
const html =
  '<p>Antes <strong>molino</strong> y <em>viento</em>.</p><p>Segundo párrafo termina.</p>'

describe('highlightCitationRange', () => {
  it('marks every rendered text-node segment covered by a raw citation range', () => {
    const container = render(html)
    const end = raw.indexOf('párrafo') + 'párrafo'.length

    expect(highlightCitationRange(container, raw, { start: 0, end })).toBe(true)

    expect(
      [...container.querySelectorAll('mark.citation-hit')].map((mark) => mark.textContent)
    ).toEqual(['Antes ', 'molino', ' y ', 'viento', '.', 'Segundo párrafo'])
  })

  it('preserves formatting elements and the complete rendered text', () => {
    const container = render(html)
    const text = container.textContent

    highlightCitationRange(container, raw, { start: 0, end: raw.length })

    expect(container.textContent).toBe(text)
    expect(container.querySelector('strong')?.textContent).toBe('molino')
    expect(container.querySelector('em')?.textContent).toBe('viento')
    expect(container.querySelector('strong mark.citation-hit')).not.toBeNull()
    expect(container.querySelector('em mark.citation-hit')).not.toBeNull()
  })

  it('scrolls only the first mark in document order to the center', () => {
    const container = render(html)

    highlightCitationRange(container, raw, { start: 0, end: raw.length })

    const scrollIntoView = vi.mocked(Element.prototype.scrollIntoView)
    const marks = container.querySelectorAll('mark.citation-hit')
    expect(scrollIntoView).toHaveBeenCalledTimes(1)
    expect(scrollIntoView).toHaveBeenCalledWith({ block: 'center' })
    expect(scrollIntoView.mock.instances[0]).toBe(marks[0])
  })

  it('selects repeated wording by its raw offsets', () => {
    const repeatedRaw = 'molino primero.\n\nmolino segundo.'
    const container = render('<p>molino primero.</p><p>molino segundo.</p>')
    const start = repeatedRaw.lastIndexOf('molino')

    expect(
      highlightCitationRange(container, repeatedRaw, {
        start,
        end: start + 'molino'.length,
      })
    ).toBe(true)

    expect(container.querySelectorAll('mark.citation-hit')).toHaveLength(1)
    expect(container.querySelectorAll('p')[0]?.querySelector('mark')).toBeNull()
    expect(container.querySelectorAll('p')[1]?.querySelector('mark')?.textContent).toBe('molino')
  })

  it('clears the previous citation before marking a second one', () => {
    const repeatedRaw = 'molino primero.\n\nviento segundo.'
    const container = render('<p>molino primero.</p><p>viento segundo.</p>')
    const molino = repeatedRaw.indexOf('molino')
    const viento = repeatedRaw.indexOf('viento')

    highlightCitationRange(container, repeatedRaw, {
      start: molino,
      end: molino + 'molino'.length,
    })
    highlightCitationRange(container, repeatedRaw, {
      start: viento,
      end: viento + 'viento'.length,
    })

    const marks = container.querySelectorAll('mark.citation-hit')
    expect(marks).toHaveLength(1)
    expect(marks[0]?.textContent).toBe('viento')
  })

  it('returns false and leaves no mark when the raw range cannot be mapped', () => {
    const container = render('<p>texto visible</p>')
    const validRaw = 'texto visible'

    highlightCitationRange(container, validRaw, { start: 0, end: 5 })

    expect(highlightCitationRange(container, 'contenido diferente', { start: 0, end: 9 })).toBe(
      false
    )
    expect(container.querySelector('mark.citation-hit')).toBeNull()
  })

  it('returns false for a null container', () => {
    expect(highlightCitationRange(null, 'texto', { start: 0, end: 5 })).toBe(false)
  })
})

describe('clearHighlight', () => {
  it('restores the original DOM after segmented marking', () => {
    const container = render(html)

    highlightCitationRange(container, raw, { start: 0, end: raw.length })
    clearHighlight(container)

    expect(container.querySelector('mark.citation-hit')).toBeNull()
    expect(container.innerHTML).toBe(html)
  })
})
