import { worksOf } from '@entropia/ui'
import type { ClusterItem } from './writing-csl'

/**
 * Turning a citation node into what the CSL engine reads (plan-editor.md §11.5).
 *
 * # Why this is shared
 *
 * Two callers need it: the editor, which re-renders every citation whenever the
 * document changes, and the exporter, which renders them all again before
 * writing a file. Two copies of this mapping would drift, and the way they
 * would drift is the worst kind — the manuscript on screen and the manuscript
 * in the exported file would cite the same works differently, and nothing would
 * report it.
 */

function readString(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value : null
}

function snapshotOf(item: Record<string, unknown>): string {
  return typeof item.metadataSnapshot === 'string'
    ? item.metadataSnapshot
    : JSON.stringify(item.metadataSnapshot ?? {})
}

/**
 * One cluster, in the shape the engine reads.
 *
 * Read through `worksOf` rather than `attrs.items`: a citation written before a
 * citation could hold several works keeps its single work on the node itself,
 * and reading only the array would render it as nothing.
 */
export function clusterOf(attrs: Record<string, unknown>): ClusterItem[] {
  return worksOf({ attrs }).map((raw, index) => {
    const item = (raw ?? {}) as Record<string, unknown>
    return {
      csl_json: snapshotOf(item),
      locator: readString(item.locator),
      locator_kind: readString(item.locatorType),
      // The affixes belong to the cluster, not to one of its works, so they
      // ride on its first.
      prefix: index === 0 ? readString(attrs.prefix) : null,
      suffix: index === 0 ? readString(attrs.suffix) : null,
      suppress_author: item.suppressAuthor === true,
    }
  })
}

/**
 * The works a set of clusters cites, each once, in the order first cited, as
 * the CSL-JSON `writing_csl_bibliography` parses — not as their ids. Handing it
 * bare ids is what made serde answer "expected value at line 1 column 1".
 *
 * The identity is the CSL id, because that is what the engine's bibliography is
 * keyed by. A work cited twice is one entry — a bibliography that listed it
 * twice would be read as a mistake by the author.
 */
export function citedWorks(clusters: ClusterItem[][]): string[] {
  const works: string[] = []
  const seen = new Set<string>()

  for (const cluster of clusters) {
    for (const item of cluster) {
      let id: unknown = null
      try {
        id = (JSON.parse(item.csl_json) as { id?: unknown }).id
      } catch {
        // A snapshot that is not JSON cannot name a work. Skipped rather than
        // guessed at: an invented key would put a wrong entry in the
        // bibliography, which is worse than a missing one.
        continue
      }
      if (typeof id === 'string' && !seen.has(id)) {
        seen.add(id)
        works.push(item.csl_json)
      }
    }
  }

  return works
}
