import { describe, expect, it } from 'vitest'
import {
  parseOcrRegionReference,
  renderOcrMarkup,
  replaceOcrRegionPlaceholders,
  sanitizeOcrHtml,
} from './ocr-rich-text'

describe('parseOcrRegionReference', () => {
  it('parses zero-based page and left/top/right/bottom bbox values', () => {
    expect(parseOcrRegionReference('page=0,bbox=[536,508,1507,1112]')).toEqual({
      source: 'page=0,bbox=[536,508,1507,1112]',
      page: 0,
      bbox: { left: 536, top: 508, right: 1507, bottom: 1112 },
    })
  })

  it('rejects malformed, reversed, negative, and non-finite regions', () => {
    expect(parseOcrRegionReference('page=1,bbox=[1,2,3]')).toBeNull()
    expect(parseOcrRegionReference('page=1,bbox=[2,2,1,3]')).toBeNull()
    expect(parseOcrRegionReference('page=-1,bbox=[1,2,3,4]')).toBeNull()
    expect(parseOcrRegionReference('page=1,bbox=[1,2,Infinity,4]')).toBeNull()
  })
})

describe('renderOcrMarkup', () => {
  it('renders Markdown and mixed centered HTML while extracting image placeholders', () => {
    const result = renderOcrMarkup(
      '<div align="center">\n\n# Título\n\n</div>\n\n**texto**\n\n![](page=0,bbox=[10,20,30,40])'
    )

    expect(result.html).toContain('<h1>Título</h1>')
    expect(result.html).toContain('<strong>texto</strong>')
    expect(result.html).toContain('data-ocr-region-token=')
    expect(result.references).toHaveLength(1)
  })

  it('renders GFM tables and keeps malformed image syntax as text', () => {
    const result = renderOcrMarkup(
      '| A | B |\n| --- | --- |\n| 1 | 2 |\n\n![](page=0,bbox=[1,2,3])'
    )

    expect(result.html).toContain('<table>')
    expect(result.html).toContain('<td>1</td>')
    expect(result.references).toHaveLength(0)
    expect(result.html).toContain('page=0,bbox=[1,2,3]')
  })
})

describe('sanitizeOcrHtml', () => {
  it('keeps semantic OCR elements and safe centered alignment', () => {
    const result = sanitizeOcrHtml(
      '<div align="center"><h2>Seguro</h2><table><tr><td>1</td></tr></table></div>'
    )

    expect(result).toContain('<div align="center">')
    expect(result).toContain('<h2>Seguro</h2>')
    expect(result).toContain('<table>')
  })

  it('removes executable tags, handlers, styles, and unsafe links', () => {
    const result = sanitizeOcrHtml(
      '<script>alert(1)</script><p onclick="alert(2)" style="color:red">Texto</p><a href="javascript:alert(3)">link</a>'
    )

    expect(result).not.toContain('<script')
    expect(result).not.toContain('onclick')
    expect(result).not.toContain('style=')
    expect(result).not.toContain('javascript:')
    expect(result).toContain('Texto')
    expect(result).toContain('link')
  })

  it('replaces a failed region with escaped original source', () => {
    const markup = renderOcrMarkup('antes ![](page=4,bbox=[1,2,3,4]) después')
    const html = replaceOcrRegionPlaceholders(
      markup.html,
      new Map([[markup.references[0]!.token, '&lt;regiona&gt;']])
    )

    expect(sanitizeOcrHtml(html)).toContain('&lt;regiona&gt;')
  })
})
