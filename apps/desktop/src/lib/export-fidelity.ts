/**
 * What each export format can actually keep (plan-editor.md §17.1, §17.4).
 *
 * # Why the matrix is code and not a document
 *
 * §17.4 asks the fidelity matrix to distinguish native support, a documented
 * alternative representation, and elements a format does not admit — and then
 * adds the sentence that gives the matrix teeth: *"Una advertencia no permite
 * declarar cumplido un elemento obligatorio que DOCX deba conservar."*
 *
 * A matrix that lives only in a spike report cannot enforce that. This one is
 * read by the exporters to build their warnings, and by its tests to refuse a
 * DOCX exporter that quietly downgrades a footnote to italic text.
 *
 * # Why it is keyed by schema node
 *
 * The schema in `document-contract.ts` is the single definition of what a
 * manuscript may contain. Anything added there and forgotten here would export
 * silently as nothing, so the matrix is checked against the schema itself
 * rather than against a list somebody remembered to update.
 */

export type ExportFormat = 'markdown' | 'html' | 'docx'

/**
 * Native: the format has the thing. Fallback: something else stands in for it,
 * and the export says so. Unsupported: it cannot be carried, and the export
 * says that instead of pretending.
 */
export type Support = 'native' | 'fallback' | 'unsupported'

/** How a corpus citation is written out (§17.2). The writer chooses. */
export type CitationRepresentation = 'footnote' | 'inline' | 'comment' | 'quote_with_note'

/**
 * Every node of the writing schema against every format.
 *
 * `fallback` entries name what stands in, in the comment beside them, because
 * §17.4 asks for the alternative representation to be *documented* — an
 * undocumented fallback is indistinguishable from a bug.
 */
export const NODE_FIDELITY: Record<string, Record<ExportFormat, Support>> = {
  doc: { markdown: 'native', html: 'native', docx: 'native' },
  paragraph: { markdown: 'native', html: 'native', docx: 'native' },
  text: { markdown: 'native', html: 'native', docx: 'native' },
  heading: { markdown: 'native', html: 'native', docx: 'native' },
  bulletList: { markdown: 'native', html: 'native', docx: 'native' },
  orderedList: { markdown: 'native', html: 'native', docx: 'native' },
  listItem: { markdown: 'native', html: 'native', docx: 'native' },
  blockquote: { markdown: 'native', html: 'native', docx: 'native' },
  codeBlock: { markdown: 'native', html: 'native', docx: 'native' },
  horizontalRule: { markdown: 'native', html: 'native', docx: 'native' },
  // Markdown's line break is two trailing spaces — real, but invisible in the
  // source, which is why it is native rather than a fallback.
  hardBreak: { markdown: 'native', html: 'native', docx: 'native' },
  // GFM tables. No cell spans and no block content inside a cell, but the
  // schema does not offer those either.
  table: { markdown: 'native', html: 'native', docx: 'native' },
  tableRow: { markdown: 'native', html: 'native', docx: 'native' },
  tableHeader: { markdown: 'native', html: 'native', docx: 'native' },
  tableCell: { markdown: 'native', html: 'native', docx: 'native' },
  // GFM footnotes in Markdown; real `footnotes.xml` in DOCX, verified in S4 to
  // renumber. In HTML, a list at the end with links both ways.
  footnotes: { markdown: 'native', html: 'native', docx: 'native' },
  footnote: { markdown: 'native', html: 'native', docx: 'native' },
  footnoteReference: { markdown: 'native', html: 'native', docx: 'native' },
  // Written out in whichever representation §17.2 the writer chose; what each
  // format can do with each choice is `CITATION_FIDELITY` below.
  documentCitation: { markdown: 'native', html: 'native', docx: 'native' },
  // Styled text carrying the CSL rendering, which is exactly what §17.3 asks
  // of the MVP: live Zotero fields are explicitly out of scope.
  zoteroCitation: { markdown: 'native', html: 'native', docx: 'native' },
  /**
   * A link to a note has no meaning outside the application, so every format
   * gets the snapshot the writer inserted plus a note of where it came from.
   * Documented here rather than silently flattened: a reader of the export
   * should be able to tell that something was a live link.
   */
  noteLink: { markdown: 'fallback', html: 'fallback', docx: 'fallback' },
}

/** Every mark, on the same terms. */
export const MARK_FIDELITY: Record<string, Record<ExportFormat, Support>> = {
  bold: { markdown: 'native', html: 'native', docx: 'native' },
  italic: { markdown: 'native', html: 'native', docx: 'native' },
  strike: { markdown: 'native', html: 'native', docx: 'native' },
  code: { markdown: 'native', html: 'native', docx: 'native' },
  link: { markdown: 'native', html: 'native', docx: 'native' },
  // Markdown has no underline of its own; an inline `<u>` stands in, which
  // every Markdown reader that allows HTML will render and the rest will show
  // as tags. Said out loud rather than dropped.
  underline: { markdown: 'fallback', html: 'native', docx: 'native' },
  // The same stand-in as underline: `<sub>` and `<sup>` inline, since GFM has
  // neither.
  subscript: { markdown: 'fallback', html: 'native', docx: 'native' },
  superscript: { markdown: 'fallback', html: 'native', docx: 'native' },
  // A relative font size (the only style this mark carries today). Markdown
  // has no sizes, so a `<span style="font-size: …em">` stands in; HTML keeps
  // the em, and DOCX turns it into points against the paragraph's own size.
  textStyle: { markdown: 'fallback', html: 'native', docx: 'native' },
}

