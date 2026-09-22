import { fireEvent, render, screen, within } from '@testing-library/svelte'
import { tick } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import WritingEditor from './WritingEditor.svelte'
import { WRITING_SCHEMA_VERSION, type CanonicalDocument } from './document-contract'

/**
 * The toolbar is two rows: the original tools on the first, the formatting
 * tools on the second. Each row keeps to one line on its own: groups that do
 * not fit collapse, least used first, into that row's "more tools" menu.
 *
 * There is no layout engine here, so the geometry is supplied: every button is
 * 28px, a group is as wide as its buttons plus the gaps between them, and each
 * row is as wide as the test says. The observer is a fake the test fires.
 */
const BUTTON = 28
let toolbarWidth = 0
let observers: FakeResizeObserver[] = []

class FakeResizeObserver {
  constructor(private readonly callback: ResizeObserverCallback) {
    observers.push(this)
  }
  observe() {}
  unobserve() {}
  disconnect() {
    observers = observers.filter((observer) => observer !== this)
  }
  fire() {
    this.callback([], this as unknown as ResizeObserver)
  }
}

function box(width: number): DOMRect {
  return {
    x: 0,
    y: 0,
    top: 0,
    left: 0,
    bottom: BUTTON,
    right: width,
    width,
    height: BUTTON,
    toJSON() {},
  } as DOMRect
}

function widthOf(element: HTMLElement): number {
  if (element.classList.contains('writing-editor__sep')) return 1
  if (element.dataset.toolbarGroup !== undefined || 'menuTrigger' in element.dataset) {
    const buttons = element.querySelectorAll('button').length
    return buttons * BUTTON + Math.max(0, buttons - 1) * 4
  }
  return BUTTON
}

/** TipTap hands the focus back to the text a frame after a command. */
const nextFrame = () => new Promise((resolve) => requestAnimationFrame(resolve))

async function resizeTo(width: number) {
  toolbarWidth = width
  for (const observer of [...observers]) observer.fire()
  await tick()
  await tick()
}

function manuscript(text: string): CanonicalDocument {
  return {
    schemaVersion: WRITING_SCHEMA_VERSION,
    doc: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text }] }] },
  }
}

const surface = () => document.querySelector('.writing-editor__surface') as HTMLElement
const toolbar = () => screen.getByRole('toolbar')
type Row = 'first' | 'second'
const rowElement = (row: Row) =>
  toolbar().querySelector<HTMLElement>(`[data-toolbar-row="${row}"]`)!
const moreTools = (row: Row = 'first') =>
  within(rowElement(row)).queryByRole('button', { name: 'Más herramientas' })
const menu = () => screen.queryByRole('menu', { name: 'Más herramientas' })

/** The open overflow menu's entries. A swatch has no text: it is named by its label. */
function menuItems(): (string | null | undefined)[][] {
  return [...menu()!.querySelectorAll('[role^="menuitem"]')].map((item) => [
    item.getAttribute('role'),
    item.getAttribute('aria-label') ?? item.textContent?.trim(),
  ])
}

/** A row as a reader would scan it: button names, and `|` for a separator. */
function rowOf(row: Row = 'first'): string[] {
  return [...rowElement(row).querySelectorAll<HTMLElement>('button, .writing-editor__sep')].map(
    (element) => (element.tagName === 'BUTTON' ? (element.getAttribute('aria-label') ?? '?') : '|')
  )
}

/** The toolbar as it was before the formatting tools: the first row, whole. */
const FIRST_ROW = [
  'Deshacer',
  'Rehacer',
  '|',
  'Negrita',
  'Cursiva',
  'Subrayado',
  'Tachado',
  'Código',
  '|',
  'Título 1',
  'Título 2',
  'Título 3',
  '|',
  'Lista',
  'Lista ordenada',
  'Cita en bloque',
  '|',
  'Enlace',
  'Insertar tabla',
  'Nota al pie',
  'Buscar',
  '|',
  'Iniciar dictado',
]

/** The formatting tools: typography, then paragraph. */
const SECOND_ROW = [
  'Aumentar tamaño de fuente',
  'Disminuir tamaño de fuente',
  'Cambiar mayúsculas y minúsculas',
  'Subíndice',
  'Superíndice',
  'Borrar formato',
  'Color de resaltado',
  'Color de texto',
  '|',
  'Disminuir sangría',
  'Aumentar sangría',
  'Alinear a la izquierda',
  'Centrar',
  'Alinear a la derecha',
  'Justificar',
  'Interlineado',
]

/** The paragraph group as the overflow lists it, with the caret in a plain paragraph. */
const PARAGRAPH_ITEMS = [
  ['menuitem', 'Disminuir sangría'],
  ['menuitem', 'Aumentar sangría'],
  ['menuitemradio', 'Alinear a la izquierda'],
  ['menuitemradio', 'Centrar'],
  ['menuitemradio', 'Alinear a la derecha'],
  ['menuitemradio', 'Justificar'],
  ['menuitemradio', '1'],
  ['menuitemradio', '1,15'],
  ['menuitemradio', '1,5'],
  ['menuitemradio', '2'],
  ['menuitemradio', 'Predeterminado'],
]

