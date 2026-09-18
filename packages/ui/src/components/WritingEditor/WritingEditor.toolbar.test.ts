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

function renderEditor(text = 'Hola mundo') {
  return render(WritingEditor, { props: { document: manuscript(text), ondictate: vi.fn() } })
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
