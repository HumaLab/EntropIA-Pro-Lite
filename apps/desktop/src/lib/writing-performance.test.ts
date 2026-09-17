import { invoke } from '@tauri-apps/api/core'
import { findMatches, parseCanonical, writingSchema } from '@entropia/ui'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Node } from './export-document'
import { fidelityWarnings } from './export-fidelity'
import { exportDocument, type ExportSettings } from './writing-export'

/**
 * Performance at the sizes §19 names (plan-editor.md §19).
 *
 * # Why these are budgets and not benchmarks
 *
 * §19's last rule is *"medir tiempos de apertura, guardado, búsqueda y
 * exportación"*, and a measurement nobody can fail is a number in a report. So
 * each one has a budget, deliberately loose — several times what the operation
 * actually takes on this machine — because the failure worth catching is a
 * change of *order*, not a slow afternoon on a busy CI runner.
 *
 * An accidental O(n²) in a document walk is invisible at the size of a test
 * fixture and ruinous at thirty thousand words. That is what these hold.
 *
 * # Why the sizes are the ones in the plan
 *
 * 10–20k words is an article, 30k a chapter, and the stress document is larger
 * still. Measuring at 200 words would prove that nothing is wrong with 200
 * words.
 */

const mockInvoke = vi.mocked(invoke)

const WORDS = [
  'huelga',
  'obreros',
  'gremio',
  'ciudad',
  'conflicto',
  'documento',
  'archivo',
  'paro',
  'represión',
  'memoria',
]

/** A manuscript of roughly `words` words, with headings, lists and citations. */
function manuscript(words: number, citations = 0): Node {
  const perParagraph = 60
  const paragraphs = Math.ceil(words / perParagraph)
  const content: Node[] = []

  for (let index = 0; index < paragraphs; index += 1) {
    if (index % 12 === 0) {
      content.push({
        type: 'heading',
        attrs: { level: (index % 24 === 0 ? 1 : 2) as number },
        content: [{ type: 'text', text: `Sección ${index}` }],
      })
    }
    const text = Array.from(
      { length: perParagraph },
      (_, at) => WORDS[(index + at) % WORDS.length]
    ).join(' ')
    content.push({ type: 'paragraph', content: [{ type: 'text', text }] })
  }

  for (let index = 0; index < citations; index += 1) {
    content.push({
      type: 'paragraph',
      content: [
        { type: 'text', text: 'Según ' },
        {
          type: 'zoteroCitation',
          attrs: {
            citationNodeId: `z${index}`,
            items: [
              { metadataSnapshot: JSON.stringify({ id: `obra${index}`, title: `Obra ${index}` }) },
            ],
          },
        },
        { type: 'text', text: ', el conflicto se extendió.' },
      ],
    })
  }

  return { type: 'doc', content }
}

function wordsIn(doc: Node): number {
  const walk = (node: Node): number =>
    typeof node.text === 'string'
      ? node.text.split(/\s+/).filter(Boolean).length
      : (node.content ?? []).reduce((total, child) => total + walk(child), 0)
  return walk(doc)
}

/** Milliseconds one call takes. */
async function took(work: () => unknown | Promise<unknown>): Promise<number> {
  const started = performance.now()
  await work()
  return performance.now() - started
}

const settings: ExportSettings = {
  format: 'markdown',
  citations: 'footnote',
  bibliography: true,
  style: { kind: 'bundled', name: 'apa' },
  title: 'Un capítulo',
  bibliographyHeading: 'Bibliografía',
}

beforeEach(() => {
  mockInvoke.mockReset()
  mockInvoke.mockImplementation(async (command: string, rawArgs?: unknown) => {
    const args = (rawArgs ?? {}) as Record<string, unknown>
    if (command === 'writing_csl_render_document') {
      const clusters = (args.clusters ?? []) as unknown[]
      return clusters.map(() => ({ text: '(Autor, 2015)', author_suppressed: false })) as never
    }
    if (command === 'writing_csl_bibliography') {
      const cited = (args.cited ?? []) as string[]
      return cited.map((id) => `Entrada de ${id}.`) as never
    }
    throw new Error(`unexpected command: ${command}`)
  })
})

describe('the fixtures are the sizes §19 names', () => {
  it('builds an article and a chapter at their stated sizes', () => {
    expect(wordsIn(manuscript(10_000))).toBeGreaterThanOrEqual(10_000)
    expect(wordsIn(manuscript(30_000))).toBeGreaterThanOrEqual(30_000)
  })
})