/** A colour menu as the overflow lists it: no colour, then the eight swatches. */
const COLOUR_NAMES = ['Gris', 'Rojo', 'Naranja', 'Amarillo', 'Verde', 'Azul', 'Violeta', 'Rosa']
const PALETTE_ITEMS = [
  ['menuitemradio', 'Sin color'],
  ...COLOUR_NAMES.map((name) => ['menuitemradio', name]),
]

beforeEach(() => {
  observers = []
  toolbarWidth = 0
  vi.stubGlobal('ResizeObserver', FakeResizeObserver)
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (
    this: HTMLElement
  ) {
    return box(widthOf(this))
  })
  vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockImplementation(function (
    this: HTMLElement
  ) {
    return this.dataset.toolbarRow !== undefined ? toolbarWidth : 0
  })
})

afterEach(() => {
  vi.unstubAllGlobals()
  vi.restoreAllMocks()
})

function renderEditor(content: string | CanonicalDocument = 'Hola mundo') {
  const document = typeof content === 'string' ? manuscript(content) : content
  return render(WritingEditor, { props: { document, ondictate: vi.fn() } })
}

describe('WritingEditor toolbar: two rows', () => {
  it('is one toolbar holding the two rows, the original tools first', () => {
    renderEditor()

    expect(screen.getAllByRole('toolbar')).toHaveLength(1)
    const rows = [...toolbar().querySelectorAll<HTMLElement>('[data-toolbar-row]')]
    expect(rows.map((row) => row.dataset.toolbarRow)).toEqual(['first', 'second'])
  })

  it('puts none of the formatting tools on the first row, nor the reverse', async () => {
    renderEditor()
    await resizeTo(2000)

    const tools = (row: string[]) => row.filter((name) => name !== '|')
    for (const name of tools(SECOND_ROW)) expect(rowOf('first')).not.toContain(name)
    for (const name of tools(FIRST_ROW)) expect(rowOf('second')).not.toContain(name)
  })
})

describe('WritingEditor toolbar: when everything fits', () => {
  it('is the original row, then the formatting row, with no overflow button', async () => {
    renderEditor()
    await resizeTo(2000)

    expect(rowOf('first')).toEqual(FIRST_ROW)
    expect(rowOf('second')).toEqual(SECOND_ROW)
    expect(moreTools('first')).toBeNull()
    expect(moreTools('second')).toBeNull()
  })
})

describe('WritingEditor toolbar: when it does not fit', () => {
  it('collapses whole groups on each row, least used first, keeping the rest in order', async () => {
    renderEditor()
    await resizeTo(400)

    expect(rowOf('first')).toEqual([
      'Deshacer',
      'Rehacer',
      '|',
      'Negrita',
      'Cursiva',
      'Subrayado',
      '|',
      'Título 1',
      'Título 2',
      'Título 3',
      '|',
      'Más herramientas',
      'Buscar',
      '|',
      'Iniciar dictado',
    ])
    // The second row's own menu goes last: it has no find or microphone to
    // keep the end.
    expect(rowOf('second')).toEqual([
      'Disminuir sangría',
      'Aumentar sangría',
      'Alinear a la izquierda',
      'Centrar',
      'Alinear a la derecha',
      'Justificar',
      'Interlineado',
      '|',
      'Más herramientas',
    ])
  })

  it('fits each row on its own: a row that fits shows no menu', async () => {
    renderEditor()
    // The first row needs 624px, the second 489px.
    await resizeTo(600)

    expect(moreTools('first')).not.toBeNull()
    expect(rowOf('second')).toEqual(SECOND_ROW)
    expect(moreTools('second')).toBeNull()
  })

  it('keeps the microphone last and find just before it', async () => {
    renderEditor()
    await resizeTo(300)

    const names = rowOf().filter((name) => name !== '|')
    expect(names.at(-1)).toBe('Iniciar dictado')
    expect(names.at(-2)).toBe('Buscar')
    expect(names.at(-3)).toBe('Más herramientas')
  })

  it('lists the first row’s collapsed tools in its menu, in toolbar order, with their state', async () => {
    renderEditor()
    await resizeTo(400)

    await fireEvent.click(moreTools('first')!)
    await tick()

    expect(menuItems()).toEqual([
      ['menuitemcheckbox', 'Tachado'],
      ['menuitemcheckbox', 'Código'],
      ['menuitemcheckbox', 'Lista'],
      ['menuitemcheckbox', 'Lista ordenada'],
      ['menuitemcheckbox', 'Cita en bloque'],
      ['menuitemcheckbox', 'Enlace'],
      ['menuitem', 'Insertar tabla'],
      ['menuitem', 'Nota al pie'],
    ])
  })

  it('lists the second row’s collapsed tools in its own menu, palettes drawn whole', async () => {
    renderEditor()
    await resizeTo(250)
    expect(rowOf('second')).toEqual(['Más herramientas'])

    await fireEvent.click(moreTools('second')!)
    await tick()

    expect(menuItems()).toEqual([
      ['menuitem', 'Aumentar tamaño de fuente'],
      ['menuitem', 'Disminuir tamaño de fuente'],
      ['menuitem', 'MAYÚSCULAS'],
      ['menuitem', 'minúsculas'],
      ['menuitem', 'Tipo oración'],
      ['menuitem', 'Capitalizar palabras'],
      ['menuitemcheckbox', 'Subíndice'],
      ['menuitemcheckbox', 'Superíndice'],
      ['menuitem', 'Borrar formato'],
      ...PALETTE_ITEMS,
      ...PALETTE_ITEMS,
      ...PARAGRAPH_ITEMS,
    ])
  })

  it('brings the groups back, and drops the button, when the room returns', async () => {
    renderEditor()
    await resizeTo(250)
    await resizeTo(2000)

    expect(rowOf('first')).toEqual(FIRST_ROW)
    expect(rowOf('second')).toEqual(SECOND_ROW)
  })
})

