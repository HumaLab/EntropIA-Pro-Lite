/**
 * Relation graph of a collection (T-46): every subject and object of the
 * extracted triples is a node, every triple a link between them labelled with
 * its predicate. The same name written with other case or spacing is one node.
 */

import { normalizeGraphSettings, type GraphStructureSettings } from './analysis-settings'

export interface RelationTriple {
  subject: string
  predicate: string
  object: string
  /** The document the triple was read from; the click details list it. */
  itemId?: string
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
  /** Nodes left out by filters or caps, so the view can say so. */
  hiddenNodes: number
}

/** More nodes than this turn a 3D graph into a hairball and slow the WebView. */
export const MAX_GRAPH_NODES = 300

export const key = (name: string) => name.trim().replace(/\s+/g, ' ').toLocaleLowerCase()

export function buildRelationGraph(
  triples: readonly RelationTriple[],
  options: number | Partial<GraphStructureSettings> = MAX_GRAPH_NODES
): RelationGraph {
  const settings = normalizeGraphSettings({
    structure: typeof options === 'number' ? { maxEntities: options } : options,
  }).structure
  const asSubject = new Map<string, number>()
  const asObject = new Map<string, number>()
  const weight = new Map<GraphLink, number>()
  const labels = new Map<string, string>()
  const pairs = new Map<string, GraphLink>()
  const degree = new Map<string, number>()

  for (const triple of triples) {
    const source = key(triple.subject)
    const target = key(triple.object)
    // Count mentions per role and input occurrence, even for a self-reference.
    if (source) asSubject.set(source, (asSubject.get(source) ?? 0) + 1)
    if (target) asObject.set(target, (asObject.get(target) ?? 0) + 1)
    if (source && !labels.has(source)) labels.set(source, triple.subject.trim())
    if (target && !labels.has(target)) labels.set(target, triple.object.trim())
    // Empty and self pairs name an entity but never make a link.
    if (!source || !target || source === target) continue

    const pairKey = `${source}\u0000${target}`
    let link = pairs.get(pairKey)
    if (!link) {
      link = { source, target, predicates: [] }
      pairs.set(pairKey, link)
      degree.set(source, (degree.get(source) ?? 0) + 1)
      degree.set(target, (degree.get(target) ?? 0) + 1)
    }
    weight.set(link, (weight.get(link) ?? 0) + 1)
    const predicate = triple.predicate.trim()
    if (predicate && !link.predicates.includes(predicate)) link.predicates.push(predicate)
  }

  // Thresholds admit entities; an admitted entity brings every relation that
  // survives the weight filter and the entities on the other end, however
  // rare. A cap admits whole stars only, so no filter can leave a node without
  // the links that put it there. Order: aggregate; keep relations by weight;
  // admit seeds (degree, then role frequency) ranked by degree, mentions and
  // first appearance while their star fits both caps; add the relations among
  // admitted entities that still fit; then entities with no relation at all
  // in the data (only self-references), when asked. Links keep input order;
  // nodes keep original degree for sizing.
  const eligible = [...pairs.values()].filter(
    (link) => weight.get(link)! >= settings.minRelationWeight
  )
  const star = new Map<string, GraphLink[]>()
  for (const link of eligible) {
    for (const id of [link.source, link.target]) {
      const links = star.get(id)
      if (links) links.push(link)
      else star.set(id, [link])
    }
  }
  const order = [...labels.keys()]
  const mentions = (id: string) => (asSubject.get(id) ?? 0) + (asObject.get(id) ?? 0)
  const seeds = order
    .filter(
      (id) =>
        (degree.get(id) ?? 0) >= settings.minDegree &&
        ((asSubject.get(id) ?? 0) >= settings.minSubjectFrequency ||
          (asObject.get(id) ?? 0) >= settings.minObjectFrequency)
    )
    .sort(
      (a, b) =>
        (degree.get(b) ?? 0) - (degree.get(a) ?? 0) ||
        mentions(b) - mentions(a) ||
        order.indexOf(a) - order.indexOf(b)
    )
  const admitted = new Set<string>()
  const shown = new Set<GraphLink>()
  for (const seed of seeds) {
    const links = (star.get(seed) ?? []).filter((link) => !shown.has(link))
    if (!links.length) continue
    const newcomers = new Set(
      [seed, ...links.flatMap((link) => [link.source, link.target])].filter(
        (id) => !admitted.has(id)
      )
    )
    if (admitted.size + newcomers.size > settings.maxEntities) continue
    if (shown.size + links.length > settings.maxRelations) continue
    for (const id of newcomers) admitted.add(id)
    for (const link of links) shown.add(link)
  }
  for (const link of eligible) {
    if (shown.size >= settings.maxRelations) break
    if (admitted.has(link.source) && admitted.has(link.target)) shown.add(link)
  }
  // Truly isolated: named in the data but never in a link (self-references).
  const isolated = settings.includeIsolated
    ? seeds.filter((id) => (degree.get(id) ?? 0) === 0 && !admitted.has(id))
    : []
  for (const id of isolated) {
    if (admitted.size >= settings.maxEntities) break
    admitted.add(id)
  }
  const visible = order.filter((id) => admitted.has(id))

  return {
    nodes: visible.map((id) => ({ id, label: labels.get(id)!, degree: degree.get(id) ?? 0 })),
    links: eligible.filter((link) => shown.has(link)),
    hiddenNodes: labels.size - visible.length,
  }
}

export interface NodeDetails {
  /** The node's triples as written, deduplicated, in first-seen order. */
  relations: RelationTriple[]
  /** Documents those triples come from, in first-seen order. */
  itemIds: string[]
}

/** What the 3D view shows when a node is clicked: its relations and the
 *  documents they were read from. */
export function describeNode(id: string, triples: readonly RelationTriple[]): NodeDetails {
  const relations: RelationTriple[] = []
  const seen = new Set<string>()
  const itemIds: string[] = []
  for (const triple of triples) {
    const source = key(triple.subject)
    const target = key(triple.object)
    if (source === target || (source !== id && target !== id)) continue
    const relation = {
      subject: triple.subject.trim(),
      predicate: triple.predicate.trim(),
      object: triple.object.trim(),
    }
    const relationKey = `${source}\u0000${key(relation.predicate)}\u0000${target}`
    if (!seen.has(relationKey)) {
      seen.add(relationKey)
      relations.push(relation)
    }
    if (triple.itemId && !itemIds.includes(triple.itemId)) itemIds.push(triple.itemId)
  }
  return { relations, itemIds }
}
