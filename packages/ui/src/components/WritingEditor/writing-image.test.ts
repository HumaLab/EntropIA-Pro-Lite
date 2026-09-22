import { Editor } from '@tiptap/core'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createWritingExtensions } from './extensions'
import { emptyDocument } from './document-contract'

let editor: Editor | undefined

afterEach(() => {
  editor?.destroy()
  editor = undefined
})

function mount() {
  const element = document.createElement('div')
  document.body.appendChild(element)
  editor = new Editor({ element, extensions: createWritingExtensions(), content: emptyDocument().doc })
  return editor
}

describe('the writingImage node', () => {
  it('is created by insertWritingImage, carrying its attrs', () => {
    const instance = mount()

    instance
      .chain()
      .focus()
      .insertWritingImage({ src: 'writing-images/abc.png', width: 300, height: 150 })
      .run()

    const json = instance.getJSON()
    const figure = json.content?.find((node) => node.type === 'writingImage')
    expect(figure?.attrs).toMatchObject({
      src: 'writing-images/abc.png',
      width: 300,
      height: 150,
      align: 'center',
    })
  })

  it('inserts at the cursor without replacing an existing text selection', () => {
    const instance = mount()
    instance.chain().focus().insertContent('hola mundo').run()
    // Selects exactly "hola" (positions 1..5, the four letters). "mundo"
    // alone cannot prove this: it sits outside the selected range and
    // survives whether or not the selection was deleted. The selected word
    // itself is the only thing that tells the two cases apart.
    instance.commands.setTextSelection({ from: 1, to: 5 })

    instance.chain().insertWritingImage({ src: 'writing-images/abc.png' }).run()

    const text = instance.getText()
    expect(text).toContain('hola')
    const names: string[] = []
    instance.state.doc.forEach((node) => names.push(node.type.name))
    expect(names).toContain('writingImage')
  })

  it('round-trips figure[data-writing-image] back into the node, caption included', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    const before = instance.getJSON()

    const rebuilt = new Editor({
      element: (() => {
        const el = document.createElement('div')
        document.body.appendChild(el)
        return el
      })(),
      extensions: createWritingExtensions(),
      content: before,
    })

    expect(rebuilt.getJSON()).toEqual(before)
    rebuilt.destroy()
  })

  it('does not parse a bare img[src] as a writingImage', () => {
    const instance = mount()

    instance.chain().focus().insertContent('<img src="https://example.org/remote.png">').run()

    const names: string[] = []
    instance.state.doc.forEach((node) => names.push(node.type.name))
    expect(names).not.toContain('writingImage')
  })

  it('joins the trailing-paragraph TRAPPING set: a caret always has somewhere to land after it', () => {
    const instance = mount()

    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()

    const names: string[] = []
    instance.state.doc.forEach((node) => names.push(node.type.name))
    expect(names.at(-1)).toBe('paragraph')
  })

  it('Enter inside the caption exits the node instead of splitting it', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    // A caption with real text, cursor genuinely mid-caption — not at
    // offset 0 and not empty. An empty caption cannot mutation-test this
    // handler: ProseMirror's own default `splitBlock`, when splitting an
    // empty textblock, already falls back to the document's default block
    // type instead of duplicating the current node's type, so an empty
    // caption produces the same outcome whether or not this handler runs at
    // all. Only real caption text, split mid-way, exposes what stock
    // `splitBlock` actually does here: duplicate the writingImage node
    // itself, each half carrying the same `src` — the defect this handler
    // exists to prevent.
    instance.chain().insertContent('un pie de foto').run()
    const midPos = instance.state.selection.to - 5 // between "un pie de" and " foto"
    instance.commands.setTextSelection(midPos)
    expect(instance.isActive('writingImage')).toBe(true)

    // `commands.keyboardShortcut()` only replays the keymap's document
    // steps onto its observable transaction; the handler's own trailing
    // `setTextSelection` carries no steps, so it would be captured and then
    // silently dropped — the same gap round 1 found for Backspace. Deliver
    // the key the way ProseMirror itself delivers one, so the cursor's
    // final position is genuinely observable too.
    const event = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true })
    instance.view.someProp('handleKeyDown', (handler) => handler(instance.view, event))

    const json = instance.getJSON()
    const figures = json.content?.filter((node) => node.type === 'writingImage') ?? []
    // Exactly one writingImage — never two, which is what stock splitBlock
    // does to a mid-caption Enter left unhandled — and its caption is the
    // whole, unsplit text, proving Enter exited the node rather than
    // dividing its content.
    expect(figures).toHaveLength(1)
    expect(figures[0]?.content?.map((n) => n.text).join('')).toBe('un pie de foto')

    const names: string[] = []
    instance.state.doc.forEach((node) => names.push(node.type.name))
    // Two paragraphs: the one Enter inserts plus the one TrailingParagraph
    // already kept after the figure.
    expect(names.filter((name) => name === 'paragraph')).toHaveLength(2)
    // The cursor lands in the newly inserted paragraph, not back inside the
    // figure's caption — confirming Enter actually exited the node rather
    // than leaving the selection untouched.
    expect(instance.state.selection.toJSON()).toEqual({ type: 'text', anchor: 17, head: 17 })
  })

  it('Backspace at the start of an empty caption selects the figure', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    // Land inside the (empty) caption itself (offset 0 of the writingImage's
    // own inline content). `doc.content.size - 2` lands one node further
    // out, on the doc-level boundary between the figure and the paragraph
    // TrailingParagraph appends after it — a position ProseMirror's own
    // default Backspace binding (`selectNodeBackward`) already resolves to a
    // node selection on its own, regardless of this node's keymap, so it
    // does not exercise the code under test.
    instance.commands.setTextSelection(1)

    // `commands.keyboardShortcut()` captures the keymap's transaction and
    // replays only its document steps onto a fresh one — a pure selection
    // change carries zero steps, so it is captured and then silently
    // dropped, and the assertion below could never observe it no matter how
    // correct the keymap is. Deliver the key the way ProseMirror itself
    // delivers one instead: through `handleKeyDown`, the real path a
    // writer's keypress takes in a live editor, with no capture/replay in
    // between.
    const event = new KeyboardEvent('keydown', {
      key: 'Backspace',
      bubbles: true,
      cancelable: true,
    })
    instance.view.someProp('handleKeyDown', (handler) => handler(instance.view, event))

    expect(instance.state.selection.toJSON().type).toBe('node')
  })

  it('deleting a selected image removes the node; undo restores it fully; redo removes it again', () => {
    // The image and its caption are the document's starting content, not
    // built up through separate insertWritingImage/insertContent
    // transactions: prosemirror-history groups transactions that land within
    // its newGroupDelay of each other into one undo step, and three
    // transactions fired back to back with no real time between them (as a
    // synchronous test does) land in the same group — a single undo would
    // then revert the insert and the caption too, not just the delete this
    // test means to isolate. Starting from this document, the only
    // transaction in the editor's history is the delete itself.
    const element = document.createElement('div')
    document.body.appendChild(element)
    const initial = {
      type: 'doc',
      content: [
        {
          type: 'writingImage',
          attrs: {
            src: 'writing-images/abc.png',
            alt: 'Vista',
            title: 'Un título',
            width: 300,
            height: 150,
            align: 'right',
          },
          content: [{ type: 'text', text: 'un pie de foto' }],
        },
        { type: 'paragraph' },
      ],
    }
    const instance = new Editor({ element, extensions: createWritingExtensions(), content: initial })
    editor = instance

    const pos = writingImagePos(instance)
    instance.commands.setNodeSelection(pos)
    instance.commands.deleteSelection()

    expect(namesOf(instance)).not.toContain('writingImage')

    instance.commands.undo()
    const restored = instance.getJSON()
    const figure = restored.content?.find((node) => node.type === 'writingImage')
    expect(figure?.attrs).toMatchObject({
      src: 'writing-images/abc.png',
      width: 300,
      height: 150,
      align: 'right',
      alt: 'Vista',
      title: 'Un título',
    })
    expect(figure?.content?.map((n) => n.text).join('')).toBe('un pie de foto')

    instance.commands.redo()
    expect(namesOf(instance)).not.toContain('writingImage')
  })
})

