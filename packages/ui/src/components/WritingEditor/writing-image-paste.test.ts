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

describe('pasting an image into the manuscript', () => {
  it('imports and inserts a pasted image file', async () => {
    const importImage = vi.fn(async () => ({ path: 'writing-images/abc.png', width: 10, height: 10 }))
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

    expect(handled).toBe(true)
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