describe('WritingEditor toolbar: the overflow menu', () => {
  it('runs the same command as the button, on the selection, and leaves the focus in the text', async () => {
    const { component } = renderEditor('Hola mundo')
    await resizeTo(400)
    surface().focus()
    await fireEvent.keyDown(surface(), { key: 'a', ctrlKey: true })
    expect(component.selectedText()).toBe('Hola mundo')

    await fireEvent.click(moreTools()!)
    await tick()
    await fireEvent.click(screen.getByRole('menuitemcheckbox', { name: 'Tachado' }))
    await tick()
    await nextFrame()

    expect(surface().querySelector('s')).toHaveTextContent('Hola mundo')
    expect(component.selectedText()).toBe('Hola mundo')
    expect(surface().contains(document.activeElement)).toBe(true)
    expect(menu()).toBeNull()
  })

  it('shows a toggle that is on as checked', async () => {
    renderEditor('Hola mundo')
    await resizeTo(400)
    surface().focus()

    await fireEvent.click(moreTools()!)
    await tick()
    await fireEvent.click(screen.getByRole('menuitemcheckbox', { name: 'Cita en bloque' }))
    await tick()
    expect(surface().querySelector('blockquote')).toHaveTextContent('Hola mundo')

    await fireEvent.click(moreTools()!)
    await tick()
    expect(screen.getByRole('menuitemcheckbox', { name: 'Cita en bloque' })).toHaveAttribute(
      'aria-checked',
      'true'
    )
  })

  it('keeps a disabled tool disabled', async () => {
    renderEditor('Hola mundo')
    await resizeTo(400)
    surface().focus()

    await fireEvent.click(moreTools()!)
    await tick()
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Insertar tabla' }))
    await tick()

    // The caret is inside the new table now, where another cannot go.
    await fireEvent.click(moreTools()!)
    await tick()
    expect(screen.getByRole('menuitem', { name: 'Insertar tabla' })).toBeDisabled()
  })

  it('closes on Escape and gives the focus back to its button', async () => {
    renderEditor()
    await resizeTo(400)

    await fireEvent.click(moreTools()!)
    await tick()
    await fireEvent.keyDown(document.activeElement!, { key: 'Escape' })
    await tick()

    expect(menu()).toBeNull()
    expect(document.activeElement).toBe(moreTools())
  })
})

describe('WritingEditor toolbar: tooltips', () => {
  it('gives every button a tooltip from the shared system and none a native title', async () => {
    renderEditor()
    await resizeTo(400)

    const buttons = [...toolbar().querySelectorAll('button')]
    expect(buttons.length).toBeGreaterThan(5)
    for (const button of buttons) {
      expect(button.getAttribute('data-tooltip'), button.outerHTML).toBe(
        button.getAttribute('aria-label')
      )
      expect(button.hasAttribute('title'), button.outerHTML).toBe(false)
    }
    expect(moreTools('first')).toHaveAttribute('data-tooltip', 'Más herramientas')
    expect(moreTools('second')).toHaveAttribute('data-tooltip', 'Más herramientas')
  })

  it('takes the overflow label from the labels it is given', async () => {
    render(WritingEditor, {
      props: { document: manuscript('Hola'), labels: { moreTools: 'More tools' } },
    })
    await resizeTo(300)

    const triggers = within(toolbar()).getAllByRole('button', { name: 'More tools' })
    expect(triggers).toHaveLength(2)
    for (const trigger of triggers) expect(trigger).toHaveAttribute('data-tooltip', 'More tools')
  })
})

/** A one-paragraph manuscript whose single run carries these marks. */
function marked(text: string, marks: { type: string; attrs?: Record<string, unknown> }[]) {
  return {
    schemaVersion: WRITING_SCHEMA_VERSION,
    doc: {
      type: 'doc',
      content: [{ type: 'paragraph', content: [{ type: 'text', text, marks }] }],
    },
  }
}

