import { writable, type Readable } from 'svelte/store'
import {
  CITATION_FIDELITY,
  type CitationRepresentation,
  type ExportFormat,
} from './export-fidelity'
import type { I18nKey } from './i18n'
import { settingsGet, settingsSet, SETTINGS_KEYS } from './settings'

/**
 * How a manuscript is exported (plan-editor.md §17), chosen once and applied to
 * every download.
 *
 * # Why these are the writer's and not the document's
 *
 * §17.2 closes with *"La configuración de exportación no alterará el documento
 * canónico"*, so nothing here is written into a manuscript. And a writer who
 * sends every chapter out as quoted text with a note does not want to say so
 * again for each chapter, so the choice is kept in the application settings,
 * where it survives switching documents and restarting.
 *
 * # Why the format is not one of them
 *
 * The format is what the download button asks for, every time. What is decided
 * here holds for all three formats; where one of them cannot do what was
 * chosen, `citationsForFormat` says what it does instead, and the export says so.
 */

export interface ExportPreferences {
  citations: CitationRepresentation
  /** Whether a bibliography is appended (§11.6). */
  bibliography: boolean
}

/**
 * The first-run choice, and only that: once anything has been saved, the saved
 * value is used, including a saved `false`.
 *
 * The quoted fragment stays where the writer put it, as in the editor.
 */
export const DEFAULT_EXPORT_PREFERENCES: ExportPreferences = Object.freeze({
  citations: 'quote_with_note',
  bibliography: true,
})

/** §17.2's four representations, in the order the Export tab offers them. */
export const CITATION_CHOICES: readonly { id: CitationRepresentation; label: I18nKey }[] = [
  { id: 'footnote', label: 'writing.exportCiteFootnote' },
  { id: 'inline', label: 'writing.exportCiteInline' },
  { id: 'comment', label: 'writing.exportCiteComment' },
  { id: 'quote_with_note', label: 'writing.exportCiteQuote' },
]

function isRepresentation(value: string | null): value is CitationRepresentation {
  return value !== null && value in CITATION_FIDELITY
}

/**
 * Reads the two stored strings. A value this version does not recognise is
 * treated as absent rather than guessed at.
 */
export function parseExportPreferences(
  citations: string | null,
  bibliography: string | null
): ExportPreferences {
  return {
    citations: isRepresentation(citations) ? citations : DEFAULT_EXPORT_PREFERENCES.citations,
    bibliography:
      bibliography === 'true'
        ? true
        : bibliography === 'false'
          ? false
          : DEFAULT_EXPORT_PREFERENCES.bibliography,
  }
}

/**
 * What a format actually writes for the chosen representation.
 *
 * Markdown has no comment that survives rendering (CITATION_FIDELITY), so a
 * comment goes out as a footnote there — the one representation every format
 * carries natively. The export reports the substitution; the preference itself
 * is left alone, so the next DOCX still gets its comments.
 */
export function citationsForFormat(
  citations: CitationRepresentation,
  format: ExportFormat
): CitationRepresentation {
  return CITATION_FIDELITY[citations][format] === 'unsupported' ? 'footnote' : citations
}

const store = writable<ExportPreferences>({ ...DEFAULT_EXPORT_PREFERENCES })

/** The preferences in force. Shared by the Export tab and the download button. */
export const exportPreferences: Readable<ExportPreferences> = { subscribe: store.subscribe }

let loading: Promise<void> | null = null
/** Set once the writer has chosen, so a slow first read cannot undo the choice. */
let chosen = false

/**
 * Reads the saved preferences, once per application run.
 *
 * A read that fails leaves the defaults in place: not being able to recall a
 * preference is no reason to refuse an export.
 */
export function loadExportPreferences(): Promise<void> {
  loading ??= (async () => {
    try {
      const [citations, bibliography] = await Promise.all([
        settingsGet(SETTINGS_KEYS.WRITING_EXPORT_CITATIONS),
        settingsGet(SETTINGS_KEYS.WRITING_EXPORT_BIBLIOGRAPHY),
      ])
      if (!chosen) store.set(parseExportPreferences(citations, bibliography))
    } catch {
      // The defaults stay; see above.
    }
  })()
  return loading
}

/**
 * Applies a choice at once and saves it.
 *
 * Both values are saved together, so from the first change on neither of them
 * is a default any more. A write that fails costs the choice at the next
 * start, not the choice on screen.
 */
export async function setExportPreferences(next: ExportPreferences): Promise<void> {
  chosen = true
  store.set({ ...next })
  try {
    await Promise.all([
      settingsSet(SETTINGS_KEYS.WRITING_EXPORT_CITATIONS, next.citations),
      settingsSet(SETTINGS_KEYS.WRITING_EXPORT_BIBLIOGRAPHY, String(next.bibliography)),
    ])
  } catch {
    // See above.
  }
}
