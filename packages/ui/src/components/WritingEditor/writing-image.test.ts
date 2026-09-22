import { Editor } from '@tiptap/core'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createWritingExtensions } from './extensions'
import { emptyDocument } from './document-contract'
import { DEFAULT_WRITING_IMAGE_LABELS } from './writing-image-labels'

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
        resizeHandle: 'ASA DE REDIMENSION',
        missingImage: 'IMAGEN PERDIDA',
        captionPlaceholder: 'MARCADOR DE PIE DE FOTO',
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

  it('labels the alignment buttons, the alt field and the resize handle from imageLabels', () => {
    const instance = mountWithLabels()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    if (!figure) throw new Error('no writingImage node view mounted')

    const alignButtons = [...figure.querySelectorAll<HTMLButtonElement>('.writing-editor__image-align button')]
    expect(alignButtons.map((button) => button.textContent)).toEqual(['IZQUIERDA', 'CENTRO', 'DERECHA'])

    // Defect 4: the bar carries exactly one field — alt text. `title` is an
    // HTML tooltip, not a caption, and has no editable UI here any more.
    const inputs = [...figure.querySelectorAll<HTMLInputElement>('.writing-editor__image-fields input')]
    expect(inputs).toHaveLength(1)
    expect(inputs[0]?.getAttribute('aria-label')).toBe('TEXTO ALT')

    const handle = figure.querySelector<HTMLButtonElement>('.writing-editor__image-handle')
    expect(handle?.getAttribute('aria-label')).toBe('ASA DE REDIMENSION')

    const figcaption = figure.querySelector('figcaption')
    expect(figcaption?.dataset.placeholder).toBe('MARCADOR DE PIE DE FOTO')
  })

  it('edits alt from the inline field, never through window.prompt (I3)', () => {
    const promptSpy = vi.spyOn(window, 'prompt')
    const instance = mount()
    instance
      .chain()
      .focus()
      .insertWritingImage({ src: 'writing-images/abc.png', alt: 'antes' })
      .run()
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    if (!figure) throw new Error('no writingImage node view mounted')

    const altInput = figure.querySelector<HTMLInputElement>('.writing-editor__image-fields input')
    if (!altInput) throw new Error('no alt field in the chrome')
    expect(altInput.value).toBe('antes')

    altInput.value = 'después'
    altInput.dispatchEvent(new Event('input', { bubbles: true }))

    const json = instance.getJSON()
    const written = json.content?.find((node) => node.type === 'writingImage')
    expect(written?.attrs).toMatchObject({ alt: 'después' })
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

// Defect 1: the node is `content: 'inline*'`, not an atom, so ProseMirror's
// own default click handling (`selectClickedLeaf`, prosemirror-view
// dist/index.js:3224-3233) never selects it — that default requires
// `node.isAtom`, and drops the caret into the caption instead. The only way
// to reach a NodeSelection used to be Backspace at the start of an empty
// caption (the keymap above), which is exactly the anomaly this fix removes:
// clicking the image now selects it directly.
describe('click-to-select on the image (defect 1)', () => {
  it('clicking the image creates a NodeSelection at the figure', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    const pos = writingImagePos(instance)
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    const img = figure?.querySelector('img')
    if (!figure || !img) throw new Error('no writingImage node view mounted')

    // Starts from a selection that is provably not already a NodeSelection
    // at this position, so the assertion below proves the click moved it.
    instance.commands.setTextSelection(0)
    expect(instance.state.selection.toJSON().type).not.toBe('node')

    img.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))

    expect(instance.state.selection.toJSON()).toMatchObject({ type: 'node', anchor: pos })
  })

  it('clicking the broken-image placeholder also creates a NodeSelection', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/missing.png' }).run()
    const pos = writingImagePos(instance)
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    const img = figure?.querySelector('img')
    const placeholder = figure?.querySelector<HTMLElement>('.writing-editor__image-placeholder')
    if (!figure || !img || !placeholder) throw new Error('no writingImage node view mounted')
    img.dispatchEvent(new Event('error'))

    instance.commands.setTextSelection(0)
    placeholder.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))

    expect(instance.state.selection.toJSON()).toMatchObject({ type: 'node', anchor: pos })
  })

  it('clicking the caption does not go through the image’s click-to-select handler', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    const pos = writingImagePos(instance)
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    const figcaption = figure?.querySelector('figcaption')
    if (!figure || !figcaption) throw new Error('no writingImage node view mounted')
    instance.commands.setNodeSelection(pos)
    expect(instance.state.selection.toJSON().type).toBe('node')

    // Real caret placement inside a contentEditable figcaption needs
    // coordinate-based hit testing (posAtCoords) that happy-dom does not
    // implement, so "the caret lands in the caption" is not asserted here —
    // only that this fix's own new listener, wired to the image and its
    // placeholder alone, never fires for a click that lands on the caption.
    const dispatchSpy = vi.spyOn(instance.view, 'dispatch')
    figcaption.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true }))
    expect(dispatchSpy).not.toHaveBeenCalled()
    dispatchSpy.mockRestore()
  })
})

