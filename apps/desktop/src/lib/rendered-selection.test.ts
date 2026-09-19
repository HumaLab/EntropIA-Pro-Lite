import { describe, expect, it } from 'vitest'
import { mapRenderedText } from './rendered-text-map'
import { highlightRanges, renderedSelection } from './rendered-selection'

// The raw extraction, and the DOM the OCR renderer makes of it.
const RAW = '# Convenio Laboral\n\nLa **aplicación** del convenio'
const HTML = '<h1>Convenio Laboral</h1>\n<p>La <strong>aplicación</strong> del convenio</p>\n'

function mount() {
  const container = document.createElement('div')
  container.innerHTML = HTML
  document.body.append(container)
  const map = mapRenderedText(container.textContent ?? '', RAW)
  return { container, map }
}

/** The text node holding `text`, and where `text` starts in it. */
function at(container: HTMLElement, text: string): [Text, number] {
  const walker = document.createTreeWalker(container, NodeFilter.SHOW_TEXT)
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const index = (node as Text).data.indexOf(text)
    if (index >= 0) return [node as Text, index]
  }
  throw new Error(`no text node holds "${text}"`)
}

function range(start: [Node, number], end: [Node, number]): Range {
  const selected = document.createRange()
  selected.setStart(...start)
  selected.setEnd(...end)
  return selected
}

describe('reading a selection on rendered OCR', () => {
  it('anchors a selection that crosses a heading, a paragraph and a bold word', () => {
    const { container, map } = mount()
    const [heading, laboral] = at(container, 'Laboral')
    const [bold, aplicacion] = at(container, 'aplicación')

    const chosen = renderedSelection(
      container,
      range([heading, laboral], [bold, aplicacion + 'aplicación'.length]),
      map
    )

    expect(chosen).toEqual({
      start: RAW.indexOf('Laboral'),
      end: RAW.indexOf('aplicación') + 'aplicación'.length,
      quote: 'Laboral\n\nLa aplicación',
    })
  })

  it('keeps the page layout in the quote: a blank line between blocks, a break where one was', () => {
    const container = document.createElement('div')
    container.innerHTML =
      '<h1>SOLICITADA</h1>\n<p>A mis compañeros<br>\nobreros del pescado:</p>\n<p>Después   de dos años</p>\n'
    document.body.append(container)
    const map = mapRenderedText(
      container.textContent ?? '',
      '# SOLICITADA\n\nA mis compañeros\nobreros del pescado:\n\nDespués   de dos años'
    )
    const [first] = at(container, 'SOLICITADA')
    const [last, index] = at(container, 'dos años')

    const chosen = renderedSelection(container, range([first, 0], [last, index + 8]), map)

    expect(chosen && chosen !== 'unmapped' ? chosen.quote : null).toBe(
      'SOLICITADA\n\nA mis compañeros\nobreros del pescado:\n\nDespués de dos años'
    )
  })

  it('quotes what the reader saw, from the middle of one block to the next', () => {
    const { container, map } = mount()
    const [heading, convenio] = at(container, 'Convenio')
    const [paragraph] = at(container, 'La ')

    const chosen = renderedSelection(container, range([heading, convenio], [paragraph, 2]), map)

    expect(chosen).not.toBe('unmapped')
    expect(chosen && chosen !== 'unmapped' ? chosen.quote : null).toBe('Convenio Laboral\n\nLa')
  })

  it('says so when the selection cannot be anchored, rather than guessing', () => {
    const container = document.createElement('div')
    container.innerHTML = '<p>texto que el crudo no tiene</p>'
    document.body.append(container)
    const map = mapRenderedText(container.textContent ?? '', 'otra cosa')
    const [node] = at(container, 'texto')

    expect(renderedSelection(container, range([node, 0], [node, 5]), map)).toBe('unmapped')
  })

  it('ignores an empty selection and one outside the text', () => {
    const { container, map } = mount()
    const [node, index] = at(container, 'Laboral')
    const outside = document.createElement('p')
    outside.textContent = 'fuera'
    document.body.append(outside)

    expect(renderedSelection(container, range([node, index], [node, index]), map)).toBeNull()
    expect(
      renderedSelection(container, range([outside.firstChild!, 0], [outside.firstChild!, 5]), map)
    ).toBeNull()
    expect(renderedSelection(container, null, map)).toBeNull()
  })
})

describe('marking words in rendered OCR', () => {
  it('finds a word inside formatting, ignoring case and accents', () => {
    const { container } = mount()

    const marks = highlightRanges(container, ['APLICACION', 'convenio'])

    expect(marks.map((mark) => mark.toString())).toEqual(['Convenio', 'aplicación', 'convenio'])
  })
})
