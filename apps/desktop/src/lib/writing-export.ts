import { save } from '@tauri-apps/plugin-dialog'
import { writeFile } from '@tauri-apps/plugin-fs'
import { citedWorks, clusterOf } from './citation-clusters'
import type { ExportContext, Node } from './export-document'
import { toDocx } from './export-docx'
import {
  fidelityWarnings,
  losesRequiredElement,
  type CitationRepresentation,
  type ExportFormat,
  type FidelityWarning,
} from './export-fidelity'
import { toHtml } from './export-html'
import { toMarkdown } from './export-markdown'
import { citationsForFormat, type ExportPreferences } from './export-preferences'
import {
  isCslError,
  renderBibliography,
  renderDocument,
  type ClusterItem,
  type StyleSource,
} from './writing-csl'

/**
 * Exporting a manuscript (plan-editor.md §17).
 *
 * # What this module decides and what it does not
 *
 * The three renderers know their formats. This knows the two things that are
 * not about any one format: that the citations must be rendered by the CSL
 * engine before anything is written, and that §17.4 draws a line a DOCX export
 * may not cross.
 *
 * # Why the CSL rendering happens here and once
 *
 * §11.5: disambiguation is a property of the document. Which of two works by
 * one author in one year reads `2015a` depends on all the others, so the
 * citations are rendered together, for the whole manuscript, before any format
 * sees them. Asking per citation could only ever produce `2015` twice.
 *
 * And it happens *before* the export, not from the cache on each node: §11.5 is
 * explicit that `renderedText` is never the source of truth. Exporting from it
 * would put yesterday's citation style into today's document — with no error,
 * which is the worst way for it to be wrong.
 *
 * # Why a lossy DOCX is a failure and not a warning
 *
 * §17.4, in the same breath as asking for warnings: *"Una advertencia no
 * permite declarar cumplido un elemento obligatorio que DOCX deba conservar."*
 * Markdown may substitute and say so — §17.1's own "en la medida admitida por
 * cada formato" grants it that. DOCX has no such excuse, because S4 verified
 * the format admits every obligatory element natively. So a DOCX that would
 * lose one does not get written.
 */

export type { ExportFormat, CitationRepresentation, FidelityWarning }

export interface ExportSettings {
  format: ExportFormat
  citations: CitationRepresentation
  /** Whether a bibliography is appended (§11.6). Derived, never stored. */
  bibliography: boolean
  style: StyleSource
  title: string
  bibliographyHeading: string
}

export interface ExportResult {
  bytes: Uint8Array
  /** Everything the format could not carry as itself. Possibly empty. */
  warnings: FidelityWarning[]
  /**
   * Citations the CSL engine could not render, reported rather than silently
   * left as markers: a bibliography missing an entry looks like the writer
   * forgot to cite something.
   */
  citationTrouble: string | null
}

export type ExportFailure = {
  ok: false
  code: 'loses_required_element'
  /** Which obligatory elements §17.4 will not let a DOCX drop. */
  elements: string[]
}

const EXTENSION: Record<ExportFormat, { extension: string; name: string }> = {
  markdown: { extension: 'md', name: 'Markdown' },
  html: { extension: 'html', name: 'HTML' },
  docx: { extension: 'docx', name: 'Word' },
}

/**
 * Renders every citation and the bibliography, together, once.
 *
 * A failure is reported, never thrown: §11.3's rule that a citation which
 * cannot be rendered must not stop the manuscript being edited applies at least
 * as strongly to exporting it. The export proceeds with whatever each node
 * cached, and says what happened.
 */
async function renderCitations(
  doc: Node,
  settings: ExportSettings
): Promise<{ zotero: Record<string, string>; bibliography: string[]; trouble: string | null }> {
  // Through the same mapping the editor renders with, so what is on screen and
  // what lands in the file cite the same works the same way.
  const clusters: { id: string; items: ClusterItem[] }[] = []

  const walk = (node: Node) => {
    if (node.type === 'zoteroCitation') {
      const id = typeof node.attrs?.citationNodeId === 'string' ? node.attrs.citationNodeId : ''
      const items = clusterOf(node.attrs ?? {})
      if (id && items.length > 0) clusters.push({ id, items })
    }
    for (const child of node.content ?? []) walk(child)
  }
  walk(doc)

  if (clusters.length === 0) return { zotero: {}, bibliography: [], trouble: null }

  const rendered = await renderDocument(
    clusters.map((cluster) => cluster.items),
    settings.style
  )
  if (isCslError(rendered)) {
    return { zotero: {}, bibliography: [], trouble: rendered.message }
  }

  const zotero: Record<string, string> = {}
  clusters.forEach((cluster, index) => {
    const text = rendered[index]?.text
    if (typeof text === 'string') zotero[cluster.id] = text
  })

  if (!settings.bibliography) return { zotero, bibliography: [], trouble: null }

  const entries = await renderBibliography(
    citedWorks(clusters.map((cluster) => cluster.items)),
    settings.style
  )
  return isCslError(entries)
    ? { zotero, bibliography: [], trouble: entries.message }
    : { zotero, bibliography: entries, trouble: null }
}

