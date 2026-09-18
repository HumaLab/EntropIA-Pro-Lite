import { Packer } from 'docx'
import { unzipSync } from 'fflate'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { toDocx } from './export-docx'

/**
 * DOCX export where the app actually runs: a WebView, not Node.
 *
 * # Why the platform is simulated at `Packer.pack`
 *
 * The WebView has no `Buffer`. JSZip, which `docx` bundles, decides once — when
 * the module is first evaluated — whether it can produce a Node buffer, and
 * `Packer.toBuffer` asks it for exactly that. Under Node every test passed while
 * every real export failed with "nodebuffer is not supported by this platform".
 *
 * Hiding `Buffer` before `docx` loads reproduces it faithfully in a bare Node
 * script, but inside a Vitest worker it hangs the worker, whose own plumbing
 * needs `Buffer`. So the WebView is simulated one step down instead: every
 * `Packer.to*` goes through `Packer.pack(file, type)`, and here, as in the
 * WebView, the `nodebuffer` type is refused with JSZip's own message while every
 * browser output type still works.
 */

afterEach(() => {
  vi.restoreAllMocks()
})

function asInTheWebView() {
  const pack = Packer.pack.bind(Packer)
  vi.spyOn(Packer, 'pack').mockImplementation(async (file, type, ...rest) => {
    if (type === 'nodebuffer') throw new Error('nodebuffer is not supported by this platform')
    return pack(file, type, ...rest)
  })
}

describe('a DOCX export in a WebView', () => {
  it('produces the package without asking for a Node buffer', async () => {
    asInTheWebView()

    const bytes = await toDocx(
      { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text: 'hola' }] }] },
      {
        title: '',
        citations: 'footnote',
        zotero: {},
        bibliography: [],
        bibliographyHeading: 'Bibliografía',
      }
    )

    expect(bytes).toBeInstanceOf(Uint8Array)
    expect(Object.keys(unzipSync(bytes))).toContain('word/document.xml')
  })
})
