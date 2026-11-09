import { describe, it, expect } from 'vitest'
import {
  stripSpeakerLabels,
  tokenize,
  buildFrequencies,
  buildFrequenciesAsync,
  topN,
  computeTicks,
  truncateLabel,
  buildNgrams,
  buildCollocates,
  buildWordTree,
  type CorpusText,
} from './text-analysis'
import { DEFAULT_STOPWORDS, STOPWORDS_ES, STOPWORDS_EN } from './stopwords'

describe('stripSpeakerLabels', () => {
  it('strips "Hablante N:" labels (backend format)', () => {
    expect(stripSpeakerLabels('Hablante 1: hola a todos')).toBe('hola a todos')
    expect(stripSpeakerLabels('hablante 12:  sin espacio fijo')).toBe('sin espacio fijo')
  })

  it('strips legacy "Speaker N:" labels', () => {
    expect(stripSpeakerLabels('Speaker 2: hi there')).toBe('hi there')
  })

  it('strips manually edited capitalized aliases', () => {
    expect(stripSpeakerLabels('María Pérez: la fábrica cerró')).toBe('la fábrica cerró')
    expect(stripSpeakerLabels('Entrevistador: contame')).toBe('contame')
  })

  it('does not strip lowercase prefixes', () => {
    expect(stripSpeakerLabels('la reunión: detalles')).toBe('la reunión: detalles')
  })

  it('does not strip mid-sentence colons preceded by lowercase words', () => {
    expect(stripSpeakerLabels('Pescado del País: crónica')).toBe('Pescado del País: crónica')
  })

  it('handles multiline transcripts', () => {
    const input = 'Hablante 1: primera línea\nHablante 2: segunda línea\nsin etiqueta'
    expect(stripSpeakerLabels(input)).toBe('primera línea\nsegunda línea\nsin etiqueta')
  })
})

describe('tokenize', () => {
  it('lowercases and keeps accents', () => {
    expect(tokenize('Análisis HISTÓRICO')).toEqual(['análisis', 'histórico'])
  })

  it('normalizes to NFC before counting', () => {
    const decomposed = 'análisis' // 'analisis' with combining acute (NFD)
    expect(tokenize(decomposed)).toEqual(['análisis'])
  })

  it('strips punctuation', () => {
    expect(tokenize('¡fábrica!, conserva. (huelga)')).toEqual(['fábrica', 'conserva', 'huelga'])
  })

  it('removes Spanish and English stopwords', () => {
    expect(tokenize('el trabajo de la fábrica según los obreros')).toEqual([
      'trabajo',
      'fábrica',
      'obreros',
    ])
    expect(tokenize('the workers and the strike')).toEqual(['workers', 'strike'])
  })

  it('excludes isolated numbers but keeps alphanumeric compounds', () => {
    expect(tokenize('en 1934 hubo 120 huelgas y 3,14 promedios')).toEqual(['huelgas', 'promedios'])
    expect(tokenize('la covid-19 y el período 1944-1955')).toEqual([
      'covid-19',
      'período',
      '1944-1955',
    ])
  })

  it('drops tokens shorter than minTokenLength (default 3)', () => {
    expect(tokenize('fe luz mar')).toEqual(['luz', 'mar'])
    expect(tokenize('fe luz mar', { minTokenLength: 2 })).toEqual(['fe', 'luz', 'mar'])
  })

  it('trims apostrophes and hyphens at token edges', () => {
    expect(tokenize("'huelga- 'obreros'")).toEqual(['huelga', 'obreros'])
  })
})

describe('buildFrequencies', () => {
  it('counts frequencies across texts and sorts desc with alphabetical tiebreak', () => {
    const texts: CorpusText[] = [
      { text: 'fábrica fábrica huelga', kind: 'extraction' },
      { text: 'huelga conserva fábrica', kind: 'extraction' },
    ]
    expect(buildFrequencies(texts)).toEqual([
      { word: 'fábrica', count: 3 },
      { word: 'huelga', count: 2 },
      { word: 'conserva', count: 1 },
    ])
  })

  it('strips speaker labels only for transcriptions', () => {
    const texts: CorpusText[] = [
      { text: 'Hablante 1: huelga general', kind: 'transcription' },
      { text: 'Conclusión: huelga general', kind: 'extraction' },
    ]
    const result = buildFrequencies(texts)
    const words = result.map((f) => f.word)
    expect(words).not.toContain('hablante')
    expect(words).toContain('conclusión')
    expect(result.find((f) => f.word === 'huelga')!.count).toBe(2)
  })

  it('strips capitalized aliases in transcriptions but keeps them in extractions', () => {
    const transcription = buildFrequencies([
      { text: 'Entrevistadora: la huelga siguió', kind: 'transcription' },
    ])
    expect(transcription.map((f) => f.word)).not.toContain('entrevistadora')

    const extraction = buildFrequencies([
      { text: 'Entrevistadora: la huelga siguió', kind: 'extraction' },
    ])
    expect(extraction.map((f) => f.word)).toContain('entrevistadora')
  })

  it('returns empty array for empty corpus', () => {
    expect(buildFrequencies([])).toEqual([])
    expect(buildFrequencies([{ text: '  \n ', kind: 'extraction' }])).toEqual([])
  })
})

describe('buildFrequenciesAsync', () => {
  it('produces identical output to the sync version across chunk boundaries', async () => {
    const texts: CorpusText[] = Array.from({ length: 60 }, (_, i) => ({
      text: `fábrica huelga conserva palabra${i}`,
      kind: i % 2 === 0 ? ('extraction' as const) : ('transcription' as const),
    }))
    expect(await buildFrequenciesAsync(texts)).toEqual(buildFrequencies(texts))
  })
})

