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
  'Buscar',
  '|',
  'Iniciar dictado',
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

    const items = [...menu()!.querySelectorAll('[role^="menuitem"]')].map((item) => [
      item.getAttribute('role'),
      item.textContent?.trim(),
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
    expect(tools).toHaveLength(6)
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