/** The document position of the (only) writingImage node in the doc. */
function writingImagePos(instance: Editor): number {
  let found = -1
  instance.state.doc.descendants((node, pos) => {
    if (node.type.name === 'writingImage') found = pos
    return found === -1
  })
  if (found === -1) throw new Error('no writingImage node in the document')
  return found
}

function namesOf(instance: Editor): string[] {
  const names: string[] = []
  instance.state.doc.forEach((node) => names.push(node.type.name))
  return names
}

/** Drives the node view's resize handle the way a real pointer gesture would,
 *  without relying on happy-dom's DragEvent/PointerEvent construction (which
 *  drops eventInit fields — see writing-image-paste.test.ts's dropEvent
 *  comment for the same limitation with DragEvent). `clientWidth` is stubbed
 *  on the figure's parent because happy-dom runs no layout engine at all: the
 *  clamp arithmetic itself is only ever exercised with real numbers in
 *  writing-image-resize.test.ts, and this harness's job is only to prove the
 *  node view *reaches* that arithmetic with the right inputs — not to prove
 *  a real browser lays anything out a particular way, which no unit test in
 *  this repository can do (spec, Node View and Resizing: "no test drags a
 *  handle or reads a rendered size"). */
function dragHandle(figure: HTMLElement, available: number, from: number, to: number) {
  const parent = figure.parentElement
  if (parent) Object.defineProperty(parent, 'clientWidth', { value: available, configurable: true })
  const handle = figure.querySelector<HTMLButtonElement>('.writing-editor__image-handle')
  if (!handle) throw new Error('no resize handle in the node view')

  const pointerdown = new Event('pointerdown')
  Object.defineProperty(pointerdown, 'clientX', { value: from })
  handle.dispatchEvent(pointerdown)

  const pointermove = new Event('pointermove')
  Object.defineProperty(pointermove, 'clientX', { value: to })
  window.dispatchEvent(pointermove)

  const pointerup = new Event('pointerup')
  Object.defineProperty(pointerup, 'clientX', { value: to })
  window.dispatchEvent(pointerup)
}