// Defect 2: prosemirror-view marks the figure (`nodeDOM`) `draggable = true`
// whenever the node is selected, because this node view has a `contentDOM`
// (dist/index.js:1490-1493) — independently of `img.draggable`/
// `handle.draggable`, which this view already sets to `false` and which do
// not stop the browser walking up to the nearest draggable ancestor. A
// pointerdown on the handle must cancel the native `dragstart` the browser
// would otherwise fire on the figure once the gesture moves, or the resize
// gesture dies mid-drag under a "not allowed" cursor. What is asserted here
// is this cancellation logic itself (a plain `dragstart` listener) — not
// real native HTML5 drag-and-drop, real layout, or the cursor the browser
// draws, none of which happy-dom or any unit-test runner can produce; that
// part is for the user to confirm in the running app (see report).
describe('native drag suppression while resizing (defect 2)', () => {
  it('cancels the dragstart the browser would fire when the gesture begins on the resize handle', () => {
    const instance = mount()
    instance
      .chain()
      .focus()
      .insertWritingImage({ src: 'writing-images/abc.png', width: 300, height: 150 })
      .run()
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    const handle = figure?.querySelector<HTMLButtonElement>('.writing-editor__image-handle')
    if (!figure || !handle) throw new Error('no writingImage node view mounted')

    const pointerdown = new Event('pointerdown')
    Object.defineProperty(pointerdown, 'clientX', { value: 100 })
    handle.dispatchEvent(pointerdown)

    const dragstart = new Event('dragstart', { cancelable: true })
    figure.dispatchEvent(dragstart)
    expect(dragstart.defaultPrevented).toBe(true)

    // Tears the gesture down the way a real pointerup would, so it cannot
    // leak into another test.
    window.dispatchEvent(new Event('pointerup'))
  })

  it('leaves the figure’s own drag (repositioning it) untouched when the gesture does not start on the handle', () => {
    const instance = mount()
    instance
      .chain()
      .focus()
      .insertWritingImage({ src: 'writing-images/abc.png', width: 300, height: 150 })
      .run()
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    if (!figure) throw new Error('no writingImage node view mounted')

    const dragstart = new Event('dragstart', { cancelable: true })
    figure.dispatchEvent(dragstart)
    expect(dragstart.defaultPrevented).toBe(false)
  })

  it('re-arms after a completed resize: a second, unrelated dragstart is not cancelled', () => {
    const instance = mount()
    instance
      .chain()
      .focus()
      .insertWritingImage({ src: 'writing-images/abc.png', width: 300, height: 150 })
      .run()
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    const handle = figure?.querySelector<HTMLButtonElement>('.writing-editor__image-handle')
    if (!figure || !handle) throw new Error('no writingImage node view mounted')

    dragHandle(figure, 900, 100, 150)

    const dragstart = new Event('dragstart', { cancelable: true })
    figure.dispatchEvent(dragstart)
    expect(dragstart.defaultPrevented).toBe(false)
  })

  // Defect 5, half 1: `writingImage`'s schema declares `draggable: true`, so
  // prosemirror-view's mousedown handling arms `figure.draggable = true`
  // (temporarily via its own `mightDrag` bookkeeping on the very first click,
  // and persistently via `selectNode()` once the node is the selected node —
  // both paths converge on the same figure element, since `nodeDOM` for a
  // contentDOM-bearing node view is always the outer `dom`, never the
  // contentDOM). Only the resize handle's gesture was ever exempted from the
  // native drag this produces; a click landing inside the caption — the only
  // way to place a caret in it — was not, so the browser's native drag
  // hijacks the gesture before a caret can land, and the caption is
  // unreachable. `dragstart` bubbles, so a gesture beginning inside the
  // caption fires the figure's own listener with `event.target` still the
  // caption element that started it.
  it('cancels the dragstart the browser would fire when the gesture begins inside the caption', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    const figcaption = figure?.querySelector('figcaption')
    if (!figure || !figcaption) throw new Error('no writingImage node view mounted')

    const dragstart = new Event('dragstart', { bubbles: true, cancelable: true })
    figcaption.dispatchEvent(dragstart)
    expect(dragstart.defaultPrevented).toBe(true)
  })
})

