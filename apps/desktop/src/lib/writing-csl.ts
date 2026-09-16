import { invoke } from '@tauri-apps/api/core'

/**
 * Rendering citations and the bibliography (plan-editor.md §11.5, §11.6).
 *
 * # Why the rendered text is never the truth
 *
 * §11.5 asks for CSL-equivalent data to be stored, never the rendered string.
 * The node keeps a `renderedText` all the same, for one narrow reason: the
 * engine is asked asynchronously, and a citation that showed nothing until the
 * answer came back would blink on every reload. So the last rendering is kept
 * as *what to draw meanwhile*, and it is replaced, never trusted.
 *
 * That distinction is what makes a change of style work. The style is applied
 * by re-rendering every cluster from its stored CSL data; if the text were the
 * truth, changing style would leave the old strings exactly where they were.
 */

export interface ClusterItem {
  csl_json: string
  locator?: string | null
  locator_kind?: string | null
  prefix?: string | null
  suffix?: string | null
  suppress_author?: boolean
}

export type StyleSource =
  | { kind: 'bundled'; name: string }
  | { kind: 'custom'; xml: string }

export interface RenderedCluster {
  text: string
  /** False when suppression was asked for and could not be done. */
  author_suppressed: boolean
}

export interface CslError {
  code: string
  message: string
}

export interface StyleInfo {
  id: string
  title: string
}

/** The style a manuscript uses until it says otherwise. */
export const DEFAULT_STYLE: StyleSource = { kind: 'bundled', name: 'apa' }

function asError(error: unknown): CslError {
  if (typeof error === 'object' && error !== null && 'code' in error && 'message' in error) {
    const shaped = error as CslError
    return { code: String(shaped.code), message: String(shaped.message) }
  }
  return { code: 'unknown', message: error instanceof Error ? error.message : String(error) }
}

/**
 * Renders one cluster, or reports why it could not.
 *
 * Returns the error rather than throwing, because a citation that cannot be
 * rendered must not stop the manuscript being edited or saved. §11.3 says that
 * about Zotero; the same holds for a style file that turned out to be broken.
 */
export async function renderCluster(
  items: ClusterItem[],
  style: StyleSource = DEFAULT_STYLE
): Promise<RenderedCluster | CslError> {
  try {
    return await invoke<RenderedCluster>('writing_csl_render', { items, style })
  } catch (error) {
    return asError(error)
  }
}

/**
 * Renders every citation of the manuscript together (§11.5).
 *
 * Disambiguation is a property of the document: which of two works by one
 * author in one year reads `2015a` depends on all the others. Asking about one
 * citation at a time can only ever produce `2015` twice, so this is how a
 * manuscript's citations are rendered and `renderCluster` is kept for a preview
 * of one that is still being edited.
 */
export async function renderDocument(
  clusters: ClusterItem[][],
  style: StyleSource = DEFAULT_STYLE
): Promise<RenderedCluster[] | CslError> {
  try {
    return await invoke<RenderedCluster[]>('writing_csl_render_document', { clusters, style })
  } catch (error) {
    return asError(error)
  }
}

/** The bibliography: a derived view of what the manuscript cites (§11.6). */
export async function renderBibliography(
  cited: string[],
  style: StyleSource = DEFAULT_STYLE
): Promise<string[] | CslError> {
  try {
    return await invoke<string[]>('writing_csl_bibliography', { cited, style })
  } catch (error) {
    return asError(error)
  }
}

/** Checks a `.csl` file before it is ever chosen (§11.6). */
export async function validateStyle(xml: string): Promise<StyleInfo | CslError> {
  try {
    return await invoke<StyleInfo>('writing_csl_validate_style', { xml })
  } catch (error) {
    return asError(error)
  }
}

export function isCslError(value: unknown): value is CslError {
  return typeof value === 'object' && value !== null && 'code' in value && 'message' in value
}
