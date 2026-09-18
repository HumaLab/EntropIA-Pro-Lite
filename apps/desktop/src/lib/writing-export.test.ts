import { invoke } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Node } from './export-document'
import { exportDocument, isExportFailure, type ExportSettings } from './writing-export'

/**
 * The CSL half of an export, against the contract the Rust commands keep.
 *
 * The mock below is deliberately as strict as `csl::render`: every entry it is
 * handed is parsed as CSL-JSON, and one that is not answers with the same
 * `invalid_csl_json` error serde would give. A mock that accepted anything is
 * how the bibliography came to be asked for with bare ids — which serde reads as
 * "expected value at line 1 column 1" — while every test stayed green.
 */

const mockInvoke = vi.mocked(invoke)

const GINZBURG = JSON.stringify({ id: 'ginzburg1976', type: 'book', title: 'Il formaggio' })
const DARNTON = JSON.stringify({ id: 'darnton1984', type: 'book', title: 'The Great Cat Massacre' })

function parseLikeSerde(cslJson: string): { id?: unknown } {
  try {
    return JSON.parse(cslJson) as { id?: unknown }
  } catch {
    throw { code: 'invalid_csl_json', message: 'expected value at line 1 column 1' }
  }
}

const cite = (id: string, ...snapshots: string[]): Node => ({
  type: 'zoteroCitation',
  attrs: {
    citationNodeId: id,
    items: snapshots.map((metadataSnapshot) => ({ metadataSnapshot })),
  },
})
const p = (...content: Node[]): Node => ({ type: 'paragraph', content })
const doc = (...content: Node[]): Node => ({ type: 'doc', content })
const text = (value: string): Node => ({ type: 'text', text: value })

const settings: ExportSettings = {
  format: 'markdown',
  citations: 'inline',
  bibliography: true,
  style: { kind: 'bundled', name: 'apa' },
  title: '',
  bibliographyHeading: 'Bibliografía',
}

beforeEach(() => {
  mockInvoke.mockReset()
  mockInvoke.mockImplementation(async (command: string, rawArgs?: unknown) => {
    const args = (rawArgs ?? {}) as Record<string, unknown>
    if (command === 'writing_csl_render_document') {
      const clusters = args.clusters as { csl_json: string }[][]
      return clusters.map((cluster) => ({
        text: `(${cluster.map((item) => String(parseLikeSerde(item.csl_json).id)).join('; ')})`,
        author_suppressed: false,
      })) as never
    }
    if (command === 'writing_csl_bibliography') {
      const cited = args.cited as string[]
      return cited.map((entry) => `Entrada de ${String(parseLikeSerde(entry).id)}.`) as never
    }
    throw new Error(`unexpected command: ${command}`)
  })
})

async function exported(node: Node, extra: Partial<ExportSettings> = {}) {
  const out = await exportDocument(node, { ...settings, ...extra })
  if (isExportFailure(out)) throw new Error(`refused: ${out.elements.join(', ')}`)
  return { ...out, text: new TextDecoder().decode(out.bytes) }
}

describe('the bibliography of an export', () => {
  /**
   * The reported bug: Markdown with the bibliography on said the citations
   * could not be rendered — "expected value at line 1 column 1" — and the file
   * had no bibliography. `writing_csl_bibliography` parses each entry as a
   * work's CSL-JSON; it was being handed the works' ids.
   */
  it('hands the engine each cited work as CSL-JSON', async () => {
    const out = await exported(doc(p(text('Según '), cite('z1', GINZBURG))))

    expect(out.citationTrouble).toBeNull()
    expect(out.text).toContain('Entrada de ginzburg1976')
  })

  it('lists a work cited twice once, in the order first cited', async () => {
    await exported(doc(p(cite('z1', DARNTON), text(' y '), cite('z2', GINZBURG, DARNTON))))

    const [, args] = mockInvoke.mock.calls.find(
      ([command]) => command === 'writing_csl_bibliography'
    )!
    expect(args).toMatchObject({ cited: [DARNTON, GINZBURG] })
  })
})

describe('a manuscript with no citations', () => {
  /** Whatever the checkbox says, there is nothing to render and nothing to fail. */
  it.each([true, false])(
    'never reports citation trouble (bibliography %s)',
    async (bibliography) => {
      for (const format of ['markdown', 'html', 'docx'] as const) {
        const out = await exported(doc(p(text('Sin citas.'))), { bibliography, format })
        expect(out.citationTrouble).toBeNull()
      }
      expect(mockInvoke).not.toHaveBeenCalled()
    }
  )
})
