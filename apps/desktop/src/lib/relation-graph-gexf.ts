/**
 * The relation graph as a GEXF 1.3 file, the format Gephi opens directly
 * (T-63). It writes the graph as drawn: the same nodes and links the 3D view
 * shows, so filters and caps apply to the file too.
 */

import type { RelationGraph } from './relation-graph'

/** XML 1.0 forbids most control characters even escaped; OCR text carries them. */
const xml = (text: string) =>
  text
    // eslint-disable-next-line no-control-regex
    .replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F￾￿]/g, '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')

/**
 * `types` maps a node id to the entity type its name was tagged with, if any.
 * A link carries its predicates as the label and the number of triples that
 * state them as the weight.
 */
export function relationGraphToGexf(
  graph: Pick<RelationGraph, 'nodes' | 'links'>,
  types: ReadonlyMap<string, string> = new Map()
): string {
  const nodes = graph.nodes.map((node) => {
    const type = types.get(node.id)
    const attrs = type ? `<attvalues><attvalue for="0" value="${xml(type)}"/></attvalues>` : ''
    return `      <node id="${xml(node.id)}" label="${xml(node.label)}">${attrs}</node>`
  })
  const edges = graph.links.map((link, index) => {
    const weight = link.counts.reduce((sum, count) => sum + count, 0) || 1
    const label = xml(link.predicates.join(' · '))
    return `      <edge id="${index}" source="${xml(link.source)}" target="${xml(link.target)}" label="${label}" weight="${weight}"/>`
  })
  return [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<gexf xmlns="http://gexf.net/1.3" version="1.3">',
    '  <meta><creator>EntropIA</creator></meta>',
    '  <graph mode="static" defaultedgetype="directed">',
    '    <attributes class="node"><attribute id="0" title="type" type="string"/></attributes>',
    '    <nodes>',
    ...nodes,
    '    </nodes>',
    '    <edges>',
    ...edges,
    '    </edges>',
    '  </graph>',
    '</gexf>',
    '',
  ].join('\n')
}
