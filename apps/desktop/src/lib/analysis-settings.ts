/**
 * Per-collection persistence for the text-analysis panel settings
 * (text and relation graph), stored in localStorage.
 */

export interface CollectionAnalysisSettings {
  cloudTermCount: number
  customStopwords: string[]
  graph: GraphSettings
}

export interface GraphStructureSettings {
  maxEntities: number
  maxRelations: number
  minDegree: number
  /** Times an entity must be the subject of a triple to enter the graph on its own. */
  minSubjectFrequency: number
  /** Times an entity must be the object of a triple to enter the graph on its own. */
  minObjectFrequency: number
  minRelationWeight: number
  includeIsolated: boolean
}

export interface GraphRenderSettings {
  nodeSizeScale: number
  linkOpacity: number
  showArrows: boolean
  showLinkLabels: boolean
  linkDistance: number
  chargeStrength: number
  cooldownTime: number
  autoRotate: boolean
}

export interface GraphSettings {
  structure: GraphStructureSettings
  render: GraphRenderSettings
}

/** Numeric UI limits; only counts and cooldown milliseconds are rounded. */
export const GRAPH_SETTINGS_BOUNDS = {
  structure: {
    maxEntities: { min: 1, max: 3000, integer: true },
    maxRelations: { min: 1, max: 10000, integer: true },
    minDegree: { min: 0, max: 3000, integer: true },
    minSubjectFrequency: { min: 1, max: 100000, integer: true },
    minObjectFrequency: { min: 1, max: 100000, integer: true },
    minRelationWeight: { min: 1, max: 100000, integer: true },
  },
  render: {
    nodeSizeScale: { min: 0.1, max: 10, integer: false },
    linkOpacity: { min: 0, max: 1, integer: false },
    linkDistance: { min: 1, max: 500, integer: false },
    chargeStrength: { min: -1000, max: 0, integer: false },
    cooldownTime: { min: 0, max: 60000, integer: true },
  },
} as const

/**
 * 300 entities preserves the previous cap; 1000 links bounds dense graphs
 * while leaving typical graphs unchanged. Isolates were previously retained.
 * nodeVal = nodeSizeScale * (1 + degree); arrows retain length 3 / position 1.
 * Opacity 0.2, link distance 30, charge -30 and cooldown 15000 ms are the
 * 3d-force-graph / d3 defaults (previously not overridden by the component).
 */
export const DEFAULT_GRAPH_SETTINGS: Readonly<{
  structure: Readonly<GraphStructureSettings>
  render: Readonly<GraphRenderSettings>
}> = Object.freeze({
  structure: Object.freeze({
    maxEntities: 300,
    maxRelations: 1000,
    minDegree: 0,
    minSubjectFrequency: 1,
    minObjectFrequency: 1,
    minRelationWeight: 1,
    includeIsolated: true,
  }),
  render: Object.freeze({
    nodeSizeScale: 1,
    linkOpacity: 0.2,
    showArrows: true,
    showLinkLabels: true,
    linkDistance: 30,
    chargeStrength: -30,
    cooldownTime: 15000,
    autoRotate: false,
  }),
})

function record(value: unknown): Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {}
}

/**
 * Settings saved before the per-role thresholds held one `minFrequency`: it
 * becomes both, unless a role already has its own value.
 */
function legacyStructure(raw: unknown): Record<string, unknown> {
  const values = record(raw)
  const { minFrequency, ...rest } = values
  if (minFrequency === undefined) return values
  return {
    minSubjectFrequency: minFrequency,
    minObjectFrequency: minFrequency,
    ...rest,
  }
}

/** Strict types: corrupt fields fall back independently, not via truthiness/coercion. */
export function normalizeGraphSettings(value: unknown): GraphSettings {
  const input = record(value)
  function group<T extends object>(
    defaults: T,
    raw: unknown,
    bounds: Record<string, { min: number; max: number; integer: boolean }>
  ): T {
    const values = record(raw)
    return Object.fromEntries(
      Object.entries(defaults).map(([field, fallback]) => {
        const value = values[field]
        const bound = bounds[field]
        if (typeof fallback === 'boolean') {
          return [field, typeof value === 'boolean' ? value : fallback]
        }
        return [
          field,
          bound && typeof value === 'number' && Number.isFinite(value)
            ? Math.max(bound.min, Math.min(bound.max, bound.integer ? Math.round(value) : value))
            : fallback,
        ]
      })
    ) as T
  }
  return {
    structure: group(
      DEFAULT_GRAPH_SETTINGS.structure,
      legacyStructure(input.structure),
      GRAPH_SETTINGS_BOUNDS.structure
    ),
    render: group(DEFAULT_GRAPH_SETTINGS.render, input.render, GRAPH_SETTINGS_BOUNDS.render),
  }
}

/** Render-only edits do not invalidate the graph built from already-loaded triples. */
export function graphStructureKey(settings: GraphSettings): string {
  return JSON.stringify(normalizeGraphSettings(settings).structure)
}

/** Pure reset; callers persist through saveAnalysisSettings, as for text settings. */
export function resetGraphSettings(
  settings: CollectionAnalysisSettings
): CollectionAnalysisSettings {
  return { ...settings, graph: normalizeGraphSettings(undefined) }
}

export const MIN_CLOUD_TERMS = 20
export const MAX_CLOUD_TERMS = 100
export const DEFAULT_CLOUD_TERMS = 50

const STORAGE_PREFIX = 'entropia-collection-analysis-settings:'

export function defaultAnalysisSettings(): CollectionAnalysisSettings {
  return {
    cloudTermCount: DEFAULT_CLOUD_TERMS,
    customStopwords: [],
    graph: normalizeGraphSettings(undefined),
  }
}

export function clampCloudTermCount(value: number): number {
  if (!Number.isFinite(value)) return DEFAULT_CLOUD_TERMS
  return Math.max(MIN_CLOUD_TERMS, Math.min(MAX_CLOUD_TERMS, Math.round(value)))
}

/**
 * Parse free-form user input (comma / semicolon / whitespace separated)
 * into a normalized, deduplicated stopword list. Normalization matches
 * the tokenizer (NFC + lowercase) so entries always hit the lookup Set.
 */
export function parseStopwordsInput(raw: string): string[] {
  const seen = new Set<string>()
  for (const part of raw.split(/[\s,;]+/)) {
    const word = part.trim().normalize('NFC').toLowerCase()
    if (word) seen.add(word)
  }
  return [...seen]
}

function storageKey(collectionId: string): string {
  return `${STORAGE_PREFIX}${collectionId}`
}

export function loadAnalysisSettings(collectionId: string): CollectionAnalysisSettings {
  try {
    const raw = localStorage.getItem(storageKey(collectionId))
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<CollectionAnalysisSettings>
      return {
        graph: normalizeGraphSettings(parsed.graph),
        cloudTermCount: clampCloudTermCount(Number(parsed.cloudTermCount)),
        customStopwords: Array.isArray(parsed.customStopwords)
          ? parseStopwordsInput(parsed.customStopwords.join(' '))
          : [],
      }
    }
  } catch {
    // corrupt entry or storage unavailable — fall back to defaults
  }
  return defaultAnalysisSettings()
}

export function saveAnalysisSettings(
  collectionId: string,
  settings: Omit<CollectionAnalysisSettings, 'graph'> & { graph?: GraphSettings }
): void {
  try {
    localStorage.setItem(
      storageKey(collectionId),
      JSON.stringify({ ...settings, graph: normalizeGraphSettings(settings.graph) })
    )
  } catch {
    // storage unavailable — settings stay session-only
  }
}
