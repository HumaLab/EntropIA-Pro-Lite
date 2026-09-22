import { Editor } from '@tiptap/core'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createWritingExtensions } from './extensions'
import { emptyDocument } from './document-contract'

let editor: Editor | undefined

afterEach(() => {
  editor?.destroy()
  editor = undefined
})

function mount(importImage: ((bytes: Uint8Array) => Promise<{ path: string; width: number; height: number } | null>) | undefined) {
  const element = document.createElement('div')
  document.body.appendChild(element)
  editor = new Editor({
    element,
    extensions: createWritingExtensions({ importImage }),
    content: emptyDocument().doc,
  })
  return editor
}

function pngFile(): File {
  const bytes = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])
  return new File([bytes], 'pasted.png', { type: 'image/png' })
}

function pasteEvent(files: File[], html?: string): ClipboardEvent {
  const data = new DataTransfer()
  files.forEach((file) => data.items.add(file))
  if (html) data.setData('text/html', html)
  const event = new ClipboardEvent('paste', { clipboardData: data, cancelable: true })
  return event
}

function textFile(): File {
  return new File(['hola'], 'notes.txt', { type: 'text/plain' })
}

/**
 * happy-dom 17.6.3's `DragEvent` inherits the bare `Event` constructor and
 * never reads `eventInit.dataTransfer` — a real `new DragEvent('drop', {
 * dataTransfer })` arrives at the handler with `dataTransfer` stuck at
 * `null`, which cannot exercise either branch. Reaching `handlePaste` above
 * already goes through `someProp` directly rather than dispatching a native
 * event, so a duck-typed object shaped like a drop event carries the same
 * property `handleDrop` reads (`dataTransfer.files`) without needing a
 * construct the sandbox cannot build.
 */
function dropEvent(files: File[]): { dataTransfer: { files: File[] }; clientX: number; clientY: number; preventDefault: () => void } {
  return {
    dataTransfer: { files },
    clientX: 0,
    clientY: 0,
    preventDefault: () => {},
  }
}

/** The (only) writingImage node in the document, or null — I8: the
 *  behavioural tests below assert the document afterwards, not just
 *  `handled` and whether `importImage` was called. A regression that claims
 *  the event, imports the bytes and inserts nothing passed every assertion
 *  in this file before this addition. */
function findWritingImage(instance: Editor): { attrs: Record<string, unknown> } | null {
  let found: { attrs: Record<string, unknown> } | null = null
  instance.state.doc.descendants((node) => {
    if (node.type.name === 'writingImage') found = { attrs: { ...node.attrs } }
    return found === null
  })
  return found
}

describe('pasting an image into the manuscript', () => {
  it('imports and inserts a pasted image file', async () => {
    const importImage = vi.fn(async () => ({ path: 'writing-images/abc.png', width: 999, height: 10 }))
    const instance = mount(importImage)
    instance.commands.focus()

    // someProp infers `fn` as ProseMirror's full three-arg handlePaste
    // signature (view, event, slice). Every call site here only needs the
    // first two — the `any` sidesteps the arity mismatch that would
    // otherwise be a compile error, and matches this repo's convention of
    // not fighting inferred callback types with contortions (AGENTS.md).
    const handled = instance.view.someProp('handlePaste', (fn: any) =>
      fn(instance.view, pasteEvent([pngFile()]))
    )
    await vi.waitFor(() => expect(importImage).toHaveBeenCalled())
    // importAndInsert dispatches asynchronously, after handlePaste's own
    // synchronous return — the document is not updated yet the instant
    // `someProp` returns.
    await vi.waitFor(() => expect(findWritingImage(instance)).not.toBeNull())

    expect(handled).toBe(true)
    const node = findWritingImage(instance)
    expect(node?.attrs.src).toBe('writing-images/abc.png')
    // I4/C1: constructed through insertWritingImage itself, which defaults
    // width to null — the imported bytes' intrinsic width (999) is never
    // taken as the node's chosen width.
    expect(node?.attrs.width).toBeNull()
    expect(node?.attrs.height).toBe(10)
    expect(node?.attrs.align).toBe('center')
  })

  it('does not intercept a plain text paste', () => {
    const importImage = vi.fn()
    const instance = mount(importImage)
    instance.commands.focus()

    const handled = instance.view.someProp('handlePaste', (fn: any) =>
      fn(instance.view, pasteEvent([]))
    )

    expect(handled).toBeFalsy()
    expect(importImage).not.toHaveBeenCalled()
  })

  it('never produces a node from a remote or foreign-filesystem img with no accompanying bytes', () => {
    const importImage = vi.fn()
    const instance = mount(importImage)
    instance.commands.focus()

    instance.view.someProp('handlePaste', (fn: any) =>
      fn(instance.view, pasteEvent([], '<img src="https://example.org/remote.png">'))
    )

    const names: string[] = []
    instance.state.doc.forEach((node) => names.push(node.type.name))
    expect(names).not.toContain('writingImage')
    expect(importImage).not.toHaveBeenCalled()
  })
})

