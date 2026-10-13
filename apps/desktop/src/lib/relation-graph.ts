/**
 * Relation graph of a collection (T-46): every subject and object of the
 * extracted triples is a node, every triple a link between them labelled with
 * its predicate. The same name written with other case or spacing is one node.
 */

export interface RelationTriple {
  subject: string
  predicate: string
  object: string
}

export interface GraphNode {
  id: string
  label: string
  /** Links touching this node; the 3D view sizes nodes by it. */
  degree: number
}

export interface GraphLink {
  source: string
  target: string
  /** Distinct predicates between the pair, in first-seen order. */
  predicates: string[]
}

export interface RelationGraph {
  nodes: GraphNode[]
  links: GraphLink[]
  /** Nodes left out by the cap, so the view can say so. */
  hiddenNodes: number
}

/** More nodes than this turn a 3D graph into a hairball and slow the WebView. */
export const MAX_GRAPH_NODES = 300

const key = (name: string) => name.trim().replace(/\s+/g, ' ').toLocaleLowerCase()

export function buildRelationGraph(
  triples: readonly RelationTriple[],
  maxNodes = MAX_GRAPH_NODES
): RelationGraph {
  const labels = new Map<string, string>()
  const pairs = new Map<string, GraphLink>()
  const degree = new Map<string, number>()

  for (const triple of triples) {
    const source = key(triple.subject)
    const target = key(triple.object)
    if (!source || !target || source === target) continue
    if (!labels.has(source)) labels.set(source, triple.subject.trim())
    if (!labels.has(target)) labels.set(target, triple.object.trim())

    const pairKey = `${source}\u0000${target}`
    let link = pairs.get(pairKey)
    if (!link) {
      link = { source, target, predicates: [] }
      pairs.set(pairKey, link)
      degree.set(source, (degree.get(source) ?? 0) + 1)
      degree.set(target, (degree.get(target) ?? 0) + 1)
    }
    const predicate = triple.predicate.trim()
    if (predicate && !link.predicates.includes(predicate)) link.predicates.push(predicate)
  }

  // Keep the best-connected nodes; ties keep first-seen order (sort is stable).
  const kept = [...labels.keys()]
    .sort((a, b) => (degree.get(b) ?? 0) - (degree.get(a) ?? 0))
    .slice(0, maxNodes)
  const keptSet = new Set(kept)

  return {
    nodes: kept.map((id) => ({ id, label: labels.get(id)!, degree: degree.get(id) ?? 0 })),
    links: [...pairs.values()].filter((l) => keptSet.has(l.source) && keptSet.has(l.target)),
    hiddenNodes: labels.size - kept.length,
  }
}
