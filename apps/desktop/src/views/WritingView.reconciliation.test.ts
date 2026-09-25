import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * The reconciling effect is the one place the store's open document is
 * decided (its own doc comment says so): every route into Escritura —a
 * card, Back, the section crumb, the top-bar icon, and since the visual
 * polish round also a Home pane redirecting "new document"/a recent writing
 * row to this owner tab— lands here as a `requested` document id that may
 * differ from whatever is currently open.
 *
 * `WritingStore.openDocument` (writing.ts) cancels the autosave timer and
 * replaces `content` outright; it does not itself flush whatever was
 * pending on the document it is leaving. Rendering the view needs the whole
 * store and Tauri behind it (the same reason every other WritingView.*.test
 * stays at the source level), so this checks the fact the way those do: as
 * a fact about the source — see writing.test.ts's "switching documents
 * without flushing first loses a pending edit" for the behavioral proof of
 * why this matters.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'WritingView.svelte'), 'utf-8')

describe('the reconciling effect flushes before switching documents', () => {
  function effectBody(): string {
    const effectAt = SOURCE.indexOf('$effect(() => {\n    const view = navSnapshot')
    expect(effectAt, 'the reconciling effect is missing or moved').toBeGreaterThan(-1)
    return SOURCE.slice(effectAt, SOURCE.indexOf('\n  })', effectAt))
  }

  it('flushes before opening the requested document, guarded on something already being open', () => {
    const body = effectBody()
    const flushAt = body.indexOf('store.flush()')
    const openAt = body.indexOf('store.openDocument(requested)')
    expect(flushAt, 'store.flush() call is missing from the reconciling effect').toBeGreaterThan(-1)
    expect(openAt, 'store.openDocument(requested) call is missing').toBeGreaterThan(-1)

    // Order matters: flushing after the switch already landed is too late —
    // openDocument has already cancelled the old timer and wiped `content`.
    expect(flushAt).toBeLessThan(openAt)

    // Guarded, not unconditional: flushing when nothing was open is a
    // needless no-op call on every navigation into Escritura, not just a
    // document-to-document switch. (`store.flush()` is itself a no-op when
    // nothing is open or nothing is pending, but the guard here still
    // documents the intent and avoids the call entirely on first open.)
    const guardedFlush = /if\s*\(openId\)\s*(?:await\s*)?store\.flush\(\)/.test(body)
    expect(guardedFlush, 'store.flush() should be guarded on openId').toBe(true)
  })
})