describe('WritingEditor toolbar: typography', () => {
  const button = (name: string) => within(toolbar()).getByRole('button', { name })
  const caseMenu = () => screen.queryByRole('menu', { name: 'Cambiar mayúsculas y minúsculas' })

  async function selectAll(component: { selectedText(): string }) {
    surface().focus()
    await fireEvent.keyDown(surface(), { key: 'a', ctrlKey: true })
    await tick()
    expect(component.selectedText()).not.toBe('')
  }

  it('is the first group of its row to give way, before paragraph', async () => {
    renderEditor()
    await resizeTo(480)

    const row = rowOf('second')
    expect(row).not.toContain('Aumentar tamaño de fuente')
    expect(row).toContain('Centrar')
    expect(row.slice(-3)).toEqual(['Interlineado', '|', 'Más herramientas'])
  })

  it('gives every typography button its tooltip', async () => {
    renderEditor()
    await resizeTo(2000)

    const tools = SECOND_ROW.slice(0, SECOND_ROW.indexOf('Color de texto') + 1)
    expect(tools).toHaveLength(8)
    for (const name of tools) {
      expect(button(name)).toHaveAttribute('data-tooltip', name)
      expect(button(name).hasAttribute('title')).toBe(false)
    }
  })

  it('enlarges the selection, keeping the selection and the focus in the text', async () => {
    const { component } = renderEditor('Hola mundo')
    await resizeTo(2000)
    await selectAll(component)

    await fireEvent.click(button('Aumentar tamaño de fuente'))
    await tick()
    await nextFrame()

    expect((surface().querySelector('span[style]') as HTMLElement).style.fontSize).toBe('1.125em')
    expect(component.selectedText()).toBe('Hola mundo')
    expect(surface().contains(document.activeElement)).toBe(true)
  })

  it('disables A+ at the top of the scale', async () => {
    const { component } = renderEditor(
      marked('enorme', [{ type: 'textStyle', attrs: { fontSize: '2em' } }])
    )
    await resizeTo(2000)
    await selectAll(component)

    expect(button('Aumentar tamaño de fuente')).toBeDisabled()
    expect(button('Disminuir tamaño de fuente')).toBeEnabled()
  })

  it('shows subscript and superscript as pressed, one at a time', async () => {
    const { component } = renderEditor('x2')
    await resizeTo(2000)
    await selectAll(component)

    await fireEvent.click(button('Subíndice'))
    await tick()
    expect(button('Subíndice')).toHaveAttribute('aria-pressed', 'true')
    expect(surface().querySelector('sub')).toHaveTextContent('x2')

    await fireEvent.click(button('Superíndice'))
    await tick()
    expect(button('Superíndice')).toHaveAttribute('aria-pressed', 'true')
    expect(button('Subíndice')).not.toHaveAttribute('aria-pressed', 'true')
    expect(surface().querySelector('sub')).toBeNull()
  })

  it('clears the formatting of the selection and keeps its link', async () => {
    const { component } = renderEditor(
      marked('el sitio', [{ type: 'bold' }, { type: 'link', attrs: { href: 'https://e.org' } }])
    )
    await resizeTo(2000)
    await selectAll(component)

    await fireEvent.click(button('Borrar formato'))
    await tick()

    expect(surface().querySelector('strong')).toBeNull()
    expect(surface().querySelector('a')).toHaveTextContent('el sitio')
  })

  it('disables change case when nothing is selected', async () => {
    renderEditor('hola')
    await resizeTo(2000)

    expect(button('Cambiar mayúsculas y minúsculas')).toBeDisabled()
  })

  it('changes case from its menu, keeping the selection and the focus in the text', async () => {
    const { component } = renderEditor('hola mundo')
    await resizeTo(2000)
    await selectAll(component)

    await fireEvent.click(button('Cambiar mayúsculas y minúsculas'))
    await tick()
    const items = [...caseMenu()!.querySelectorAll('[role="menuitem"]')].map((item) =>
      item.textContent?.trim()
    )
    expect(items).toEqual(['MAYÚSCULAS', 'minúsculas', 'Tipo oración', 'Capitalizar palabras'])

    await fireEvent.click(screen.getByRole('menuitem', { name: 'MAYÚSCULAS' }))
    await tick()
    await nextFrame()

    expect(surface()).toHaveTextContent('HOLA MUNDO')
    expect(component.selectedText()).toBe('HOLA MUNDO')
    expect(surface().contains(document.activeElement)).toBe(true)
    expect(caseMenu()).toBeNull()
  })

  it('opens the case menu from the keyboard and applies with Enter', async () => {
    const { component } = renderEditor('HOLA MUNDO')
    await resizeTo(2000)
    await selectAll(component)

    const trigger = button('Cambiar mayúsculas y minúsculas')
    trigger.focus()
    await fireEvent.keyDown(trigger, { key: 'ArrowDown' })
    await tick()
    expect(document.activeElement).toBe(screen.getByRole('menuitem', { name: 'MAYÚSCULAS' }))

    await fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' })
    await fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' })
    expect(document.activeElement).toBe(screen.getByRole('menuitem', { name: 'Tipo oración' }))
    await fireEvent.keyDown(document.activeElement!, { key: 'Enter' })
    await tick()
    await nextFrame()

    expect(surface()).toHaveTextContent('Hola mundo')
    expect(surface().contains(document.activeElement)).toBe(true)
  })

  it('offers every typography tool in the overflow menu, working on the selection', async () => {
    const { component } = renderEditor('hola mundo')
    await resizeTo(400)
    await selectAll(component)

    await fireEvent.click(moreTools('second')!)
    await tick()
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Capitalizar palabras' }))
    await tick()
    await nextFrame()

    expect(surface()).toHaveTextContent('Hola Mundo')
    expect(component.selectedText()).toBe('Hola Mundo')
    expect(surface().contains(document.activeElement)).toBe(true)
  })
})

