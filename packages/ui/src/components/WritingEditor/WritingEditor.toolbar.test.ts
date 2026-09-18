import { fireEvent, render, screen, within } from '@testing-library/svelte'
import { tick } from 'svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import WritingEditor from './WritingEditor.svelte'
import { WRITING_SCHEMA_VERSION, type CanonicalDocument } from './document-contract'

/**
 * The toolbar keeps to one row: groups that do not fit collapse, least used
 * first, into a trailing "more tools" menu.
 *
 * There is no layout engine here, so the geometry is supplied: every button is
 * 28px, a group is as wide as its buttons plus the gaps between them, and the
 * toolbar is as wide as the test says. The observer is a fake the test fires.
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
const moreTools = () => within(toolbar()).queryByRole('button', { name: 'Más herramientas' })
const menu = () => screen.queryByRole('menu', { name: 'Más herramientas' })

/** The row as a reader would scan it: button names, and `|` for a separator. */
function rowOf(): string[] {
  return [...toolbar().querySelectorAll<HTMLElement>('button, .writing-editor__sep')].map(
    (element) => (element.tagName === 'BUTTON' ? (element.getAttribute('aria-label') ?? '?') : '|')
  )
}

const FULL_ROW = [
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
  '|',
  'Aumentar tamaño de fuente',
  'Disminuir tamaño de fuente',
  'Cambiar mayúsculas y minúsculas',
  'Subíndice',
  'Superíndice',
  'Borrar formato',
  'Color de resaltado',
  'Color de texto',
  'Buscar',
  '|',
  'Iniciar dictado',
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
    return this.getAttribute('role') === 'toolbar' ? toolbarWidth : 0
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

describe('WritingEditor toolbar: when everything fits', () => {
  it('is today’s row exactly, with no overflow button', async () => {
    renderEditor()
    await resizeTo(2000)

    expect(rowOf()).toEqual(FULL_ROW)
    expect(moreTools()).toBeNull()
  })
})

describe('WritingEditor toolbar: when it does not fit', () => {
  it('collapses whole groups, least used first, and keeps the rest in order', async () => {
    renderEditor()
    await resizeTo(400)

    expect(rowOf()).toEqual([
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
  })

  it('keeps the microphone last and find just before it', async () => {
    renderEditor()
    await resizeTo(300)

    const names = rowOf().filter((name) => name !== '|')
    expect(names.at(-1)).toBe('Iniciar dictado')
    expect(names.at(-2)).toBe('Buscar')
    expect(names.at(-3)).toBe('Más herramientas')
  })

  it('lists the collapsed tools in the menu, in toolbar order, with their state', async () => {
    renderEditor()
    await resizeTo(400)

    await fireEvent.click(moreTools()!)
    await tick()

    // A swatch has no text of its own: it is named by its label.
    const items = [...menu()!.querySelectorAll('[role^="menuitem"]')].map((item) => [
      item.getAttribute('role'),
      item.getAttribute('aria-label') ?? item.textContent?.trim(),
    ])
    expect(items).toEqual([
      ['menuitemcheckbox', 'Tachado'],
      ['menuitemcheckbox', 'Código'],
      ['menuitemcheckbox', 'Lista'],
      ['menuitemcheckbox', 'Lista ordenada'],
      ['menuitemcheckbox', 'Cita en bloque'],
      ['menuitemcheckbox', 'Enlace'],
      ['menuitem', 'Insertar tabla'],
      ['menuitem', 'Nota al pie'],
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
    ])
  })

  it('brings the groups back, and drops the button, when the room returns', async () => {
    renderEditor()
    await resizeTo(400)
    await resizeTo(2000)

    expect(rowOf()).toEqual(FULL_ROW)
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
    expect(moreTools()).toHaveAttribute('data-tooltip', 'Más herramientas')
  })

  it('takes the overflow label from the labels it is given', async () => {
    render(WritingEditor, {
      props: { document: manuscript('Hola'), labels: { moreTools: 'More tools' } },
    })
    await resizeTo(300)

    expect(within(toolbar()).getByRole('button', { name: 'More tools' })).toHaveAttribute(
      'data-tooltip',
      'More tools'
    )
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

  it('is the first group to give way, before strike-through and code', async () => {
    renderEditor()
    await resizeTo(700)

    const row = rowOf()
    expect(row).not.toContain('Aumentar tamaño de fuente')
    expect(row).toContain('Tachado')
    expect(row).toContain('Código')
    expect(row.slice(-6)).toEqual([
      'Nota al pie',
      '|',
      'Más herramientas',
      'Buscar',
      '|',
      'Iniciar dictado',
    ])
  })

  it('gives every typography button its tooltip', async () => {
    renderEditor()
    await resizeTo(2000)

    const tools = FULL_ROW.slice(FULL_ROW.indexOf('Aumentar tamaño de fuente'), -3)
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

    await fireEvent.click(moreTools()!)
    await tick()
    await fireEvent.click(screen.getByRole('menuitem', { name: 'Capitalizar palabras' }))
    await tick()
    await nextFrame()

    expect(surface()).toHaveTextContent('Hola Mundo')
    expect(component.selectedText()).toBe('Hola Mundo')
    expect(surface().contains(document.activeElement)).toBe(true)
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

    const row = rowOf()
    expect(row.slice(row.indexOf('Borrar formato'), row.indexOf('Buscar'))).toEqual([
      'Borrar formato',
      'Color de resaltado',
      'Color de texto',
    ])
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
    await resizeTo(700)
    expect(rowOf()).not.toContain('Color de texto')
    await selectAll(component)

    await fireEvent.click(moreTools()!)
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