describe('opening a manuscript', () => {
  /**
   * Opening is validation: `parseCanonical` builds the ProseMirror document to
   * prove the schema accepts it. Spike S1 established why that has to happen
   * before the editor sees it, and this establishes that doing so at a
   * chapter's size stays linear.
   */
  it('validates a 30.000-word chapter well inside budget', async () => {
    const document = { schemaVersion: 1, doc: manuscript(30_000) }

    const elapsed = await took(() => {
      const parsed = parseCanonical(document)
      expect(parsed.ok).toBe(true)
    })

    expect(elapsed, `validating a chapter took ${Math.round(elapsed)}ms`).toBeLessThan(3000)
  })

  /**
   * The shape of the growth, not its speed: three times the words must not be
   * anything like nine times the work. An accidental quadratic passes every
   * fixed budget on a fast machine and fails on the writer's.
   */
  it('grows with the document and not with its square', async () => {
    const small = await took(() => parseCanonical({ schemaVersion: 1, doc: manuscript(10_000) }))
    const large = await took(() => parseCanonical({ schemaVersion: 1, doc: manuscript(30_000) }))

    // Three times the size, allowed four times the time. A quadratic would need
    // nine, so this catches it while leaving room for a noisy runner.
    expect(large, `10k took ${Math.round(small)}ms, 30k took ${Math.round(large)}ms`).toBeLessThan(
      Math.max(small * 4, 250)
    )
  })
})

describe('searching a manuscript', () => {
  /**
   * §19: *"cancelar búsquedas obsoletas"*. What is measured here is the cost of
   * one search at a chapter's size, because that is what bounds how often it
   * can run while someone types.
   */
  it('finds every occurrence in a chapter well inside budget', async () => {
    const parsed = parseCanonical({ schemaVersion: 1, doc: manuscript(30_000) })
    if (!parsed.ok) throw new Error('the fixture did not validate')
    const node = writingSchema().nodeFromJSON(parsed.document.doc)

    const elapsed = await took(() => {
      const matches = findMatches(node, 'represión')
      expect(matches.length).toBeGreaterThan(100)
    })

    expect(elapsed, `searching a chapter took ${Math.round(elapsed)}ms`).toBeLessThan(500)
  })
})

describe('exporting a manuscript', () => {
  it('writes a 30.000-word chapter to markdown inside budget', async () => {
    const doc = manuscript(30_000, 200)

    const elapsed = await took(async () => {
      const out = await exportDocument(doc, settings)
      expect('bytes' in out).toBe(true)
    })

    expect(elapsed, `exporting to markdown took ${Math.round(elapsed)}ms`).toBeLessThan(5000)
  })

  it('writes the same chapter to DOCX inside budget', async () => {
    const doc = manuscript(30_000, 200)

    const elapsed = await took(async () => {
      const out = await exportDocument(doc, { ...settings, format: 'docx' })
      expect('bytes' in out).toBe(true)
    })

    expect(elapsed, `exporting to DOCX took ${Math.round(elapsed)}ms`).toBeLessThan(15_000)
  })

  /**
   * §11.5: the citations are rendered for the whole document at once, because
   * disambiguation depends on all of them. So the engine is asked **once** per
   * export however many citations there are — asking per citation would be both
   * wrong and, at hundreds of them, ruinous.
   */
  it('asks the citation engine once for hundreds of citations', async () => {
    await exportDocument(manuscript(2_000, 300), settings)

    const rendered = mockInvoke.mock.calls.filter(
      ([command]) => command === 'writing_csl_render_document'
    )
    expect(rendered).toHaveLength(1)
    expect((rendered[0]![1] as { clusters: unknown[] }).clusters).toHaveLength(300)
  })

  /** And the bibliography is one more call, not one per work. */
  it('asks for the bibliography once for an extensive one', async () => {
    await exportDocument(manuscript(2_000, 300), settings)

    expect(
      mockInvoke.mock.calls.filter(([command]) => command === 'writing_csl_bibliography')
    ).toHaveLength(1)
  })
})

describe('the fidelity walk', () => {
  /**
   * The warnings are built by walking the whole document, and that walk runs
   * every time the writer changes the format in the dialog — so it has to be
   * cheap at a chapter's size, not merely correct.
   */
  it('surveys a chapter quickly enough to run on every choice', async () => {
    const doc = manuscript(30_000, 200)

    const elapsed = await took(() => fidelityWarnings(doc, 'docx'))

    expect(elapsed, `the fidelity walk took ${Math.round(elapsed)}ms`).toBeLessThan(500)
  })
})

describe('a stress document', () => {
  /**
   * §19 asks for one larger than a chapter, as a stress test. The point is not
   * the number: it is that nothing here falls over — no stack overflow from a
   * recursive walk, no quadratic that only shows up past some size.
   */
  it('validates and exports a 60.000-word document', async () => {
    const doc = manuscript(60_000, 400)

    const parsed = parseCanonical({ schemaVersion: 1, doc })
    expect(parsed.ok).toBe(true)

    const out = await exportDocument(doc, settings)
    expect('bytes' in out && out.bytes.length).toBeGreaterThan(100_000)
  }, 60_000)
})