describe('the resize handle keeps reading the current node after an external update (C1)', () => {
  it('bases a drag on the node’s current width, not the width it had when the view was created', () => {
    const instance = mount()
    instance
      .chain()
      .focus()
      .insertWritingImage({ src: 'writing-images/abc.png', width: 500, height: 250 })
      .run()
    const pos = writingImagePos(instance)

    // Something other than this drag changed the node's width after the view
    // was created — the same situation a *second* drag leaves behind: the
    // first drag's own dispatch calls update(), but (before this fix) never
    // reassigns the closure's `node`, so the next drag still starts from the
    // width the node had when the view was first mounted rather than the
    // width the previous drag actually landed on.
    instance.view.dispatch(
      instance.state.tr.setNodeAttribute(pos, 'width', 800).setNodeAttribute(pos, 'height', 400)
    )

    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    if (!figure) throw new Error('no writingImage node view mounted')

    // available=900, drag +50px from the *current* 800 → 850. A view still
    // reading the stale 500 would instead compute 500+50=550, and the stale
    // 500/250 aspect would give it the wrong height too.
    dragHandle(figure, 900, 100, 150)

    const json = instance.getJSON()
    const after = json.content?.find((node) => node.type === 'writingImage')
    expect(after?.attrs?.width).toBe(850)
    expect(after?.attrs?.height).toBe(425)
  })

  it('a resize transaction changes width and undoes as a single step (contract 9)', () => {
    const instance = mount()
    instance
      .chain()
      .focus()
      .insertWritingImage({ src: 'writing-images/abc.png', width: 400, height: 200 })
      .run()

    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    if (!figure) throw new Error('no writingImage node view mounted')

    dragHandle(figure, 700, 100, 160)

    const resized = instance.getJSON().content?.find((node) => node.type === 'writingImage')
    expect(resized?.attrs?.width).toBe(460)

    instance.commands.undo()
    const undone = instance.getJSON().content?.find((node) => node.type === 'writingImage')
    expect(undone?.attrs).toMatchObject({ width: 400, height: 200 })
  })
})

/** Mounts with a custom `imageLabels`, so the routing (I2) is provably from
 *  the option and not a coincidence with the English default set. */
function mountWithLabels() {
  const element = document.createElement('div')
  document.body.appendChild(element)
  editor = new Editor({
    element,
    extensions: createWritingExtensions({
      imageLabels: {
        alignLeft: 'IZQUIERDA',
        alignCenter: 'CENTRO',
        alignRight: 'DERECHA',
        altLabel: 'TEXTO ALT',
        titleLabel: 'TITULO CAMPO',
        resizeHandle: 'ASA DE REDIMENSION',
        missingImage: 'IMAGEN PERDIDA',
      },
    }),
    content: emptyDocument().doc,
  })
  return editor
}

