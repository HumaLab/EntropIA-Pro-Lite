import type { CitationRepresentation } from './export-fidelity'

/**
 * How a corpus citation reads in an export (plan-editor.md §17.2).
 *
 * # Why this is shared and pure
 *
 * §17.2 gives the writer four representations, and §17.2's last line is the
 * constraint that matters: *"La configuración de exportación no alterará el
 * documento canónico."* So the choice is applied here, over a copy of the
 * node's attributes, and never written back.
 *
 * All three exporters go through this, which is what keeps a footnote in DOCX
 * saying the same thing as a footnote in Markdown. Three implementations of
 * "what a citation says" would drift within a week.
 *
 * # What a citation has to say
 *
 * The snapshot on the node, and nothing fetched. §10.1 put the quoted text and
 * a minimal metadata snapshot on the node precisely so a citation survives its
 * source moving or being deleted — an exporter that went looking for the asset
 * would produce a different document depending on what happened to the corpus
 * since.
 */

/** A corpus citation as it stands in the manuscript. */
export interface CorpusCitationAttrs {
  quotedText?: unknown
  pageNumber?: unknown
  metadataSnapshot?: unknown
  assetId?: unknown
}

export interface RenderedCorpusCitation {
  /** What stands where the citation node was. May be empty. */
  inline: string
  /**
   * The note that accompanies it, if the representation has one. Where the note
   * goes — foot of the page, comment balloon, list at the end — is the format's
   * business; what it says is decided here.
   */
  note: string | null
}

function text(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value.trim() : null
}

function titleOf(snapshot: unknown): string | null {
  if (typeof snapshot === 'string') {
    try {
      return titleOf(JSON.parse(snapshot))
    } catch {
      // A snapshot that is not JSON is still a string someone can read.
      return text(snapshot)
    }
  }
  if (snapshot && typeof snapshot === 'object') {
    return text((snapshot as { title?: unknown }).title)
  }
  return null
}

function pageOf(attrs: CorpusCitationAttrs): number | null {
  if (typeof attrs.pageNumber === 'number') return attrs.pageNumber
  const snapshot = attrs.metadataSnapshot
  if (snapshot && typeof snapshot === 'object') {
    const page = (snapshot as { pageNumber?: unknown }).pageNumber
    if (typeof page === 'number') return page
  }
  return null
}

/**
 * The source, named as briefly as it can be: a title and a page.
 *
 * A citation with neither says so rather than rendering an empty pair of
 * parentheses. An export that shows `()` looks like a bug in the exporter; one
 * that shows "fuente sin identificar" tells the truth about the manuscript.
 */
export function sourceLabel(attrs: CorpusCitationAttrs): string {
  const title = titleOf(attrs.metadataSnapshot)
  const page = pageOf(attrs)
  const parts = [title, page === null ? null : `p. ${page}`].filter(Boolean)
  return parts.length > 0 ? parts.join(', ') : 'fuente sin identificar'
}

/** The quoted fragment in guillemets, or nothing when none was transcribed. */
function quoted(attrs: CorpusCitationAttrs): string | null {
  const value = text(attrs.quotedText)
  return value ? `«${value}»` : null
}

/**
 * Applies one of §17.2's representations to one citation.
 *
 * `comment` says the same thing a footnote does; the difference is only where
 * the format puts it, so the two share their text deliberately rather than
 * by coincidence.
 */
export function renderCorpusCitation(
  attrs: CorpusCitationAttrs,
  representation: CitationRepresentation
): RenderedCorpusCitation {
  const label = sourceLabel(attrs)
  const fragment = quoted(attrs)

  switch (representation) {
    case 'inline':
      // Brief, in the flow of the sentence. The quotation is not repeated: the
      // writer already wrote the sentence around it.
      return { inline: `(${label})`, note: null }

    case 'quote_with_note':
      // §17.2's fourth option verbatim: the quoted text, plus a note saying
      // where it came from.
      return { inline: fragment ?? `(${label})`, note: fragment ? label : null }

    case 'comment':
    case 'footnote':
    default:
      // Nothing in the flow — the format places its own marker — and everything
      // in the note, quotation included, because a footnote is where a reader
      // goes to find out what was actually said.
      return { inline: '', note: fragment ? `${label}. ${fragment}` : label }
  }
}

/**
 * What a note link becomes outside the application (§13).
 *
 * The matrix calls this a fallback in every format, and this is the fallback:
 * the snapshot the writer inserted, marked as having been a link to a note. A
 * reader of the export can tell that something was live; silently flattening it
 * to plain text would lose that, and lose it invisibly.
 */
export function renderNoteLink(attrs: { contentSnapshot?: unknown }): string {
  const body = text(attrs.contentSnapshot)
  // The same rule the node itself draws by: the snapshot when there is one, the
  // marker only when there is nothing else to show. An export that added the
  // marker on top would say something the page does not, and in a finished
  // article "[nota]" is noise — the snapshot is the writer's own words.
  //
  // That the link was live is not lost by dropping it: HTML keeps the
  // `note-link` class, DOCX keeps the italics, and all three formats report the
  // substitution in the export's warnings, which is where §17.4 asks for it.
  return body ? `«${body}»` : '[nota]'
}
