import { render } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import WritingEditor from './WritingEditor.svelte'
import { WRITING_SCHEMA_VERSION, type CanonicalDocument } from './document-contract'

/**
 * An OS file drop never reaches this component's own `handleDrop` plugin —
 * that plugin reads `event.dataTransfer.files`, and Tauri v2's
 * `dragDropEnabled` default intercepts the OS drag before the webview's own
 * HTML5 drop ever fires (see extensions.ts's removed `handleDrop`, and
 * apps/desktop's WritingView.svelte, which now listens to Tauri's own
 * `onDragDropEvent` instead). What that Tauri-aware caller needs from this
 * Tauri-free package is a plain, viewport-coordinate seam: "is this point
 * over the manuscript", "what document position is under it", and "insert
 * here instead of at the caret" — the same vocabulary every other pointer
 * interaction in this component already uses (resize handles, toolbar
 * measurement), not a Tauri type in sight.
 */

function manuscript(text: string): CanonicalDocument {
  return {
    schemaVersion: WRITING_SCHEMA_VERSION,
    doc: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text }] }] },
  }
}

function renderEditor(content: string) {
  return render(WritingEditor, { props: { document: manuscript(content) } })
}

/** The manuscript surface sits at this fixed viewport rect; every other
 *  element (toolbar buttons, panels) reports the zero rect happy-dom gives
 *  everything by default, which is itself the point: coordinates over
 *  anything that is not the manuscript host must not read as "inside". */
const HOST_RECT = { top: 100, left: 50, right: 450, bottom: 500 }

function rectFor(el: HTMLElement) {
  if (!el.classList.contains('writing-editor__host')) {
    return { top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0, x: 0, y: 0, toJSON() {} }
  }
  return {
    ...HOST_RECT,
    width: HOST_RECT.right - HOST_RECT.left,
    height: HOST_RECT.bottom - HOST_RECT.top,
    x: HOST_RECT.left,
    y: HOST_RECT.top,
    toJSON() {},
  }
}

beforeEach(() => {
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockImplementation(function (
    this: HTMLElement
  ) {
    return rectFor(this) as DOMRect
  })
})

afterEach(() => {
  vi.restoreAllMocks()
})

describe('WritingEditor: locating a drop over the manuscript', () => {
  it('reports a point inside the manuscript surface as contained', () => {
    const { component } = renderEditor('hola mundo')

    expect(component.containsPoint(200, 300)).toBe(true)
  })

  it('reports a point outside the manuscript surface as not contained', () => {
    const { component } = renderEditor('hola mundo')

    expect(component.containsPoint(10, 10)).toBe(false)
    expect(component.containsPoint(1000, 1000)).toBe(false)
  })

  it('never throws resolving a document position, whatever the coordinates', () => {
    const { component } = renderEditor('hola mundo')

    const pos = component.posAtCoords(200, 300)

    expect(pos === null || typeof pos === 'number').toBe(true)
  })
})

describe('WritingEditor: inserting an image at an explicit position', () => {
  it('lands at the given position, splitting the text there, not at the caret', () => {
    const { component } = renderEditor('hola mundo')
    // "hola mundo" as a paragraph: doc position 1 is the paragraph's start,
    // position 6 is right after "hola ". The caret is untouched (never
    // focused/moved here) — only an explicit `at` should be able to reach 6.
    const ok = component.insertImage({ src: 'writing-images/abc.png', height: 5 }, 6)

    expect(ok).toBe(true)
    const host = document.querySelector('.writing-editor__host') as HTMLElement
    const figure = host.querySelector('figure[data-writing-image]')
    expect(figure).not.toBeNull()
    // The split puts "hola " (position 6 is right after the space) before
    // the figure and "mundo" after it.
    expect(figure?.previousElementSibling?.textContent).toBe('hola ')
    expect(figure?.nextElementSibling?.textContent).toBe('mundo')
  })

  it('falls back to the caret when no explicit position is given, exactly as before', () => {
    const { component } = renderEditor('hola mundo')

    const ok = component.insertImage({ src: 'writing-images/abc.png', height: 5 })

    expect(ok).toBe(true)
    const host = document.querySelector('.writing-editor__host') as HTMLElement
    expect(host.querySelector('figure[data-writing-image]')).not.toBeNull()
  })
})
