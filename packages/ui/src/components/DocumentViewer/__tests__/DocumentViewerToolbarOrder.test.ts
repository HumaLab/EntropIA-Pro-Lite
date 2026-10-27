import { describe, expect, it } from 'vitest'
import source from '../DocumentViewer.svelte?raw'

describe('DocumentViewer toolbar order', () => {
  it('puts the markup toolbar before the annotation toolbar in the right-aligned anchor', () => {
    // The anchor aligns its toolbars to the right edge. With the wide markup
    // toolbar last, it pushed the zoom column left onto the page.
    const anchor = source.slice(source.indexOf('document-viewer__toolbar-anchor'))
    expect(anchor.indexOf('<MarkupToolbar')).toBeGreaterThan(-1)
    expect(anchor.indexOf('<MarkupToolbar')).toBeLessThan(anchor.indexOf('<AnnotationToolbar'))
  })
})
