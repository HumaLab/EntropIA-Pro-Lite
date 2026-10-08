/**
 * Shared pdf.js document options.
 *
 * Every `getDocument` call in the app goes through {@link pdfDocumentOptions}
 * so no viewer can end up fast while another stays slow (plan-pdf.md, fase 1).
 */

export interface PdfDocumentOptions {
  url: string
  isImageDecoderSupported?: boolean
}

/**
 * Chromium majors below this must NOT get `isImageDecoderSupported: true`:
 * pdf.js disabled the native `ImageDecoder` there because of two Chromium bugs
 * (see pdfjs-dist `display/api.d.ts`), both fixed around Chrome 132–133 and
 * re-enabled by default in mozilla/pdf.js#20961 once its supported minimum
 * reached Chrome 125:
 *
 * - crbug 374807001 — the BMP decoder crashes the process on huge images
 *   (e.g. issue6741.pdf). A process crash bypasses pdf.js error handling, so
 *   there is no ordered fallback to the slow JS decoder.
 * - crbug 378869810 — the JPEG decoder draws broken images that carry their
 *   own colour profile.
 *
 * 134 gives a one-version safety margin over the 132–133 fixes.
 */
export const IMAGE_DECODER_MIN_CHROMIUM_MAJOR = 134

/** UA-CH brands that identify a Chromium engine and carry its major version. */
const CHROMIUM_BRANDS = ['Chromium', 'Google Chrome', 'Microsoft Edge']

/** Edge reports `Edg/NNN` (its engine version) alongside a stale `Chrome/NNN`
 *  compatibility token, so Edge must win when both appear. Plain Chrome and
 *  other Chromium forks report only `Chrome/NNN`. */
const UA_EDGE_MAJOR = /\bEdg\/(\d+)/
const UA_CHROME_MAJOR = /\bChrome\/(\d+)/

interface UserAgentDataLike {
  brands?: ReadonlyArray<{ brand?: string; version?: string }>
}

interface NavigatorLike {
  userAgentData?: UserAgentDataLike
  userAgent?: string
}

function majorFromBrands(brands: UserAgentDataLike['brands']): number | null {
  if (!Array.isArray(brands)) return null
  for (const entry of brands) {
    const brand = entry?.brand?.toLowerCase()
    if (!brand || !CHROMIUM_BRANDS.some((candidate) => candidate.toLowerCase() === brand)) continue
    const major = Number.parseInt(entry.version ?? '', 10)
    if (Number.isFinite(major)) return major
  }
  return null
}

function majorFromUserAgent(userAgent: string | undefined): number | null {
  if (!userAgent) return null
  const match = UA_EDGE_MAJOR.exec(userAgent) ?? UA_CHROME_MAJOR.exec(userAgent)
  if (!match) return null
  const major = Number.parseInt(match[1] ?? '', 10)
  return Number.isFinite(major) ? major : null
}

function chromiumMajor(nav: NavigatorLike | undefined): number | null {
  if (!nav) return null
  return majorFromBrands(nav.userAgentData?.brands) ?? majorFromUserAgent(nav.userAgent)
}

/**
 * pdf.js `getDocument` parameters for `url`: `{ url, isImageDecoderSupported:
 * true }` on Chromium >= 134, plain `{ url }` anywhere else. Non-Chromium and
 * unparseable engines keep the pdf.js default (the slow JS decoder).
 */
export function pdfDocumentOptions(url: string): PdfDocumentOptions {
  const nav = (globalThis as { navigator?: NavigatorLike }).navigator
  const major = chromiumMajor(nav)
  return major !== null && major >= IMAGE_DECODER_MIN_CHROMIUM_MAJOR
    ? { url, isImageDecoderSupported: true }
    : { url }
}