describe('topN', () => {
  it('returns the first N entries', () => {
    const freqs = [
      { word: 'a', count: 5 },
      { word: 'b', count: 3 },
      { word: 'c', count: 1 },
    ]
    expect(topN(freqs, 2)).toEqual(freqs.slice(0, 2))
    expect(topN(freqs, 10)).toEqual(freqs)
  })
})

describe('render helpers', () => {
  it('computeTicks returns nice ticks covering the max value', () => {
    expect(computeTicks(10, 4)).toEqual([0, 5, 10])
    expect(computeTicks(3, 4)).toEqual([0, 1, 2, 3])
    expect(computeTicks(0)).toEqual([0])
    const ticks = computeTicks(11, 4)
    expect(ticks[0]).toBe(0)
    expect(ticks[ticks.length - 1]).toBeGreaterThanOrEqual(11)
  })

  it('truncateLabel adds ellipsis beyond max length', () => {
    expect(truncateLabel('corta')).toBe('corta')
    expect(truncateLabel('extraordinariamente', 12)).toBe('extraordina…')
    expect(truncateLabel('extraordinariamente', 12).length).toBe(12)
  })
})

describe('stopword lists', () => {
  it('includes accentless variants for noisy OCR', () => {
    expect(DEFAULT_STOPWORDS.has('según')).toBe(true)
    expect(DEFAULT_STOPWORDS.has('segun')).toBe(true)
    expect(DEFAULT_STOPWORDS.has('más')).toBe(true)
    expect(DEFAULT_STOPWORDS.has('mas')).toBe(true)
  })

  it('merges both languages lowercase', () => {
    expect(STOPWORDS_ES.length).toBeGreaterThan(200)
    expect(STOPWORDS_EN.length).toBeGreaterThan(150)
    for (const word of DEFAULT_STOPWORDS) {
      expect(word).toBe(word.toLowerCase())
    }
  })
})

describe('phrases, associations and word trees', () => {
  const corpus: CorpusText[] = [
    {
      kind: 'extraction',
      text:
        'La guerra del Paraguay cambió la economía. La guerra del Paraguay terminó en 1870.\n' +
        'La economía de la guerra fue dura.',
    },
  ]
  const stopwords = new Set(['la', 'del', 'en', 'de', 'fue'])

  it('counts n-grams that start and end on content words, within sentences', () => {
    expect(buildNgrams(corpus, 2, { stopwords, minCount: 1 })).toEqual([
      { gram: 'paraguay cambió', count: 1 },
      { gram: 'paraguay terminó', count: 1 },
    ])
    expect(buildNgrams(corpus, 3, { stopwords, minCount: 1 })).toEqual([
      { gram: 'guerra del paraguay', count: 2 },
      { gram: 'cambió la economía', count: 1 },
      { gram: 'guerra fue dura', count: 1 },
    ])
    expect(buildNgrams(corpus, 3, { stopwords })).toEqual([
      { gram: 'guerra del paraguay', count: 2 },
    ])
    expect(buildNgrams(corpus, 4, { stopwords, minCount: 1 })).toEqual([
      { gram: 'economía de la guerra', count: 1 },
      { gram: 'guerra del paraguay cambió', count: 1 },
      { gram: 'guerra del paraguay terminó', count: 1 },
      { gram: 'paraguay cambió la economía', count: 1 },
    ])
    expect(() => buildNgrams(corpus, 1)).toThrow(RangeError)
    expect(() => buildNgrams(corpus, 6)).toThrow(RangeError)
  })

  it('ranks collocates by log-likelihood G² and keeps only positive associations', () => {
    const collocates = buildCollocates(corpus, 'Guerra', { stopwords, window: 2, minCount: 1 })
    expect(collocates.map((c) => c.word)).toEqual(['paraguay', 'cambió', 'dura', 'terminó'])
    expect(collocates[0]).toMatchObject({ word: 'paraguay', count: 2, frequency: 2 })
    expect(collocates[0]!.g2).toBeCloseTo(2.369878, 5)
    expect(collocates[1]!.g2).toBeCloseTo(1.094925, 5)
    // "economía" co-occurs less than chance (1 < 1.2): not a collocate.
    expect(collocates.find((c) => c.word === 'economía')).toBeUndefined()
    expect(buildCollocates(corpus, 'inexistente', { stopwords })).toEqual([])
  })

  it('keeps G² finite when overlapping windows overcount a neighbour', () => {
    // "guerra" falls 6 times inside the windows of the 2 "paz" (slots), while
    // occurring only 3 times in total: o21 would go negative without clamping.
    const dense: CorpusText[] = [{ kind: 'extraction', text: 'guerra paz guerra paz guerra' }]
    const collocates = buildCollocates(dense, 'paz', { window: 5, minCount: 1 })
    const guerra = collocates.find((c) => c.word === 'guerra')
    expect(guerra).toBeDefined()
    expect(Number.isFinite(guerra!.g2)).toBe(true)
    expect(guerra!.g2).toBeGreaterThanOrEqual(0)
  })

  it('builds a word tree of what follows or precedes a word', () => {
    expect(buildWordTree(corpus, 'guerra', { depth: 2 })).toEqual({
      word: 'guerra',
      count: 3,
      children: [
        { word: 'del', count: 2, children: [{ word: 'paraguay', count: 2, children: [] }] },
        { word: 'fue', count: 1, children: [{ word: 'dura', count: 1, children: [] }] },
      ],
    })
    expect(buildWordTree(corpus, 'economía', { depth: 1, side: 'left' })).toEqual({
      word: 'economía',
      count: 2,
      children: [{ word: 'la', count: 2, children: [] }],
    })
  })
})
