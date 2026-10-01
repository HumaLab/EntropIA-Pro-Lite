/**
 * The backend reports failures as stable codes and the UI turns them into
 * messages. Two lists in two languages have to agree, so these tests read both
 * (the Rust source and the TS module) instead of restating either.
 */
import { describe, expect, it } from 'vitest'
import captureRs from '../../src-tauri/src/navegador/capture.rs?raw'
import downloadRs from '../../src-tauri/src/navegador/download.rs?raw'
import saveRs from '../../src-tauri/src/navegador/save.rs?raw'
import { locale, t } from './i18n'
import {
  CAPTURE_ERROR_CODES,
  DOWNLOAD_REASON_CODES,
  SAVE_ERROR_CODES,
  downloadReasonKey,
  parseCaptureError,
} from './navegador-capture'

/** The string values of `pub const X: &str = "value";` inside `pub mod <name>`. */
function codesIn(source: string, moduleName: string): string[] {
  const start = source.indexOf(`pub mod ${moduleName} {`)
  expect(start, `pub mod ${moduleName} not found`).toBeGreaterThanOrEqual(0)
  const end = source.indexOf('\n}', start)
  const body = source.slice(start, end)
  return [...body.matchAll(/pub const [A-Z_]+: &str = "([a-z_]+)";/g)].map((m) => m[1]!)
}

describe('capture error codes', () => {
  it('are the ones capture.rs can produce', () => {
    const rust = codesIn(captureRs, 'code').sort()
    expect([...CAPTURE_ERROR_CODES].sort()).toEqual(rust)
  })

  it.each(['es', 'en'] as const)('have a message in %s', (language) => {
    locale.set(language)
    for (const code of [...CAPTURE_ERROR_CODES, 'unknown']) {
      const key = `navegador.capture.error.${code}`
      expect(t(key, { message: 'x' }), key).not.toBe(key)
    }
  })

  it('reach the UI as the code the backend printed', () => {
    for (const code of CAPTURE_ERROR_CODES) {
      expect(parseCaptureError(code).code).toBe(code)
    }
  })
})

describe('download reason codes', () => {
  it('are the ones download.rs can produce', () => {
    const rust = codesIn(downloadRs, 'reason').sort()
    expect([...DOWNLOAD_REASON_CODES].sort()).toEqual(rust)
  })

  it.each(['es', 'en'] as const)('have a message in %s', (language) => {
    locale.set(language)
    for (const code of [...DOWNLOAD_REASON_CODES, 'unknown']) {
      const key = `navegador.download.reason.${code}`
      expect(t(key), key).not.toBe(key)
    }
  })

  it('map an unrecognised reason to the unknown message, never to a made-up key', () => {
    expect(downloadReasonKey('not_pdf')).toBe('navegador.download.reason.not_pdf')
    expect(downloadReasonKey('../../x')).toBe('navegador.download.reason.unknown')
    expect(downloadReasonKey(null)).toBe('navegador.download.reason.unknown')
  })
})

describe('save error codes', () => {
  it('are the ones save.rs can produce', () => {
    const rust = codesIn(saveRs, 'code').sort()
    expect([...SAVE_ERROR_CODES].sort()).toEqual(rust)
  })

  it.each(['es', 'en'] as const)('have a message in %s', (language) => {
    locale.set(language)
    for (const code of [...SAVE_ERROR_CODES, 'unknown']) {
      const key = `navegador.save.error.${code}`
      expect(t(key, { message: 'x' }), key).not.toBe(key)
    }
  })
})

describe('status and kind labels', () => {
  it.each(['es', 'en'] as const)('exist in %s', (language) => {
    locale.set(language)
    for (const key of [
      'navegador.capture.kind.page',
      'navegador.capture.kind.selection',
      'navegador.capture.hash.html',
      'navegador.capture.hash.quote',
      'navegador.download.status.downloading',
      'navegador.download.status.ready',
      'navegador.download.status.rejected',
      'navegador.download.status.failed',
      'navegador.save.action',
      'navegador.save.saving',
      'navegador.save.saved',
    ]) {
      expect(t(key), key).not.toBe(key)
    }
  })
})
