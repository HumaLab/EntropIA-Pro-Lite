import { save } from '@tauri-apps/plugin-dialog'
import { writeFile } from '@tauri-apps/plugin-fs'

import type { ExportFormat } from './export-fidelity'
import { renderMarkdown } from './markdown'
import { buildPrintableHtml, generateDocxBytes } from './ocr-export'

/**
 * Downloading a research report in the three formats Escritura offers.
 *
 * # Why this exports the markdown and not the rendered view
 *
 * The engine writes the whole report — coverage, quoted passages with their
 * `[n]`, "Fuentes citadas", the profile and the agreed framing — into the
 * artifact's `markdown`, and InvestigationView says so where it reads it: the
 * front end decides how the report LOOKS, never what it says. So the export
 * starts from that canonical text. Rebuilding it from `sections[].text` would
 * quietly drop everything the engine put around the sections.
 *
 * # Why the query is added here rather than asked of the engine
 *
 * The report opens on screen with the question it answers. A downloaded copy
 * that lost it would be a document nobody can place a year from now. Older
 * artifacts were written before the report carried the question at all, so it
 * is prepended — and only when the text does not already say it, because a
 * newer engine that writes it itself must not produce it twice.
 */

const EXTENSION: Record<ExportFormat, { name: string; extension: string }> = {
  markdown: { name: 'Markdown', extension: 'md' },
  html: { name: 'HTML', extension: 'html' },
  docx: { name: 'Microsoft Word', extension: 'docx' },
}

const MAX_FILENAME_STEM_LENGTH = 80

/** Generators are injectable so a test never loads the html-docx browser bundle. */
export interface InvestigationExportGenerators {
  docx: (html: string) => Promise<Uint8Array>
}

export interface InvestigationReport {
  /** The artifact's canonical markdown. */
  markdown: string
  /** The question the investigation was given. */
  question: string
  /** What the query section is called, in the interface's language. */
  queryHeading: string
}

/**
 * The report's markdown with the query as its first section.
 *
 * It goes after the title rather than above it: a document whose first line is
 * not its name reads as a fragment. When the report has no `# ` title at all —
 * an artifact built by the fallback path, which emits nothing when there is
 * neither title nor sections — the query heads the file instead.
 */
export function reportMarkdownWithQuery({
  markdown,
  question,
  queryHeading,
}: InvestigationReport): string {
  const text = markdown.replace(/\r\n?/g, '\n').trim()
  const query = question.trim()
  if (!query) return text

  // Already there: a newer engine writes the question itself, and a second copy
  // two lines below the first is worse than none.
  if (text.includes(query)) return text

  const block = `## ${queryHeading}\n\n${query}`
  const lines = text.split('\n')
  const titleAt = lines.findIndex((line) => line.startsWith('# '))
  if (titleAt === -1) return `${block}\n\n${text}`.trim()

  const head = lines.slice(0, titleAt + 1).join('\n')
  const rest = lines
    .slice(titleAt + 1)
    .join('\n')
    .trim()
  return rest ? `${head}\n\n${block}\n\n${rest}` : `${head}\n\n${block}`
}

/**
 * A filename stem the save dialog can carry on Windows.
 *
 * The dialog is what finally names the file, but a default path holding `?` or
 * `:` is rejected before the writer ever sees it.
 */
export function reportFileName(title: string): string {
  const cleaned = title
    .normalize('NFKC')
    .replace(/[<>:"/\\|?*\p{Cc}]/gu, '')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, MAX_FILENAME_STEM_LENGTH)
  return cleaned || 'informe'
}

/**
 * The bytes of one format.
 *
 * HTML and Word are the same document: the printable wrapper the OCR and chat
 * exports already write, so a report opens with the same margins and the same
 * table rules as everything else this app puts on disk.
 */
export async function investigationReportBytes(
  format: ExportFormat,
  report: InvestigationReport,
  generators: Partial<InvestigationExportGenerators> = {}
): Promise<Uint8Array> {
  const markdown = reportMarkdownWithQuery(report)
  if (format === 'markdown') return new TextEncoder().encode(markdown)

  const html = buildPrintableHtml(renderMarkdown(markdown))
  if (format === 'html') return new TextEncoder().encode(html)

  return (generators.docx ?? generateDocxBytes)(html)
}

/** How a download ended. `null` is a closed file dialog, which says nothing. */
export type InvestigationDownload =
  | { kind: 'saved'; path: string }
  | { kind: 'cancelled' }
  | { kind: 'failed'; message: string }

/**
 * Asks where to put it and writes it.
 *
 * The name is asked for first, unlike the manuscript export: that one can
 * refuse a format after rendering, and has to say so before the writer picks a
 * folder. Nothing here can refuse — every format takes the same text — so the
 * cheapest order is the plain one.
 */
export async function downloadInvestigationReport(
  report: InvestigationReport,
  format: ExportFormat,
  defaultName: string,
  generators: Partial<InvestigationExportGenerators> = {}
): Promise<InvestigationDownload> {
  const option = EXTENSION[format]
  try {
    const path = await save({
      defaultPath: `${defaultName}.${option.extension}`,
      filters: [{ name: option.name, extensions: [option.extension] }],
    })
    if (!path) return { kind: 'cancelled' }

    await writeFile(path, await investigationReportBytes(format, report, generators))
    return { kind: 'saved', path }
  } catch (error) {
    return { kind: 'failed', message: error instanceof Error ? error.message : String(error) }
  }
}
