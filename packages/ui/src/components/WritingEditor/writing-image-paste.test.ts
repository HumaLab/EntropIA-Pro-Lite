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

// There used to be a "dropping an image into the manuscript" suite here,
// driving this plugin's `handleDrop` the same way the paste tests above
// drive `handlePaste`: a duck-typed event handed straight to `someProp`.
// Those tests passed and proved nothing about the real app — Tauri v2's
// `dragDropEnabled` (on by default in both apps/desktop configs, and left
// on deliberately: turning it off would break CollectionView's own working
// file drop) intercepts an OS file drag before the webview's HTML5 drop
// ever fires, so `event.dataTransfer.files` never carries a real file here.
// `handleDrop` itself is gone from extensions.ts for the same reason; the
// coverage for an actual drop now lives at WritingEditor.svelte's own
// `insertImage`/`posAtCoords`/`containsPoint` (WritingEditor.imageDrop.test.ts)
// and at apps/desktop's writing-image-drop.ts, which is what a real Tauri
// drop event now reaches.
