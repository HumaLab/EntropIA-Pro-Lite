import { afterEach, describe, expect, it, vi } from 'vitest'
import { pdfDocumentOptions } from './pdf-document-options'

/**
 * The helper reads the ambient `navigator`, so each test installs exactly the
 * engine signal it is about and restores the real one afterwards. Stubbing the
 * global is safe here: this file owns no other global stubs.
 */
function stubNavigator(value: unknown) {
  vi.stubGlobal('navigator', value)
}

afterEach(() => {
  vi.unstubAllGlobals()
})

describe('pdfDocumentOptions', () => {
  it('enables the native image decoder on Chromium 154 (userAgentData brands)', () => {
    stubNavigator({
      userAgentData: {
        brands: [
          { brand: 'Chromium', version: '154' },
          { brand: 'Microsoft Edge', version: '154' },
          { brand: 'Not:A-Brand', version: '24' },
        ],
      },
    })

    expect(pdfDocumentOptions('asset://doc.pdf')).toEqual({
      url: 'asset://doc.pdf',
      isImageDecoderSupported: true,
    })
  })

  it('omits the option on Chromium 133 (userAgentData brands)', () => {
    stubNavigator({
      userAgentData: {
        brands: [
          { brand: 'Chromium', version: '133' },
          { brand: 'Google Chrome', version: '133' },
        ],
      },
    })

    expect(pdfDocumentOptions('asset://doc.pdf')).toEqual({ url: 'asset://doc.pdf' })
  })

  it('enables the option exactly at the 134 threshold', () => {
    stubNavigator({
      userAgentData: {
        brands: [{ brand: 'Chromium', version: '134' }],
      },
    })

    expect(pdfDocumentOptions('asset://doc.pdf')).toEqual({
      url: 'asset://doc.pdf',
      isImageDecoderSupported: true,
    })
  })

  it('falls back to the user-agent string and enables the option for Edg/154', () => {
    stubNavigator({
      userAgent:
        'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 ' +
        '(KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36 Edg/154.0.2903.86',
    })

    expect(pdfDocumentOptions('asset://doc.pdf')).toEqual({
      url: 'asset://doc.pdf',
      isImageDecoderSupported: true,
    })
  })

  it('omits the option for a Chrome/120 user agent', () => {
    stubNavigator({
      userAgent:
        'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 ' +
        '(KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36',
    })

    expect(pdfDocumentOptions('asset://doc.pdf')).toEqual({ url: 'asset://doc.pdf' })
  })

  it('omits the option for a Firefox user agent', () => {
    stubNavigator({
      userAgent: 'Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:134.0) Gecko/20100101 Firefox/134.0',
    })

    expect(pdfDocumentOptions('asset://doc.pdf')).toEqual({ url: 'asset://doc.pdf' })
  })

  it('omits the option when navigator is missing', () => {
    vi.stubGlobal('navigator', undefined)

    expect(pdfDocumentOptions('asset://doc.pdf')).toEqual({ url: 'asset://doc.pdf' })
  })

  it('omits the option when the engine cannot be parsed', () => {
    stubNavigator({
      userAgentData: {
        brands: [{ brand: 'Chromium', version: 'not-a-version' }],
      },
      userAgent:
        'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 ' +
        '(KHTML, like Gecko) Chrome/not-a-version Safari/537.36',
    })

    expect(pdfDocumentOptions('asset://doc.pdf')).toEqual({ url: 'asset://doc.pdf' })
  })

  it('omits the option for non-Chromium engines even at a high version', () => {
    stubNavigator({
      userAgentData: {
        brands: [{ brand: 'Not:A-Brand', version: '999' }],
      },
      userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) SomeEngine/999.0',
    })

    expect(pdfDocumentOptions('asset://doc.pdf')).toEqual({ url: 'asset://doc.pdf' })
  })

  it('keeps the requested url in every answer', () => {
    stubNavigator(undefined)
    expect(pdfDocumentOptions('asset://otro.pdf').url).toBe('asset://otro.pdf')
  })
})
