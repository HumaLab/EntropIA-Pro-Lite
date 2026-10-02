import { invoke } from '@tauri-apps/api/core'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Node } from './export-document'
import { clusterOf } from './citation-clusters'
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

const cite = (id: string, ...items: (string | Record<string, unknown>)[]): Node => ({
  type: 'zoteroCitation',
  attrs: {
    citationNodeId: id,
    items: items.map((item) => (typeof item === 'string' ? { metadataSnapshot: item } : item)),
  },
})
const qualified = (
  sourceOrigin: string,
  sourceInstanceId: string | null,
  libraryType: string,
  libraryId: string,
  itemKey = 'SAME-KEY'
): Record<string, unknown> => ({
  sourceOrigin,
  sourceInstanceId,
  libraryType,
  libraryId,
  itemKey,
  itemVersion: 9,
  metadataSnapshot: JSON.stringify({ id: 'same-csl-id', type: 'book', title: 'Shared work' }),
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

describe('qualified citation identity at the CSL/export seam', () => {
  it('keeps qualified CSL collisions distinct in rendering and bibliography', async () => {
    const works = [
      qualified('local', 'instance-a', 'user', '0'),
      qualified('web', 'instance-a', 'user', '0'),
      qualified('local', 'instance-a', 'group', '0'),
      qualified('local', 'instance-a', 'user', '1'),
      qualified('local', 'instance-b', 'user', '0'),
    ]
    const before = works.map((work) => ({ ...work }))

    const out = await exported(
      doc(...works.map((work, index) => p(cite(`qualified-${index}`, work))))
    )

    expect(out.citationTrouble).toBeNull()
    expect(works).toEqual(before)

    const [, renderArgs] = mockInvoke.mock.calls.find(
      ([command]) => command === 'writing_csl_render_document'
    )!
    const renderedIds = (renderArgs as { clusters: { csl_json: string }[][] }).clusters.map(
      (cluster) => JSON.parse(cluster[0]!.csl_json).id
    )
    expect(new Set(renderedIds).size).toBe(works.length)

    const [, bibliographyArgs] = mockInvoke.mock.calls.find(
      ([command]) => command === 'writing_csl_bibliography'
    )!
    const cited = (bibliographyArgs as { cited: string[] }).cited
    expect(cited).toHaveLength(works.length)
    expect(new Set(cited.map((entry) => JSON.parse(entry).id)).size).toBe(works.length)
  })

  it('deduplicates a repeated fully qualified work', async () => {
    const work = qualified('local', 'instance-a', 'user', '0')

    await exported(doc(p(cite('qualified-1', work)), p(cite('qualified-2', { ...work }))))

    const [, renderArgs] = mockInvoke.mock.calls.find(
      ([command]) => command === 'writing_csl_render_document'
    )!
    const renderedIds = (renderArgs as { clusters: { csl_json: string }[][] }).clusters.map(
      (cluster) => JSON.parse(cluster[0]!.csl_json).id
    )
    expect(renderedIds[0]).toBe(renderedIds[1])

    const [, bibliographyArgs] = mockInvoke.mock.calls.find(
      ([command]) => command === 'writing_csl_bibliography'
    )!
    expect((bibliographyArgs as { cited: string[] }).cited).toHaveLength(1)
  })

  it('does not merge qualified occurrences whose source instance is unknown', () => {
    const first = qualified('local', null, 'user', '0')
    const second = qualified('local', null, 'user', '0')
    const cluster = clusterOf(cite('unknown-node', first, second).attrs!)
    const ids = cluster.map((item) => JSON.parse(item.csl_json).id as string)

    expect(ids[0]).not.toBe(ids[1])
    expect(ids[0]).toContain('unknown-node')
    expect(ids[0]).toContain(':0')
    expect(ids[1]).toContain(':1')
  })

  it('keeps an unqualified legacy citation CSL id through cluster and export', async () => {
    const legacySnapshot = JSON.stringify({
      id: 'legacy-csl-id',
      type: 'book',
      title: 'Legacy work',
    })
    const legacy: Node = {
      type: 'zoteroCitation',
      attrs: {
        citationNodeId: 'legacy-node',
        itemKey: 'LEGACY-KEY',
        metadataSnapshot: legacySnapshot,
      },
    }

    expect(clusterOf(legacy.attrs!)[0]!.csl_json).toBe(legacySnapshot)
    const out = await exported(doc(p(legacy)))

    expect(out.citationTrouble).toBeNull()
    const [, renderArgs] = mockInvoke.mock.calls.find(
      ([command]) => command === 'writing_csl_render_document'
    )!
    expect(
      JSON.parse((renderArgs as { clusters: { csl_json: string }[][] }).clusters[0]![0]!.csl_json)
        .id
    ).toBe('legacy-csl-id')
    const [, bibliographyArgs] = mockInvoke.mock.calls.find(
      ([command]) => command === 'writing_csl_bibliography'
    )!
    expect((bibliographyArgs as { cited: string[] }).cited).toEqual([legacySnapshot])
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