// Defect 4: `title` renders as an HTML tooltip, invisible in the document —
// it is not the caption. The figcaption (the node's own inline content,
// `contentDOM`) is the real, visible caption.
describe('the caption, not the title field, is the visible caption (defect 4)', () => {
  it('marks the caption empty until it has real content, and clears the mark once it does', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    const figcaption = figure?.querySelector('figcaption')
    if (!figure || !figcaption) throw new Error('no writingImage node view mounted')

    expect('empty' in figcaption.dataset).toBe(true)

    instance.chain().insertContent('un pie de foto').run()
    expect('empty' in figcaption.dataset).toBe(false)
  })

  it('marks the caption empty again once its text is deleted back out', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    instance.chain().insertContent('x').run()
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    const figcaption = figure?.querySelector('figcaption')
    if (!figure || !figcaption) throw new Error('no writingImage node view mounted')
    expect('empty' in figcaption.dataset).toBe(false)

    // A real backward-character delete is native contentEditable behaviour
    // in a browser (a `beforeinput`/DOM mutation happy-dom never fires),
    // not a keymap command — even ProseMirror's own default keymap does not
    // bind it. `deleteRange` produces the same document-level step a real
    // backspace would end up producing, without depending on that native
    // path.
    const to = instance.state.selection.to
    instance.commands.deleteRange({ from: to - 1, to })
    expect('empty' in figcaption.dataset).toBe(true)
  })

  it('carries a default caption placeholder with no imageLabels option passed', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    const figcaption = figure?.querySelector('figcaption')
    if (!figure || !figcaption) throw new Error('no writingImage node view mounted')

    expect(figcaption.dataset.placeholder).toBe(DEFAULT_WRITING_IMAGE_LABELS.captionPlaceholder)
  })

  it('still carries the title attribute — serialization and export are unchanged — even with no UI left to edit it', () => {
    const instance = mount()
    instance
      .chain()
      .focus()
      .insertWritingImage({ src: 'writing-images/abc.png', title: 'Un título' })
      .run()

    const json = instance.getJSON()
    const figure = json.content?.find((node) => node.type === 'writingImage')
    expect(figure?.attrs?.title).toBe('Un título')

    // The DOM img still carries it too — it is an HTML tooltip, and stays one.
    const img = instance.view.dom.querySelector<HTMLImageElement>('[data-writing-image] img')
    expect(img?.title).toBe('Un título')
  })
})

