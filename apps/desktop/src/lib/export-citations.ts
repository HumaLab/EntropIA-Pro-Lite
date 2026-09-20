import type { CitationRepresentation } from './export-fidelity'
import type { QuotePart } from './rendered-selection'

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
  quotedParts?: unknown
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
  /**
   * The quotation itself, as it appears inside `inline` or `note`, or null when
   * this representation does not carry one. A format that has to draw the
   * quotation differently — because it took in an image — finds it with
   * {@link aroundFragment} instead of guessing where it sits.
   */
  fragment: string | null
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

/**
 * The quoted fragment in guillemets, or nothing when none was transcribed.
 *
 * Line breaks are kept: this says what a citation says, and how a break is
 * written down belongs to the format — a `<br />` in HTML, a break run in
 * DOCX, a hard break in Markdown.
 */
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
      return { inline: `(${label})`, note: null, fragment: null }

    case 'quote_with_note':
      // §17.2's fourth option verbatim: the quoted text, plus a note saying
      // where it came from.
      return {
        inline: fragment ?? `(${label})`,
        note: fragment ? label : null,
        fragment: fragment ?? null,
      }

    case 'comment':
    case 'footnote':
    default:
      // Nothing in the flow — the format places its own marker — and everything
      // in the note, quotation included, because a footnote is where a reader
      // goes to find out what was actually said.
      return {
        inline: '',
        note: fragment ? `${label}. ${fragment}` : label,
        fragment: fragment ?? null,
      }
  }
}

/**
 * The parts of a quotation that took in an image, or null when it is only
 * words. What the citation node draws on the page (`quotedParts`), validated:
 * the manuscript is a file on disk and nothing guarantees its shape.
 *
 * Null rather than an array of text parts when there is no image, so a caller
 * can ask one question — "is there anything here the plain text cannot say?" —
 * and otherwise keep the simple path it already had.
 */
export function quotedPartsOf(attrs: CorpusCitationAttrs): QuotePart[] | null {
  if (!Array.isArray(attrs.quotedParts)) return null

  const parts: QuotePart[] = []
  for (const part of attrs.quotedParts) {
    if (!part || typeof part !== 'object') continue
    const { kind, text: value, source } = part as Record<string, unknown>
    if (kind === 'text' && typeof value === 'string') parts.push({ kind: 'text', text: value })
    else if (kind === 'image' && typeof source === 'string') parts.push({ kind: 'image', source })
  }

  return parts.some((part) => part.kind === 'image') ? parts : null
}

/**
 * What comes before and after the quotation inside a rendered citation, or null
 * when the quotation is not there to be found. Lets a format replace the
 * quotation — and only the quotation — with its own drawing of the parts.
 */
export function aroundFragment(value: string, fragment: string | null): [string, string] | null {
  if (!fragment) return null
  const at = value.indexOf(fragment)
  return at === -1 ? null : [value.slice(0, at), value.slice(at + fragment.length)]
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