describe('dropping an image into the manuscript', () => {
  it('imports and inserts a dropped image file', async () => {
    const importImage = vi.fn(async () => ({ path: 'writing-images/abc.png', width: 999, height: 10 }))
    const instance = mount(importImage)
    instance.commands.focus()

    const handled = instance.view.someProp('handleDrop', (fn: any) =>
      fn(instance.view, dropEvent([pngFile()]))
    )
    await vi.waitFor(() => expect(importImage).toHaveBeenCalled())
    await vi.waitFor(() => expect(findWritingImage(instance)).not.toBeNull())

    expect(handled).toBe(true)
    const node = findWritingImage(instance)
    expect(node?.attrs.src).toBe('writing-images/abc.png')
    expect(node?.attrs.width).toBeNull()
    expect(node?.attrs.height).toBe(10)
  })

  it('lands where it was dropped, not wherever the cursor happens to be (I4)', async () => {
    const importImage = vi.fn(async () => ({ path: 'writing-images/abc.png', width: 10, height: 10 }))
    const instance = mount(importImage)
    instance.commands.focus()
    instance.chain().insertContent('hola mundo').run()
    // The cursor is now at the end, after "mundo". A drop at a different
    // position must still land there, through insertWritingImage's `at`
    // option (extensions.ts's importAndInsert) — not silently fall back to
    // the current selection the way the old bespoke `tr.insert` path could
    // have, had it read the wrong position.
    vi.spyOn(instance.view, 'posAtCoords').mockReturnValue({ pos: 6, inside: -1 })

    instance.view.someProp('handleDrop', (fn: any) => fn(instance.view, dropEvent([pngFile()])))
    await vi.waitFor(() => expect(importImage).toHaveBeenCalled())
    await vi.waitFor(() => expect(findWritingImage(instance)).not.toBeNull())

    // Splitting "hola mundo" at position 6 (right after "hola ") puts "hola"
    // before the image and "mundo" after it, in a paragraph of its own — the
    // shape only the dropped position, not the end-of-document selection,
    // produces.
    const json = instance.getJSON()
    const types = json.content?.map((node) => node.type) ?? []
    const imageIndex = types.indexOf('writingImage')
    expect(imageIndex).toBeGreaterThan(-1)
    expect(json.content?.[imageIndex - 1]?.content?.[0]?.text).toBe('hola ')
    expect(json.content?.[imageIndex + 1]?.content?.[0]?.text).toBe('mundo')
  })

  it('does not intercept a drop with no accepted image file', () => {
    const importImage = vi.fn()
    const instance = mount(importImage)
    instance.commands.focus()

    const textHandled = instance.view.someProp('handleDrop', (fn: any) =>
      fn(instance.view, dropEvent([textFile()]))
    )
    const emptyHandled = instance.view.someProp('handleDrop', (fn: any) =>
      fn(instance.view, dropEvent([]))
    )

    expect(textHandled).toBeFalsy()
    expect(emptyHandled).toBeFalsy()
    expect(importImage).not.toHaveBeenCalled()
  })
})