/**
 * The four representations of §17.2, against what each format can do.
 *
 * A comment is a real Office Open XML comment — S4 confirmed `docx` emits
 * `comments.xml` — and an HTML `<aside>` marked as one. Markdown has no
 * comment that survives rendering, so the option is simply not admitted there
 * rather than quietly becoming a footnote.
 */
export const CITATION_FIDELITY: Record<CitationRepresentation, Record<ExportFormat, Support>> = {
  footnote: { markdown: 'native', html: 'native', docx: 'native' },
  inline: { markdown: 'native', html: 'native', docx: 'native' },
  comment: { markdown: 'unsupported', html: 'fallback', docx: 'native' },
  quote_with_note: { markdown: 'native', html: 'native', docx: 'native' },
}

/**
 * The elements §17.1 lists as obligatory, named by what carries them.
 *
 * This is the list the DOCX exporter is held to: §17.4 forbids a warning from
 * standing in for one of these, so a `fallback` here is a failing export, not
 * a partial one.
 */
export const REQUIRED_BY_SPEC = [
  'heading',
  'bulletList',
  'orderedList',
  'blockquote',
  'table',
  'footnote',
  'documentCitation',
  'zoteroCitation',
] as const

export function supportOfNode(node: string, format: ExportFormat): Support {
  // An unknown node is unsupported rather than assumed fine. A schema that grew
  // a node nobody taught the exporters about must not export as silence.
  return NODE_FIDELITY[node]?.[format] ?? 'unsupported'
}

export function supportOfMark(mark: string, format: ExportFormat): Support {
  return MARK_FIDELITY[mark]?.[format] ?? 'unsupported'
}

/** One thing the export could not carry as itself. */
export interface FidelityWarning {
  /** The schema name, or the citation representation that was chosen. */
  element: string
  kind: 'node' | 'mark' | 'citation'
  support: Exclude<Support, 'native'>
  /** How many times it occurs, so a warning can say "12 of these". */
  count: number
}

type Counted = Map<string, { kind: FidelityWarning['kind']; count: number }>

function bump(into: Counted, key: string, kind: FidelityWarning['kind']) {
  const seen = into.get(key)
  if (seen) seen.count += 1
  else into.set(key, { kind, count: 1 })
}

/**
 * Walks a document and reports everything a format cannot carry as itself.
 *
 * Counts rather than a set: "una nota al pie no se pudo representar" and
 * "cuarenta notas al pie no se pudieron representar" are different facts, and
 * the second one usually changes the writer's mind about the format.
 */
export function fidelityWarnings(
  doc: unknown,
  format: ExportFormat,
  citations: CitationRepresentation = 'footnote'
): FidelityWarning[] {
  const counted: Counted = new Map()
  let usesCorpusCitation = false

  const walk = (node: unknown) => {
    if (!node || typeof node !== 'object') return
    const current = node as {
      type?: string
      content?: unknown[]
      marks?: { type?: string }[]
    }

    if (typeof current.type === 'string') {
      if (current.type === 'documentCitation') usesCorpusCitation = true
      const support = supportOfNode(current.type, format)
      if (support !== 'native') bump(counted, current.type, 'node')
    }

    for (const mark of current.marks ?? []) {
      if (typeof mark.type !== 'string') continue
      const support = supportOfMark(mark.type, format)
      if (support !== 'native') bump(counted, mark.type, 'mark')
    }

    for (const child of current.content ?? []) walk(child)
  }

  walk(doc)

  const warnings: FidelityWarning[] = [...counted].map(([element, seen]) => ({
    element,
    kind: seen.kind,
    support: (seen.kind === 'mark'
      ? supportOfMark(element, format)
      : supportOfNode(element, format)) as Exclude<Support, 'native'>,
    count: seen.count,
  }))

  // Only when the document actually cites the corpus: warning about a
  // representation nothing uses trains people to ignore warnings.
  if (usesCorpusCitation) {
    const support = CITATION_FIDELITY[citations][format]
    if (support !== 'native') {
      warnings.push({ element: citations, kind: 'citation', support, count: 1 })
    }
  }

  return warnings
}

/**
 * Whether an export may go ahead as a faithful one (§17.4).
 *
 * A DOCX that loses an obligatory element is not a partial export to be warned
 * about — it is a failed one. Markdown and HTML are held to what the matrix
 * says they can do, because §17.1 qualifies its list with *"en la medida
 * admitida por cada formato"*; DOCX has no such excuse, since S4 verified the
 * whole list natively.
 */
export function losesRequiredElement(warnings: FidelityWarning[], format: ExportFormat): string[] {
  if (format !== 'docx') return []
  const required = new Set<string>(REQUIRED_BY_SPEC)
  return warnings.filter((warning) => required.has(warning.element)).map((w) => w.element)
}