// Defect 5 (this round): the empty caption's placeholder was visible only
// while `.ProseMirror-selectednode` held. Clicking the placeholder to type
// into it places a caret, which replaces the NodeSelection with a
// TextSelection and clears that class — the placeholder vanished at the
// exact moment it was clicked, and the caption was unreachable in practice.
// The node view now also tracks whether the current selection sits inside
// its own document range (isSelectionInsideWritingImageNode,
// writing-image-node-view.ts) and reflects that as `data-caret-inside` on
// the figure; the CSS this feeds (WritingEditor.svelte) is out of reach in
// this plain-Editor test file the same way the ProseMirror-selectednode CSS
// already is (see "the node view chrome (I1/I2/I3)" above) — what is
// directly asserted here is the contract that CSS rule depends on.
describe('the empty caption stays reachable while the caret is inside it (defect 5)', () => {
  it('sets data-caret-inside once a TextSelection lands inside the (empty) caption', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    const pos = writingImagePos(instance)
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    if (!figure) throw new Error('no writingImage node view mounted')

    // The image lands as the document's very first node here (pos 0), with
    // TrailingParagraph's own empty paragraph appended right after it — the
    // one position in this document provably outside the image's own range;
    // position 0 itself does not qualify, since ProseMirror's own nearest-
    // valid-position snapping resolves it straight back into the (empty)
    // caption when the image is the document's first block.
    instance.commands.setTextSelection(instance.state.doc.content.size)
    expect('caretInside' in figure.dataset).toBe(false)

    // pos+1 is inside the (empty) caption's own content range.
    instance.commands.setTextSelection(pos + 1)
    expect('caretInside' in figure.dataset).toBe(true)
  })

  it('clears data-caret-inside once the caret leaves the node entirely', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    const pos = writingImagePos(instance)
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    if (!figure) throw new Error('no writingImage node view mounted')

    instance.commands.setTextSelection(pos + 1)
    expect('caretInside' in figure.dataset).toBe(true)

    instance.commands.setTextSelection(instance.state.doc.content.size)
    expect('caretInside' in figure.dataset).toBe(false)
  })

  it('does not set data-caret-inside for a NodeSelection of the whole figure — that stays ProseMirror-selectednode’s job', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    const pos = writingImagePos(instance)
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    if (!figure) throw new Error('no writingImage node view mounted')

    instance.commands.setNodeSelection(pos)
    expect(instance.state.selection.toJSON().type).toBe('node')
    expect('caretInside' in figure.dataset).toBe(false)
  })

  it('keeps tracking data-caret-inside correctly after the caption gains text (nodeSize changes)', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    const pos = writingImagePos(instance)
    const figure = instance.view.dom.querySelector<HTMLElement>('[data-writing-image]')
    if (!figure) throw new Error('no writingImage node view mounted')

    instance.commands.setTextSelection(pos + 1)
    instance.chain().insertContent('un pie de foto').run()
    // The caret, still inside the caption after typing, must still read as inside.
    expect('caretInside' in figure.dataset).toBe(true)

    // Now a caret past the (now longer) node's end must read as outside —
    // proving the check re-reads the node's *current* size, not a stale one
    // captured when the view was first created.
    const afterNodeEnd = pos + instance.state.doc.nodeAt(pos)!.nodeSize
    instance.commands.setTextSelection(afterNodeEnd)
    expect('caretInside' in figure.dataset).toBe(false)
  })

  it('unsubscribes its selectionUpdate listener when the editor is destroyed (no per-node-view leak)', () => {
    const instance = mount()
    const onSpy = vi.spyOn(instance, 'on')

    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()

    const registration = onSpy.mock.calls.find(([event]) => event === 'selectionUpdate')
    expect(registration).toBeDefined()
    const handler = registration?.[1]

    const offSpy = vi.spyOn(instance, 'off')
    instance.destroy()
    editor = undefined // already destroyed here — afterEach must not destroy it again

    expect(offSpy).toHaveBeenCalledWith('selectionUpdate', handler)
  })
})