describe('WritingEditor toolbar: insert image', () => {
  it('has no insert-image button when nothing can pick a file', async () => {
    renderEditor()
    await resizeTo(2000)

    expect(screen.queryByRole('button', { name: 'Insertar imagen' })).not.toBeInTheDocument()
  })

  it('sits right after the footnote tool, and calls oninsertimage when clicked', async () => {
    const oninsertimage = vi.fn()
    render(WritingEditor, { props: { document: manuscript('Hola'), oninsertimage } })
    await resizeTo(2000)

    const row = rowOf('first')
    expect(row.indexOf('Insertar imagen')).toBe(row.indexOf('Nota al pie') + 1)

    await fireEvent.click(within(toolbar()).getByRole('button', { name: 'Insertar imagen' }))

    expect(oninsertimage).toHaveBeenCalledOnce()
  })

  it('exposes insertImage, which puts a writingImage node in the document', () => {
    const { component } = renderEditor()

    const ok = component.insertImage({ src: 'writing-images/abc.png', width: 10, height: 5 })

    expect(ok).toBe(true)
    expect(surface().querySelector('figure[data-writing-image]')).not.toBeNull()
  })
})

describe('WritingEditor toolbar: colours', () => {
  const button = (name: string) => within(toolbar()).getByRole('button', { name })
  const colourMenu = (name: string) => screen.queryByRole('menu', { name })
  const swatch = (name: string) => screen.getByRole('menuitemradio', { name })

  async function selectAll(component: { selectedText(): string }) {
    surface().focus()
    await fireEvent.keyDown(surface(), { key: 'a', ctrlKey: true })
    await tick()
    expect(component.selectedText()).not.toBe('')
  }

  async function open(name: string) {
    await fireEvent.click(button(name))
    await tick()
    expect(colourMenu(name)).not.toBeNull()
  }

  it('sits at the end of the typography group, highlight first', async () => {
    renderEditor()
    await resizeTo(2000)

    const row = rowOf('second')
    expect(
      row.slice(row.indexOf('Borrar formato'), row.indexOf('|', row.indexOf('Borrar formato')))
    ).toEqual(['Borrar formato', 'Color de resaltado', 'Color de texto'])
  })

  it('colours the selection, keeping the selection and the focus in the text', async () => {
    const { component } = renderEditor('Hola mundo')
    await resizeTo(2000)
    await selectAll(component)

    await open('Color de texto')
    await fireEvent.click(swatch('Rojo'))
    await tick()
    await nextFrame()

    const span = surface().querySelector('span[data-text-color]') as HTMLElement
    expect(span).toHaveTextContent('Hola mundo')
    expect(span.dataset.textColor).toBe('red')
    expect(component.selectedText()).toBe('Hola mundo')
    expect(surface().contains(document.activeElement)).toBe(true)
    expect(colourMenu('Color de texto')).toBeNull()
  })

  it('highlights the selection', async () => {
    const { component } = renderEditor('Hola mundo')
    await resizeTo(2000)
    await selectAll(component)

    await open('Color de resaltado')
    await fireEvent.click(swatch('Amarillo'))
    await tick()

    expect(surface().querySelector('mark[data-highlight="yellow"]')).toHaveTextContent('Hola mundo')
  })

  it('checks the colour the selection has, and shows the button as on', async () => {
    const { component } = renderEditor(
      marked('verde', [
        { type: 'textStyle', attrs: { color: 'green' } },
        { type: 'highlight', attrs: { color: 'blue' } },
      ])
    )
    await resizeTo(2000)
    await selectAll(component)

    expect(button('Color de texto')).toHaveClass('icon-button--active')
    expect(button('Color de resaltado')).toHaveClass('icon-button--active')

    await open('Color de texto')
    expect(swatch('Verde')).toHaveAttribute('aria-checked', 'true')
    expect(swatch('Sin color')).toHaveAttribute('aria-checked', 'false')
    expect(colourMenu('Color de texto')!.querySelectorAll('[aria-checked="true"]')).toHaveLength(1)
  })

  it('checks no colour, and leaves the button off, on text without one', async () => {
    const { component } = renderEditor('Hola')
    await resizeTo(2000)
    await selectAll(component)

    expect(button('Color de resaltado')).not.toHaveClass('icon-button--active')
    await open('Color de resaltado')
    expect(swatch('Sin color')).toHaveAttribute('aria-checked', 'true')
  })

  it('takes the colour off with "Sin color"', async () => {
    const { component } = renderEditor(
      marked('rojo', [{ type: 'highlight', attrs: { color: 'red' } }])
    )
    await resizeTo(2000)
    await selectAll(component)

    await open('Color de resaltado')
    await fireEvent.click(swatch('Sin color'))
    await tick()

    expect(surface().querySelector('mark')).toBeNull()
    expect(surface()).toHaveTextContent('rojo')
  })

  it('is one undo step', async () => {
    const { component } = renderEditor('Hola')
    await resizeTo(2000)
    await selectAll(component)

    await open('Color de texto')
    await fireEvent.click(swatch('Azul'))
    await tick()
    expect(surface().querySelector('[data-text-color]')).not.toBeNull()

    await fireEvent.click(button('Deshacer'))
    await tick()
    expect(surface().querySelector('[data-text-color]')).toBeNull()
  })

  it('names every swatch and gives it the shared tooltip, never a native title', async () => {
    renderEditor()
    await resizeTo(2000)

    for (const tool of ['Color de resaltado', 'Color de texto']) {
      expect(button(tool)).toHaveAttribute('data-tooltip', tool)
      expect(button(tool).hasAttribute('title')).toBe(false)
      await open(tool)
      for (const name of COLOUR_NAMES) {
        expect(swatch(name)).toHaveAttribute('data-tooltip', name)
        expect(swatch(name).hasAttribute('title')).toBe(false)
      }
      await fireEvent.keyDown(document.activeElement!, { key: 'Escape' })
      await tick()
    }
  })

  it('walks the grid with the arrow keys, applies with Enter and closes with Escape', async () => {
    const { component } = renderEditor('Hola')
    await resizeTo(2000)
    await selectAll(component)
    const trigger = button('Color de texto')
    const press = async (key: string) => {
      await fireEvent.keyDown(document.activeElement!, { key })
      await tick()
    }

    trigger.focus()
    await press('ArrowDown')
    expect(document.activeElement).toBe(swatch('Sin color'))
    await press('ArrowDown')
    expect(document.activeElement).toBe(swatch('Gris'))
    await press('ArrowRight')
    expect(document.activeElement).toBe(swatch('Rojo'))
    // Four to a row: down from red is blue, left of blue is green.
    await press('ArrowDown')
    expect(document.activeElement).toBe(swatch('Azul'))
    await press('ArrowLeft')
    expect(document.activeElement).toBe(swatch('Verde'))
    await press('ArrowUp')
    expect(document.activeElement).toBe(swatch('Gris'))
    await press('ArrowUp')
    expect(document.activeElement).toBe(swatch('Sin color'))
    await press('ArrowUp')
    expect(document.activeElement).toBe(swatch('Rosa'))

    await press('Escape')
    expect(colourMenu('Color de texto')).toBeNull()
    expect(document.activeElement).toBe(trigger)

    await press('ArrowDown')
    await press('ArrowDown')
    await press('ArrowRight')
    await press('Enter')
    await nextFrame()
    expect((surface().querySelector('[data-text-color]') as HTMLElement).dataset.textColor).toBe(
      'red'
    )
    expect(surface().contains(document.activeElement)).toBe(true)
  })

  it('stays reachable from the overflow menu when typography collapses', async () => {
    const { component } = renderEditor('Hola mundo')
    await resizeTo(400)
    expect(rowOf('second')).not.toContain('Color de texto')
    await selectAll(component)

    await fireEvent.click(moreTools('second')!)
    await tick()
    const texts = within(menu()!).getByRole('group', { name: 'Color de texto' })
    await fireEvent.click(within(texts).getByRole('menuitemradio', { name: 'Violeta' }))
    await tick()
    await nextFrame()

    expect((surface().querySelector('[data-text-color]') as HTMLElement).dataset.textColor).toBe(
      'purple'
    )
    expect(component.selectedText()).toBe('Hola mundo')
    expect(surface().contains(document.activeElement)).toBe(true)
    expect(menu()).toBeNull()
  })
})

