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

type QualifiedIdentity = {
  sourceOrigin: string
  sourceInstanceId: string | null
  libraryType: string
  libraryId: string
}

/** A fully qualified item is safe to namespace; partial identity stays legacy. */
function qualifiedIdentityOf(item: Record<string, unknown>): QualifiedIdentity | null {
  if (
    typeof item.sourceOrigin !== 'string' ||
    typeof item.libraryType !== 'string' ||
    typeof item.libraryId !== 'string' ||
    (item.sourceInstanceId !== null && typeof item.sourceInstanceId !== 'string')
  ) {
    return null
  }

  return {
    sourceOrigin: item.sourceOrigin,
    sourceInstanceId: item.sourceInstanceId,
    libraryType: item.libraryType,
    libraryId: item.libraryId,
  }
}

/** Keep each identity component safe from the namespace separator. */
function escapeIdentityComponent(value: string): string {
  return encodeURIComponent(value).replace(
    /[!'()*]/g,
    (character) => `%${character.charCodeAt(0).toString(16).toUpperCase()}`
  )
}

function derivedCslId(
  item: Record<string, unknown>,
  snapshot: Record<string, unknown>,
  identity: QualifiedIdentity,
  citationNodeId: unknown,
  itemPosition: number
): string {
  const baseId = readString(item.itemKey) ?? readString(snapshot.id) ?? 'work'
  const instance =
    identity.sourceInstanceId === null
      ? 'instance-null'
      : `instance-value-${escapeIdentityComponent(identity.sourceInstanceId)}`
  const parts = [
    'csl-identity',
    escapeIdentityComponent(baseId),
    escapeIdentityComponent(identity.sourceOrigin),
    escapeIdentityComponent(identity.libraryType),
    escapeIdentityComponent(identity.libraryId),
    instance,
  ]

  if (identity.sourceInstanceId === null) {
    parts.push(
      escapeIdentityComponent(typeof citationNodeId === 'string' ? citationNodeId : ''),
      String(itemPosition)
    )
  }

  return parts.join(':')
}

/**
 * Only the renderer's derived copy may receive a namespaced id. The canonical
 * metadata snapshot object carried by the citation is never modified.
 */
function derivedSnapshotOf(
  item: Record<string, unknown>,
  citationNodeId: unknown,
  itemPosition: number
): string {
  const snapshot = snapshotOf(item)
  const identity = qualifiedIdentityOf(item)
  if (!identity) return snapshot

  let parsed: unknown
  try {
    parsed = JSON.parse(snapshot)
  } catch {
    return snapshot
  }
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return snapshot

  const csl = parsed as Record<string, unknown>
  return JSON.stringify({
    ...csl,
    id: derivedCslId(item, csl, identity, citationNodeId, itemPosition),
  })
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
      csl_json: derivedSnapshotOf(item, attrs.citationNodeId, index),
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
 * The identity is the derived CSL id, because that is what the engine's
 * bibliography is keyed by. Qualified citations use source identity there;
 * legacy citations keep their stored CSL id. A work cited twice is one entry —
 * a bibliography that listed it twice would be read as a mistake by the author.
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
