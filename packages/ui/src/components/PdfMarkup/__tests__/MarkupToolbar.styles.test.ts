import { describe, expect, it } from 'vitest'
import source from '../MarkupToolbar.svelte?raw'

describe('MarkupToolbar styles', () => {
  it('takes pointer events back from the toolbar anchor, which lets them through', () => {
    // DocumentViewer's toolbar anchor is `pointer-events: none` so its empty
    // space never blocks the page; a toolbar inside it must opt back in, or
    // every click falls through to the PDF underneath. happy-dom does no hit
    // testing, so the rule itself is pinned.
    const rule = source.match(/\n  \.markup-toolbar \{([^}]*)\}/)?.[1] ?? ''
    expect(rule).toMatch(/pointer-events: auto;/)
  })
})
