import { describe, expect, it, vi } from 'vitest'
import {
  parseOcrRegionReference,
  renderOcrHtml,
  renderOcrMarkup,
  replaceOcrRegionPlaceholders,
  replaceOcrRegionReferences,
  resolveOcrRegion,
  sanitizeOcrHtml,
  scaleOcrBbox,
  type OcrRenderContext,
  type OcrRegionResolver,
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

describe('replaceOcrRegionReferences', () => {
  it('assigns distinct tokens to repeated valid OCR references', () => {
    const tokens: string[] = []
    const source = '![](page=0,bbox=[1,2,3,4]) ![](page=0,bbox=[1,2,3,4])'

    const replaced = replaceOcrRegionReferences(source, (reference) => {
      tokens.push(reference.token)
      return `<${reference.token}>`
    })

    expect(tokens).toEqual(['region-0', 'region-1'])
    expect(replaced).toBe('<region-0> <region-1>')
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

describe('scaleOcrBbox', () => {
  it('maps OCR reference coordinates to the raster dimensions', () => {
    expect(
      scaleOcrBbox(
        { left: 100, top: 200, right: 500, bottom: 700 },
        1000,
        2000,
        2000,
        4000
      )
    ).toEqual({ left: 200, top: 400, width: 800, height: 1000 })
  })

  it('rejects missing dimensions and out-of-range coordinates', () => {
    expect(() =>
      scaleOcrBbox({ left: 0, top: 0, right: 10, bottom: 10 }, 0, 100, 100, 100)
    ).toThrow(/reference dimensions/i)
    expect(() =>
      scaleOcrBbox({ left: 0, top: 0, right: 101, bottom: 10 }, 100, 100, 100, 100)
    ).toThrow(/bounds/i)
  })
})

describe('renderOcrHtml', () => {
  const context: OcrRenderContext = {
    assetUrl: 'asset://source',
    sourceType: 'image',
    referenceWidth: 100,
    referenceHeight: 100,
  }

  it('places resolved data URLs inline and keeps source order', async () => {
    const resolver: OcrRegionResolver = async () => 'data:image/png;base64,AAAA'
    const html = await renderOcrHtml(
      'antes ![](page=0,bbox=[10,20,30,40]) después',
      context,
      resolver
    )

    expect(html).toContain('src="data:image/png;base64,AAAA"')
    expect(html.indexOf('antes')).toBeLessThan(html.indexOf('<img'))
    expect(html.indexOf('<img')).toBeLessThan(html.indexOf('después'))
  })

  it('uses a discreet fallback and logs the failed region when a resolver rejects', async () => {
    const resolver: OcrRegionResolver = async () => {
      throw new Error('source unavailable')
    }
    const warning = vi.spyOn(console, 'warn').mockImplementation(() => undefined)

    try {
      const html = await renderOcrHtml(
        'antes ![](page=0,bbox=[10,20,30,40]) después',
        context,
        resolver
      )

      expect(html).toContain('ocr-region-fallback')
      expect(html).toContain('aria-label="OCR image unavailable"')
      expect(html).not.toContain('![](page=0,bbox=[10,20,30,40])')
      expect(html).toContain('antes')
      expect(html).toContain('después')
      expect(warning).toHaveBeenCalledWith(
        '[OcrRichText] Unable to resolve OCR image region',
        expect.objectContaining({
          sourceType: 'image',
          page: 0,
          bbox: { left: 10, top: 20, right: 30, bottom: 40 },
          error: 'source unavailable',
        })
      )
    } finally {
      warning.mockRestore()
    }
  })
})

describe('resolveOcrRegion', () => {
  it('loads source images with anonymous CORS before drawing a crop', async () => {
    class FakeImage {
      static instances: FakeImage[] = []
      decoding = ''
      crossOrigin = ''
      naturalWidth = 2000
      naturalHeight = 1500
      width = 2000
      height = 1500
      src = ''

      constructor() {
        FakeImage.instances.push(this)
      }

      async decode(): Promise<void> {}
    }

    const context = {
      drawImage: vi.fn(),
    } as unknown as CanvasRenderingContext2D
    const getContext = vi
      .spyOn(HTMLCanvasElement.prototype, 'getContext')
      .mockReturnValue(context)
    const toDataURL = vi
      .spyOn(HTMLCanvasElement.prototype, 'toDataURL')
      .mockReturnValue('data:image/png;base64,AAAA')

    vi.stubGlobal('Image', FakeImage)
    try {
      const result = await resolveOcrRegion(
        {
          token: 'region-0',
          source: '![](page=0,bbox=[536,508,1507,1112])',
          page: 0,
          bbox: { left: 536, top: 508, right: 1507, bottom: 1112 },
        },
        {
          assetUrl: 'asset://cors-image',
          sourceType: 'image',
          referenceWidth: 2000,
          referenceHeight: 1500,
        }
      )

      expect(result).toBe('data:image/png;base64,AAAA')
      expect(FakeImage.instances[0]?.crossOrigin).toBe('anonymous')
      expect(context.drawImage).toHaveBeenCalled()
    } finally {
      getContext.mockRestore()
      toDataURL.mockRestore()
      vi.unstubAllGlobals()
    }
  })
})
