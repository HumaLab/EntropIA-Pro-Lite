import { save } from '@tauri-apps/plugin-dialog'
import { writeFile } from '@tauri-apps/plugin-fs'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Node } from './export-document'
import type { ExportFormat } from './export-fidelity'
import { downloadExport, exportSettingsFor } from './writing-export'

/**
 * The download button's half of an export: a format, chosen now, applied with
 * the preferences chosen earlier in the Export tab.
 *
 * All three formats read the same preferences. There is one configuration, not
 * one per format, so what differs between them is only what a format cannot do.
 */

const FORMATS: ExportFormat[] = ['markdown', 'html', 'docx']

const context = {
  title: 'Capítulo',
  fileName: 'Capítulo',
  style: { kind: 'bundled', name: 'apa' } as const,
  bibliographyHeading: 'Bibliografía',
}

const citing: Node = {
  type: 'doc',
  content: [
    {
      type: 'paragraph',
      content: [
        { type: 'text', text: 'Como dice ' },
        {
          type: 'documentCitation',
          attrs: {
            quotedText: 'el queso y los gusanos',
            pageNumber: 12,
            metadataSnapshot: { title: 'Proceso' },
          },
        },
      ],
    },
  ],
}

beforeEach(() => {
  vi.mocked(save).mockReset()
  vi.mocked(writeFile).mockReset()
})

describe('the settings a download is made with', () => {
  it.each(FORMATS)('carry both preferences into %s', (format) => {
    expect(
      exportSettingsFor(format, { citations: 'quote_with_note', bibliography: true }, context)
    ).toMatchObject({ format, citations: 'quote_with_note', bibliography: true })
    expect(
      exportSettingsFor(format, { citations: 'footnote', bibliography: false }, context)
    ).toMatchObject({ format, citations: 'footnote', bibliography: false })
  })

  it('write a comment as a footnote where the format has no comment', () => {
    expect(
      exportSettingsFor('markdown', { citations: 'comment', bibliography: true }, context)
    ).toMatchObject({ citations: 'footnote' })
  })
})

describe('a download', () => {
  it('writes the file where the writer chose and says where', async () => {
    vi.mocked(save).mockResolvedValue('/out/Capítulo.md')
    vi.mocked(writeFile).mockResolvedValue()

    const outcome = await downloadExport(
      citing,
      'markdown',
      { citations: 'quote_with_note', bibliography: true },
      context
    )

    expect(outcome).toMatchObject({ kind: 'saved', path: '/out/Capítulo.md' })
    const bytes = vi.mocked(writeFile).mock.calls[0]![1] as Uint8Array
    expect(new TextDecoder().decode(bytes)).toContain('«el queso y los gusanos»')
  })

  it('names the file after the document', async () => {
    vi.mocked(save).mockResolvedValue(null)

    await downloadExport(citing, 'docx', { citations: 'footnote', bibliography: true }, context)

    expect(vi.mocked(save).mock.calls[0]![0]).toMatchObject({ defaultPath: 'Capítulo.docx' })
  })

  it('reports nothing when the writer closes the file dialog', async () => {
    vi.mocked(save).mockResolvedValue(null)

    const outcome = await downloadExport(
      citing,
      'html',
      { citations: 'inline', bibliography: true },
      context
    )

    expect(outcome).toEqual({ kind: 'cancelled' })
    expect(writeFile).not.toHaveBeenCalled()
  })

  it('says a comment went out as a footnote, rather than doing it silently', async () => {
    vi.mocked(save).mockResolvedValue('/out/Capítulo.md')
    vi.mocked(writeFile).mockResolvedValue()

    const outcome = await downloadExport(
      citing,
      'markdown',
      { citations: 'comment', bibliography: true },
      context
    )

    expect(outcome).toMatchObject({
      kind: 'saved',
      warnings: [{ element: 'comment', kind: 'citation', support: 'fallback', count: 1 }],
    })
  })

  it('reports a failure to write as a failure, with no file', async () => {
    vi.mocked(save).mockResolvedValue('/out/Capítulo.docx')
    vi.mocked(writeFile).mockRejectedValue(new Error('disk full'))

    const outcome = await downloadExport(
      citing,
      'docx',
      { citations: 'footnote', bibliography: true },
      context
    )

    expect(outcome).toEqual({ kind: 'failed', message: 'disk full' })
  })
})
