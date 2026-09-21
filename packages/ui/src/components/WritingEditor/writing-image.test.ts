import { Editor } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
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
})
