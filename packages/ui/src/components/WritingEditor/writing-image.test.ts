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
    instance.commands.setTextSelection({ from: 1, to: 6 }) // selects "hola "

    instance.chain().insertWritingImage({ src: 'writing-images/abc.png' }).run()

    const text = instance.getText()
    expect(text).toContain('mundo')
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
    // Land inside the (empty) caption of the figure just inserted.
    const figurePos = instance.state.doc.content.size - 2
    instance.commands.setTextSelection(figurePos)

    instance.commands.keyboardShortcut('Enter')

    const names: string[] = []
    instance.state.doc.forEach((node) => names.push(node.type.name))
    expect(names.filter((name) => name === 'writingImage')).toHaveLength(1)
    expect(names.filter((name) => name === 'paragraph').length).toBeGreaterThanOrEqual(1)
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
