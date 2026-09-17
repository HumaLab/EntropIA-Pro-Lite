import { describe, expect, it } from 'vitest'
import { renderCorpusCitation, renderNoteLink, sourceLabel } from './export-citations'

/**
 * What a corpus citation says in an export (plan-editor.md §17.2).
 *
 * Shared by all three exporters on purpose: three separate ideas of what a
 * citation says would drift within a week, and the writer would find that a
 * footnote in DOCX and a footnote in Markdown cite the same fragment
 * differently.
 */

const cited = {
  quotedText: 'los obreros declararon la huelga',
  pageNumber: 112,
  metadataSnapshot: { title: 'Acta del gremio', pageNumber: 112 },
}

describe('naming the source', () => {
  it('gives the title and the page', () => {
    expect(sourceLabel(cited)).toBe('Acta del gremio, p. 112')
  })

  it('reads a snapshot that was stored as JSON text', () => {
    expect(
      sourceLabel({ ...cited, metadataSnapshot: JSON.stringify(cited.metadataSnapshot) })
    ).toBe('Acta del gremio, p. 112')
  })

  it('gives the title alone when there is no page', () => {
    expect(sourceLabel({ metadataSnapshot: { title: 'Acta del gremio' } })).toBe('Acta del gremio')
  })

  /**
   * An export that shows `()` looks like a bug in the exporter. One that says
   * the source is unidentified tells the truth about the manuscript.
   */
  it('says the source is unidentified rather than rendering empty parentheses', () => {
    expect(sourceLabel({})).toBe('fuente sin identificar')
  })
})

describe('the four representations of §17.2', () => {
  /** A footnote is where a reader goes to find out what was actually said. */
  it('puts everything in the note and nothing in the flow, as a footnote', () => {
    const out = renderCorpusCitation(cited, 'footnote')

    expect(out.inline).toBe('')
    expect(out.note).toBe('Acta del gremio, p. 112. «los obreros declararon la huelga»')
  })

  it('gives a brief reference inside the text', () => {
    const out = renderCorpusCitation(cited, 'inline')

    expect(out).toEqual({ inline: '(Acta del gremio, p. 112)', note: null })
  })

  /** §17.2's fourth option verbatim: the quoted text plus a note of provenance. */
  it('quotes the text and notes where it came from', () => {
    const out = renderCorpusCitation(cited, 'quote_with_note')

    expect(out.inline).toBe('«los obreros declararon la huelga»')
    expect(out.note).toBe('Acta del gremio, p. 112')
  })

  /**
   * A comment says what a footnote says; only where the format puts it differs.
   * Sharing the text is deliberate, not a coincidence to be refactored away.
   */
  it('says in a comment exactly what it would say in a footnote', () => {
    expect(renderCorpusCitation(cited, 'comment')).toEqual(renderCorpusCitation(cited, 'footnote'))
  })

  /**
   * §10.1 allows a citation with no transcription. Quoting nothing would print
   * an empty pair of guillemets.
   */
  it('falls back to the reference when nothing was transcribed', () => {
    const reference = { metadataSnapshot: { title: 'Acta del gremio' }, pageNumber: 4 }

    expect(renderCorpusCitation(reference, 'quote_with_note').inline).toBe(
      '(Acta del gremio, p. 4)'
    )
    expect(renderCorpusCitation(reference, 'footnote').note).toBe('Acta del gremio, p. 4')
  })
})

describe('a note link outside the application', () => {
  /**
   * The same rule the node draws by on the page. An export that added a marker
   * on top would say something the editor does not, and in a finished article
   * `[nota]` is noise: the snapshot is the writer's own words.
   *
   * That the link was live is reported in the export's warnings, which is where
   * §17.4 asks for a substitution to be declared — not in the prose.
   */
  it('reads as the snapshot, exactly as it does on the page', () => {
    expect(renderNoteLink({ contentSnapshot: 'lo que decia la nota' })).toBe(
      '«lo que decia la nota»'
    )
  })

  /** The marker survives for the one case where there is nothing else to show. */
  it('leaves a marker only when there was no snapshot to keep', () => {
    expect(renderNoteLink({})).toBe('[nota]')
  })
})