describe('the node view chrome (I1/I2/I3)', () => {
  it('reveals the chrome only once ProseMirror marks the figure as the selected node', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    if (!figure) throw new Error('no writingImage node view mounted')

    // The CSS rule that hides/reveals `.writing-editor__image-chrome` lives in
    // WritingEditor.svelte's own <style> block and is out of reach from a
    // plain Editor mount in this test file (no Svelte runtime, and happy-dom
    // runs no layout/cascade engine to assert a resolved `display` against
    // even if it were mounted). What is directly reachable and asserted here
    // is the contract that CSS rule depends on: ProseMirror's own
    // `NodeViewDesc.selectNode`/`deselectNode` toggle
    // `ProseMirror-selectednode` on exactly this figure — the selector the
    // stylesheet keys off — and nothing else marks or unmarks it.
    expect(figure.classList.contains('ProseMirror-selectednode')).toBe(false)

    const pos = writingImagePos(instance)
    instance.commands.setNodeSelection(pos)
    expect(figure.classList.contains('ProseMirror-selectednode')).toBe(true)

    instance.commands.setTextSelection(0)
    expect(figure.classList.contains('ProseMirror-selectednode')).toBe(false)
  })

  it('labels the alignment buttons, the alt/title fields and the resize handle from imageLabels', () => {
    const instance = mountWithLabels()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    if (!figure) throw new Error('no writingImage node view mounted')

    const alignButtons = [...figure.querySelectorAll<HTMLButtonElement>('.writing-editor__image-align button')]
    expect(alignButtons.map((button) => button.textContent)).toEqual(['IZQUIERDA', 'CENTRO', 'DERECHA'])

    const altInput = figure.querySelector<HTMLInputElement>('.writing-editor__image-fields input:nth-of-type(1)')
    const titleInput = figure.querySelector<HTMLInputElement>('.writing-editor__image-fields input:nth-of-type(2)')
    expect(altInput?.getAttribute('aria-label')).toBe('TEXTO ALT')
    expect(titleInput?.getAttribute('aria-label')).toBe('TITULO CAMPO')

    const handle = figure.querySelector<HTMLButtonElement>('.writing-editor__image-handle')
    expect(handle?.getAttribute('aria-label')).toBe('ASA DE REDIMENSION')
  })

  it('edits alt and title from the inline fields, never through window.prompt (I3)', () => {
    const promptSpy = vi.spyOn(window, 'prompt')
    const instance = mount()
    instance
      .chain()
      .focus()
      .insertWritingImage({ src: 'writing-images/abc.png', alt: 'antes', title: 'antes también' })
      .run()
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    if (!figure) throw new Error('no writingImage node view mounted')

    const altInput = figure.querySelector<HTMLInputElement>('.writing-editor__image-fields input:nth-of-type(1)')
    const titleInput = figure.querySelector<HTMLInputElement>('.writing-editor__image-fields input:nth-of-type(2)')
    if (!altInput || !titleInput) throw new Error('no alt/title fields in the chrome')
    expect(altInput.value).toBe('antes')
    expect(titleInput.value).toBe('antes también')

    altInput.value = 'después'
    altInput.dispatchEvent(new Event('input', { bubbles: true }))
    titleInput.value = 'después también'
    titleInput.dispatchEvent(new Event('input', { bubbles: true }))

    const json = instance.getJSON()
    const written = json.content?.find((node) => node.type === 'writingImage')
    expect(written?.attrs).toMatchObject({ alt: 'después', title: 'después también' })
    expect(promptSpy).not.toHaveBeenCalled()
    promptSpy.mockRestore()
  })

  it('keeps the alt/title fields current after a second, external attribute change (C1)', () => {
    const instance = mount()
    instance
      .chain()
      .focus()
      .insertWritingImage({ src: 'writing-images/abc.png', alt: 'primero', title: 'primero' })
      .run()
    const pos = writingImagePos(instance)
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    if (!figure) throw new Error('no writingImage node view mounted')
    const altInput = figure.querySelector<HTMLInputElement>('.writing-editor__image-fields input:nth-of-type(1)')
    if (!altInput) throw new Error('no alt field in the chrome')

    instance.view.dispatch(instance.state.tr.setNodeAttribute(pos, 'alt', 'segundo'))
    expect(altInput.value).toBe('segundo')

    instance.view.dispatch(instance.state.tr.setNodeAttribute(pos, 'alt', 'tercero'))
    expect(altInput.value).toBe('tercero')
  })
})

describe('a stored file missing at render time (I6)', () => {
  it('draws a placeholder and keeps the node', () => {
    const instance = mount()
    instance
      .chain()
      .focus()
      .insertWritingImage({ src: 'writing-images/missing.png', alt: 'Vista perdida' })
      .run()
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    const img = figure?.querySelector('img')
    if (!figure || !img) throw new Error('no writingImage node view mounted')

    expect('broken' in figure.dataset).toBe(false)

    img.dispatchEvent(new Event('error'))

    expect('broken' in figure.dataset).toBe(true)
    const placeholder = figure.querySelector('.writing-editor__image-placeholder')
    expect(placeholder).not.toBeNull()
    expect(namesOf(instance)).toContain('writingImage')

    img.dispatchEvent(new Event('load'))
    expect('broken' in figure.dataset).toBe(false)
  })
})
