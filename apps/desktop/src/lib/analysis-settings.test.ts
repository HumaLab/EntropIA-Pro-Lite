import { describe, it, expect, beforeEach } from 'vitest'
import {
  DEFAULT_GRAPH_SETTINGS,
  GRAPH_SETTINGS_BOUNDS,
  normalizeGraphSettings,
  resetGraphSettings,
  graphStructureKey,
  clampCloudTermCount,
  defaultAnalysisSettings,
  loadAnalysisSettings,
  parseStopwordsInput,
  saveAnalysisSettings,
  DEFAULT_CLOUD_TERMS,
  MAX_CLOUD_TERMS,
  MIN_CLOUD_TERMS,
} from './analysis-settings'

beforeEach(() => {
  localStorage.clear()
})

describe('graph settings', () => {
  it('loads legacy settings with graph defaults and persists normalized graph values', () => {
    saveAnalysisSettings('old', { cloudTermCount: 80, customStopwords: ['mar'] })
    expect(loadAnalysisSettings('old').graph).toEqual(DEFAULT_GRAPH_SETTINGS)
    const graph = normalizeGraphSettings({
      structure: { maxEntities: 12.7 },
      render: { autoRotate: true },
    })
    saveAnalysisSettings('new', { ...defaultAnalysisSettings(), graph })
    expect(loadAnalysisSettings('new').graph).toEqual(graph)
  })

  it('clamps and rounds every numeric field and falls back per corrupt field', () => {
    for (const group of ['structure', 'render'] as const) {
      for (const [field, bounds] of Object.entries(GRAPH_SETTINGS_BOUNDS[group])) {
        const get = (value: unknown) =>
          Reflect.get(normalizeGraphSettings({ [group]: { [field]: value } })[group], field)
        expect(get(-Infinity)).toBe(Reflect.get(DEFAULT_GRAPH_SETTINGS[group], field))
        expect(get('12')).toBe(Reflect.get(DEFAULT_GRAPH_SETTINGS[group], field))
        expect(get(bounds.min - 1)).toBe(bounds.min)
        expect(get(bounds.max + 1)).toBe(bounds.max)
        expect(get(bounds.min + 0.6)).toBe(bounds.integer ? bounds.min + 1 : bounds.min + 0.6)
      }
    }
    expect(
      normalizeGraphSettings({
        structure: { includeIsolated: 'false' },
        render: { showArrows: 0, showLinkLabels: null, autoRotate: 'true' },
      })
    ).toEqual(DEFAULT_GRAPH_SETTINGS)
  })

  it('resets only graph settings without mutating the source', () => {
    const settings = {
      cloudTermCount: 80,
      customStopwords: ['mar'],
      graph: normalizeGraphSettings({ render: { autoRotate: true } }),
    }
    expect(resetGraphSettings(settings)).toEqual({ ...settings, graph: DEFAULT_GRAPH_SETTINGS })
    expect(settings.graph.render.autoRotate).toBe(true)
  })

  it('keys only structure changes, for every field', () => {
    const baseline = graphStructureKey(DEFAULT_GRAPH_SETTINGS)
    for (const group of ['structure', 'render'] as const) {
      for (const [field, value] of Object.entries(DEFAULT_GRAPH_SETTINGS[group])) {
        const changed = normalizeGraphSettings({
          [group]: { [field]: typeof value === 'boolean' ? !value : value + 1 },
        })
        expect(graphStructureKey(changed) === baseline).toBe(group === 'render')
      }
    }
  })
})

describe('clampCloudTermCount', () => {
  it('clamps to the 20-100 range and rounds', () => {
    expect(clampCloudTermCount(19)).toBe(MIN_CLOUD_TERMS)
    expect(clampCloudTermCount(101)).toBe(MAX_CLOUD_TERMS)
    expect(clampCloudTermCount(50)).toBe(50)
    expect(clampCloudTermCount(33.6)).toBe(34)
  })

  it('falls back to the default for non-finite values', () => {
    expect(clampCloudTermCount(Number.NaN)).toBe(DEFAULT_CLOUD_TERMS)
    expect(clampCloudTermCount(Number.POSITIVE_INFINITY)).toBe(DEFAULT_CLOUD_TERMS)
  })
})

describe('parseStopwordsInput', () => {
  it('splits on commas, semicolons, whitespace and newlines', () => {
    expect(parseStopwordsInput('fábrica, obrero;huelga\npescado  mar')).toEqual([
      'fábrica',
      'obrero',
      'huelga',
      'pescado',
      'mar',
    ])
  })

  it('normalizes NFC + lowercase and deduplicates', () => {
    expect(parseStopwordsInput('FÁBRICA fábrica fábrica')).toEqual(['fábrica'])
  })

  it('returns empty array for blank input', () => {
    expect(parseStopwordsInput('')).toEqual([])
    expect(parseStopwordsInput('  , ;\n ')).toEqual([])
  })
})

describe('load/save analysis settings', () => {
  it('returns defaults when nothing is stored', () => {
    expect(loadAnalysisSettings('col-1')).toEqual(defaultAnalysisSettings())
  })

  it('round-trips settings per collection', () => {
    saveAnalysisSettings('col-1', { cloudTermCount: 80, customStopwords: ['fábrica'] })
    saveAnalysisSettings('col-2', { cloudTermCount: 25, customStopwords: [] })

    expect(loadAnalysisSettings('col-1')).toEqual({
      ...defaultAnalysisSettings(),
      cloudTermCount: 80,
      customStopwords: ['fábrica'],
    })
    expect(loadAnalysisSettings('col-2')).toEqual({
      ...defaultAnalysisSettings(),
      cloudTermCount: 25,
    })
  })

  it('sanitizes stored values on load', () => {
    localStorage.setItem(
      'entropia-collection-analysis-settings:col-1',
      JSON.stringify({ cloudTermCount: 999, customStopwords: ['  FÁBRICA ', 'mar'] })
    )
    expect(loadAnalysisSettings('col-1')).toEqual({
      ...defaultAnalysisSettings(),
      cloudTermCount: MAX_CLOUD_TERMS,
      customStopwords: ['fábrica', 'mar'],
    })
  })

  it('falls back to defaults on corrupt storage', () => {
    localStorage.setItem('entropia-collection-analysis-settings:col-1', '{not json')
    expect(loadAnalysisSettings('col-1')).toEqual(defaultAnalysisSettings())

    localStorage.setItem(
      'entropia-collection-analysis-settings:col-2',
      JSON.stringify({ cloudTermCount: 'abc', customStopwords: 'no-array' })
    )
    expect(loadAnalysisSettings('col-2')).toEqual(defaultAnalysisSettings())
  })
})