describe('WritingEditor toolbar: paragraph', () => {
  const button = (name: string) => within(toolbar()).getByRole('button', { name })
  const spacingMenu = () => screen.queryByRole('menu', { name: 'Interlineado' })
  const spacing = (name: string) => screen.getByRole('menuitemradio', { name })
  const blocks = () => [...surface().querySelectorAll('p, h1, h2, h3')] as HTMLElement[]
  const PARAGRAPH_TOOLS = SECOND_ROW.slice(SECOND_ROW.indexOf('Disminuir sangría'))
  const ALIGNMENTS = ['Alinear a la izquierda', 'Centrar', 'Alinear a la derecha', 'Justificar']

  const paragraphs = (...texts: string[]): CanonicalDocument => ({
    schemaVersion: WRITING_SCHEMA_VERSION,
    doc: {
      type: 'doc',
      content: texts.map((text) => ({ type: 'paragraph', content: [{ type: 'text', text }] })),
    },
  })

  /** Two paragraphs, the first carrying `attrs`. */
  const mixed = (attrs: Record<string, unknown>): CanonicalDocument => ({
    schemaVersion: WRITING_SCHEMA_VERSION,
    doc: {
      type: 'doc',
      content: [
        { type: 'paragraph', attrs, content: [{ type: 'text', text: 'a' }] },
        { type: 'paragraph', content: [{ type: 'text', text: 'b' }] },
      ],
    },
  })

  async function selectAll(component: { selectedText(): string }) {
    surface().focus()
    await fireEvent.keyDown(surface(), { key: 'a', ctrlKey: true })
    await tick()
    expect(component.selectedText()).not.toBe('')
  }

  async function settle() {
    await tick()
    await nextFrame()
  }

  it('is its own group right after typography, ending the second row', async () => {
    renderEditor()
    await resizeTo(2000)

    const row = rowOf('second')
    expect(row.slice(row.indexOf('Color de texto'))).toEqual([
      'Color de texto',
      '|',
      ...PARAGRAPH_TOOLS,
    ])
    expect(PARAGRAPH_TOOLS).toEqual([
      'Disminuir sangría',
      'Aumentar sangría',
      ...ALIGNMENTS,
      'Interlineado',
    ])
  })

  it('collapses after typography, leaving the second row its menu alone', async () => {
    renderEditor()
    await resizeTo(480)
    expect(rowOf('second')).not.toContain('Aumentar tamaño de fuente')
    expect(rowOf('second')).toContain('Centrar')

    await resizeTo(250)
    expect(rowOf('second')).toEqual(['Más herramientas'])
  })

  it('gives every paragraph button its tooltip, never a native title', async () => {
    renderEditor()
    await resizeTo(2000)

    for (const name of PARAGRAPH_TOOLS) {
      expect(button(name)).toHaveAttribute('data-tooltip', name)
      expect(button(name).hasAttribute('title')).toBe(false)
    }
  })

  it('aligns every selected paragraph, keeping the selection and the focus', async () => {
    const { component } = renderEditor(paragraphs('Uno', 'Dos'))
    await resizeTo(2000)
    await selectAll(component)
    const selected = component.selectedText()

    await fireEvent.click(button('Justificar'))
    await settle()

    expect(blocks().map((block) => block.style.textAlign)).toEqual(['justify', 'justify'])
    expect(component.selectedText()).toBe(selected)
    expect(surface().contains(document.activeElement)).toBe(true)
  })

  it('shows the alignment as a radio set: left when none, one at a time', async () => {
    renderEditor('Hola')
    await resizeTo(2000)
    surface().focus()
    await tick()
    const pressed = () =>
      ALIGNMENTS.filter((name) => button(name).getAttribute('aria-pressed') === 'true')

    expect(pressed()).toEqual(['Alinear a la izquierda'])
    await fireEvent.click(button('Centrar'))
    await settle()
    expect(pressed()).toEqual(['Centrar'])
    await fireEvent.click(button('Alinear a la izquierda'))
    await settle()
    expect(pressed()).toEqual(['Alinear a la izquierda'])
    expect(blocks()[0]!.hasAttribute('style')).toBe(false)
  })

  it('shows no alignment pressed on a selection that mixes them', async () => {
    const { component } = renderEditor(mixed({ textAlign: 'center' }))
    await resizeTo(2000)
    await selectAll(component)

    for (const name of ALIGNMENTS) {
      expect(button(name)).not.toHaveAttribute('aria-pressed', 'true')
    }
  })

  it('indents by levels, and disables each button at its end', async () => {
    renderEditor('Hola')
    await resizeTo(2000)
    surface().focus()
    await tick()

    expect(button('Disminuir sangría')).toBeDisabled()
    await fireEvent.click(button('Aumentar sangría'))
    await settle()
    await fireEvent.click(button('Aumentar sangría'))
    await settle()

    expect(blocks()[0]!.dataset.indent).toBe('2')
    expect(button('Disminuir sangría')).toBeEnabled()
    expect(surface().contains(document.activeElement)).toBe(true)

    for (let level = 2; level < 8; level++) {
      await fireEvent.click(button('Aumentar sangría'))
      await settle()
    }
    expect(blocks()[0]!.dataset.indent).toBe('8')
    expect(button('Aumentar sangría')).toBeDisabled()
  })

  it('nests and un-nests a list item with the indent buttons', async () => {
    const { component } = renderEditor({
      schemaVersion: WRITING_SCHEMA_VERSION,
      doc: {
        type: 'doc',
        content: [
          {
            type: 'bulletList',
            content: ['Uno', 'Dos'].map((text) => ({
              type: 'listItem',
              content: [{ type: 'paragraph', content: [{ type: 'text', text }] }],
            })),
          },
        ],
      },
    })
    await resizeTo(2000)
    surface().focus()
    // Inside "Dos", the second item.
    component.goToPosition(11)
    await tick()
    expect(button('Aumentar sangría')).toBeEnabled()

    await fireEvent.click(button('Aumentar sangría'))
    await settle()
    expect(surface().querySelector('li ul li')).toHaveTextContent('Dos')
    expect(surface().querySelector('[data-indent]')).toBeNull()

    await fireEvent.click(button('Disminuir sangría'))
    await settle()
    expect(surface().querySelector('li ul')).toBeNull()
    expect(surface().querySelectorAll('ul > li')).toHaveLength(2)
  })

  /**
   * Each tool, run on two paragraphs, comes off whole with one undo. They are
   * tried one at a time: history folds edits made within half a second of
   * each other into one step, which would hide a tool that took two.
   */
  it('is one undo step for each tool', async () => {
    const { component } = renderEditor(paragraphs('Uno', 'Dos'))
    await resizeTo(2000)
    await selectAll(component)
    const untouched = () => blocks().every((block) => !block.hasAttribute('style'))

    for (const tool of ['Centrar', 'Aumentar sangría']) {
      await fireEvent.click(button(tool))
      await settle()
      expect(untouched(), tool).toBe(false)
      await fireEvent.click(button('Deshacer'))
      await tick()
      expect(untouched(), tool).toBe(true)
    }

    await fireEvent.click(button('Interlineado'))
    await tick()
    await fireEvent.click(spacing('2'))
    await settle()
    expect(blocks().map((block) => block.dataset.lineHeight)).toEqual(['2', '2'])
    await fireEvent.click(button('Deshacer'))
    await tick()
    expect(untouched()).toBe(true)
  })

  it('sets line spacing from its menu, checking the current value', async () => {
    const { component } = renderEditor(paragraphs('Uno', 'Dos'))
    await resizeTo(2000)
    await selectAll(component)

    await fireEvent.click(button('Interlineado'))
    await tick()
    const items = [...spacingMenu()!.querySelectorAll('[role="menuitemradio"]')].map((item) => [
      item.textContent?.trim(),
      item.getAttribute('aria-checked'),
    ])
    expect(items).toEqual([
      ['1', 'false'],
      ['1,15', 'false'],
      ['1,5', 'false'],
      ['2', 'false'],
      ['Predeterminado', 'true'],
    ])

    await fireEvent.click(spacing('1,5'))
    await settle()
    expect(blocks().map((block) => block.dataset.lineHeight)).toEqual(['1.5', '1.5'])
    expect(component.selectedText()).not.toBe('')
    expect(surface().contains(document.activeElement)).toBe(true)
    expect(spacingMenu()).toBeNull()

    await fireEvent.click(button('Interlineado'))
    await tick()
    expect(spacing('1,5')).toHaveAttribute('aria-checked', 'true')
    await fireEvent.click(spacing('Predeterminado'))
    await settle()
    expect(blocks().every((block) => !block.hasAttribute('style'))).toBe(true)
  })

  it('checks no line spacing on a selection that mixes them', async () => {
    const { component } = renderEditor(mixed({ lineHeight: '2' }))
    await resizeTo(2000)
    await selectAll(component)

    await fireEvent.click(button('Interlineado'))
    await tick()
    expect(spacingMenu()!.querySelectorAll('[aria-checked="true"]')).toHaveLength(0)
  })

  it('opens the line spacing menu from the keyboard and applies with Enter', async () => {
    renderEditor('Hola')
    await resizeTo(2000)
    surface().focus()
    await tick()

    const trigger = button('Interlineado')
    trigger.focus()
    await fireEvent.keyDown(trigger, { key: 'ArrowDown' })
    await tick()
    expect(document.activeElement).toBe(spacing('1'))
    await fireEvent.keyDown(document.activeElement!, { key: 'ArrowDown' })
    await fireEvent.keyDown(document.activeElement!, { key: 'Enter' })
    await settle()

    expect(blocks()[0]!.dataset.lineHeight).toBe('1.15')
    expect(surface().contains(document.activeElement)).toBe(true)
  })

  it('takes the decimal separator from the labels', async () => {
    render(WritingEditor, {
      props: {
        document: manuscript('Hola'),
        labels: { lineHeight: 'Line spacing', lineHeight115: '1.15', lineHeightDefault: 'Default' },
      },
    })
    await resizeTo(2000)

    await fireEvent.click(within(toolbar()).getByRole('button', { name: 'Line spacing' }))
    await tick()
    expect(spacing('1.15')).toBeInTheDocument()
    expect(spacing('Default')).toBeInTheDocument()
  })

  it('offers the group in the overflow menu, spacing under a heading', async () => {
    const { component } = renderEditor(paragraphs('Uno', 'Dos'))
    await resizeTo(250)
    expect(rowOf('second')).not.toContain('Centrar')
    await selectAll(component)
    const selected = component.selectedText()
    const item = (role: string, name: string) => within(menu()!).getByRole(role, { name })

    await fireEvent.click(moreTools('second')!)
    await tick()
    expect(menu()!.querySelector('.toolbar-menu__heading')).toHaveTextContent('Interlineado')
    expect(item('menuitemradio', 'Alinear a la izquierda')).toHaveAttribute('aria-checked', 'true')
    expect(item('menuitemradio', 'Predeterminado')).toHaveAttribute('aria-checked', 'true')
    await fireEvent.click(item('menuitemradio', 'Alinear a la derecha'))
    await settle()

    expect(blocks().map((block) => block.style.textAlign)).toEqual(['right', 'right'])
    expect(component.selectedText()).toBe(selected)
    expect(surface().contains(document.activeElement)).toBe(true)
    expect(menu()).toBeNull()

    await fireEvent.click(moreTools('second')!)
    await tick()
    await fireEvent.click(item('menuitemradio', '2'))
    await settle()
    expect(blocks().map((block) => block.dataset.lineHeight)).toEqual(['2', '2'])

    await fireEvent.click(moreTools('second')!)
    await tick()
    await fireEvent.click(item('menuitem', 'Aumentar sangría'))
    await settle()
    expect(blocks().map((block) => block.dataset.indent)).toEqual(['1', '1'])
    expect(component.selectedText()).toBe(selected)
    expect(surface().contains(document.activeElement)).toBe(true)
  })
})