/**
 * Produces the file, or refuses.
 *
 * Nothing here touches the manuscript. §17.2's closing line — *"La
 * configuración de exportación no alterará el documento canónico"* — is kept by
 * construction: the settings are read, the document is not written to, and the
 * rendered citations live in a context object that is discarded afterwards.
 */
export async function exportDocument(
  doc: Node,
  settings: ExportSettings
): Promise<ExportResult | ExportFailure> {
  const warnings = fidelityWarnings(doc, settings.format, settings.citations)
  const lost = losesRequiredElement(warnings, settings.format)
  if (lost.length > 0) {
    return { ok: false, code: 'loses_required_element', elements: lost }
  }

  const { zotero, bibliography, trouble } = await renderCitations(doc, settings)

  const context: ExportContext = {
    title: settings.title,
    citations: settings.citations,
    zotero,
    bibliography,
    bibliographyHeading: settings.bibliographyHeading,
  }

  const bytes =
    settings.format === 'docx'
      ? await toDocx(doc, context)
      : new TextEncoder().encode(
          settings.format === 'html' ? toHtml(doc, context) : toMarkdown(doc, context)
        )

  return { bytes, warnings, citationTrouble: trouble }
}

export function isExportFailure(value: unknown): value is ExportFailure {
  return (
    typeof value === 'object' && value !== null && 'ok' in value && !(value as ExportFailure).ok
  )
}

/** What an export needs to know that is neither the format nor a preference. */
export interface DownloadContext {
  title: string
  /** What the file dialog proposes, before the extension. */
  fileName: string
  style: StyleSource
  bibliographyHeading: string
}

/**
 * One configuration for all three formats (§17.2): the format is chosen at the
 * download, the rest was chosen once, in the Export tab. The only thing that
 * varies by format is what the format cannot do — a comment in Markdown, which
 * goes out as a footnote.
 */
export function exportSettingsFor(
  format: ExportFormat,
  preferences: ExportPreferences,
  context: DownloadContext
): ExportSettings {
  return {
    format,
    citations: citationsForFormat(preferences.citations, format),
    bibliography: preferences.bibliography,
    style: context.style,
    title: context.title,
    bibliographyHeading: context.bibliographyHeading,
  }
}

/**
 * How a download ended. `cancelled` is its own case because it is not a
 * failure: the writer closed the file dialog, and there is nothing to report.
 */
export type DownloadOutcome =
  | { kind: 'saved'; path: string; warnings: FidelityWarning[]; trouble: string | null }
  | { kind: 'cancelled' }
  | { kind: 'refused'; elements: string[] }
  | { kind: 'failed'; message: string }

/**
 * Exports in one format with the saved preferences, asks where, and writes.
 *
 * A representation the format had to stand in for is reported as a fallback of
 * the one that was chosen, and only when the manuscript cites the corpus at
 * all — the same rule `fidelityWarnings` keeps for its own citation warning.
 */
export async function downloadExport(
  doc: Node,
  format: ExportFormat,
  preferences: ExportPreferences,
  context: DownloadContext
): Promise<DownloadOutcome> {
  const settings = exportSettingsFor(format, preferences, context)
  const substituted =
    settings.citations !== preferences.citations &&
    fidelityWarnings(doc, format, preferences.citations).some(
      (warning) => warning.kind === 'citation'
    )

  try {
    const out = await saveExport(doc, settings, context.fileName)
    if (isExportFailure(out)) return { kind: 'refused', elements: out.elements }
    if (!out.path) return { kind: 'cancelled' }

    const warnings: FidelityWarning[] = substituted
      ? [
          { element: preferences.citations, kind: 'citation', support: 'fallback', count: 1 },
          ...out.result.warnings,
        ]
      : out.result.warnings
    return { kind: 'saved', path: out.path, warnings, trouble: out.result.citationTrouble }
  } catch (error) {
    // Not citation trouble: that is reported on a file that was written. A
    // throw here means no file exists, and saying otherwise would send the
    // writer looking for it.
    return { kind: 'failed', message: error instanceof Error ? error.message : String(error) }
  }
}

/**
 * Asks where to put it and writes it. Returns the path, or null if nobody chose.
 *
 * The document is produced before the filename is asked for, so that a refusal
 * under §17.4 arrives before the writer has picked a name and a folder. After
 * that point the same refusal reads as a failure to save — as though something
 * went wrong with the disk — rather than as what it is: a fact about the format
 * and this manuscript.
 */
export async function saveExport(
  doc: Node,
  settings: ExportSettings,
  defaultName: string
): Promise<{ path: string | null; result: ExportResult } | ExportFailure> {
  const result = await exportDocument(doc, settings)
  if (isExportFailure(result)) return result

  const option = EXTENSION[settings.format]
  const path = await save({
    defaultPath: `${defaultName}.${option.extension}`,
    filters: [{ name: option.name, extensions: [option.extension] }],
  })
  if (!path) return { path: null, result }

  await writeFile(path, result.bytes)
  return { path, result }
}
