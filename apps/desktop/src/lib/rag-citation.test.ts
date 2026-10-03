import { describe, expect, it, vi } from 'vitest'
import type { RagSource } from './rag'
import { resolveRagCitation, type RagCitationStore } from './rag-citation'

const TEXT = 'Primero llegó la huelga. Después vinieron los obreros del SOIP a la plaza.'

function makeSource(overrides: Partial<RagSource> = {}): RagSource {
  const snippet = 'vinieron los obreros del SOIP'
  const start = TEXT.indexOf(snippet)
  return {
    index: 1,
    assetId: 'asset-1',
    itemId: 'item-1',
    itemTitle: 'Entrevista',
    collectionId: 'col-1',
    collectionName: 'Historia oral',
    snippet,
    score: 0.9,
    startSeconds: null,
    endSeconds: null,
    provenance: {
      retrievalUnit: 'chunk',
      sourceKind: 'extraction',
      sourceId: 'ext-1',
      chunkIds: ['c1'],
      startChar: start,
      endChar: start + snippet.length,
    },
    ...overrides,
  }
}

function makeStore(
  extraction: { id: string; textContent: string } | null,
  transcription: { id: string; textContent: string } | null = null
): RagCitationStore {
  return {
    extractions: { findByAsset: vi.fn(async () => extraction) },
    transcriptions: { findByAsset: vi.fn(async () => transcription) },
  }
}

describe('resolveRagCitation', () => {
  it('returns the exact provenance range when the text still matches', async () => {
    const source = makeSource()
    const result = await resolveRagCitation(makeStore({ id: 'ext-1', textContent: TEXT }), source)

    expect(result.citationRange).toEqual({
      start: source.provenance!.startChar,
      end: source.provenance!.endChar,
      text: source.snippet,
    })
    expect(result.citationSeconds).toBeNull()
  })

  it('locates the snippet when the text was re-extracted under another id', async () => {
    const shifted = `Encabezado nuevo. ${TEXT}`
    const result = await resolveRagCitation(
      makeStore({ id: 'ext-2', textContent: shifted }),
      makeSource()
    )

    const start = shifted.indexOf('vinieron los obreros del SOIP')
    expect(result.citationRange).toEqual({
      start,
      end: start + 'vinieron los obreros del SOIP'.length,
      text: 'vinieron los obreros del SOIP',
    })
  })

  it('locates the snippet when the stored range now names different words', async () => {
    const edited = TEXT.replace('Primero llegó', 'Antes de todo llegó')
    const result = await resolveRagCitation(
      makeStore({ id: 'ext-1', textContent: edited }),
      makeSource()
    )

    const start = edited.indexOf('vinieron los obreros del SOIP')
    expect(result.citationRange?.start).toBe(start)
    expect(edited.slice(result.citationRange!.start, result.citationRange!.end)).toBe(
      'vinieron los obreros del SOIP'
    )
  })

  it('locates a snippet whose whitespace differs from the current text', async () => {
    const reflowed = 'Primero llegó la huelga.\nDespués vinieron  los\nobreros del SOIP a la plaza.'
    const result = await resolveRagCitation(
      makeStore({ id: 'ext-9', textContent: reflowed }),
      makeSource()
    )

    expect(result.citationRange).not.toBeNull()
    expect(reflowed.slice(result.citationRange!.start, result.citationRange!.end)).toBe(
      'vinieron  los\nobreros del SOIP'
    )
  })

  it('falls back to locating the snippet when provenance is missing (old conversations)', async () => {
    const result = await resolveRagCitation(
      makeStore({ id: 'ext-1', textContent: TEXT }),
      makeSource({ provenance: null })
    )

    const start = TEXT.indexOf('vinieron los obreros del SOIP')
    expect(result.citationRange).toEqual({
      start,
      end: start + 'vinieron los obreros del SOIP'.length,
      text: 'vinieron los obreros del SOIP',
    })
  })

  it('returns no range when the snippet is not in the text any more', async () => {
    const result = await resolveRagCitation(
      makeStore({ id: 'ext-1', textContent: 'Otro texto completamente distinto.' }),
      makeSource()
    )

    expect(result.citationRange).toBeNull()
  })

  it('returns no range when the snippet is ambiguous and the range cannot vouch for it', async () => {
    const twice = 'vinieron los obreros del SOIP. Y luego vinieron los obreros del SOIP.'
    const result = await resolveRagCitation(
      makeStore({ id: 'ext-2', textContent: twice }),
      makeSource({ provenance: null })
    )

    expect(result.citationRange).toBeNull()
  })

  it('returns no range when the asset has no text', async () => {
    const result = await resolveRagCitation(makeStore(null), makeSource())
    expect(result.citationRange).toBeNull()
  })

  it('never throws when the store fails', async () => {
    const store: RagCitationStore = {
      extractions: {
        findByAsset: vi.fn(async () => {
          throw new Error('db closed')
        }),
      },
      transcriptions: { findByAsset: vi.fn(async () => null) },
    }

    await expect(resolveRagCitation(store, makeSource({ startSeconds: 3 }))).resolves.toEqual({
      citationRange: null,
      citationSeconds: 3,
    })
  })

  it('converts Rust char offsets to UTF-16 offsets around astral characters', async () => {
    const text = '😀😀 vinieron los obreros del SOIP'
    const snippet = 'vinieron los obreros del SOIP'
    // Rust counts chars: two emoji + one space = 3 chars before the snippet.
    const source = makeSource({
      snippet,
      provenance: {
        retrievalUnit: 'chunk',
        sourceKind: 'extraction',
        sourceId: 'ext-1',
        chunkIds: [],
        startChar: 3,
        endChar: 3 + snippet.length,
      },
    })
    const result = await resolveRagCitation(makeStore({ id: 'ext-1', textContent: text }), source)

    expect(text.slice(result.citationRange!.start, result.citationRange!.end)).toBe(snippet)
  })

  it('reads a transcription source and carries the start second', async () => {
    const spoken = 'buenas tardes, la huelga empezó en junio'
    const snippet = 'la huelga empezó en junio'
    const start = spoken.indexOf(snippet)
    const source = makeSource({
      snippet,
      startSeconds: 65,
      endSeconds: 80,
      provenance: {
        retrievalUnit: 'chunk',
        sourceKind: 'transcription',
        sourceId: 'tr-1',
        chunkIds: [],
        startChar: start,
        endChar: start + snippet.length,
      },
    })
    const result = await resolveRagCitation(
      makeStore(null, { id: 'tr-1', textContent: spoken }),
      source
    )

    expect(result.citationRange).toEqual({ start, end: start + snippet.length, text: snippet })
    expect(result.citationSeconds).toBe(65)
  })

  it('still seeks when the transcript text cannot be matched', async () => {
    const result = await resolveRagCitation(
      makeStore(null, { id: 'tr-1', textContent: 'otra cosa' }),
      makeSource({
        startSeconds: 12.5,
        provenance: {
          retrievalUnit: 'chunk',
          sourceKind: 'transcription',
          sourceId: 'tr-1',
          chunkIds: [],
          startChar: 0,
          endChar: 5,
        },
      })
    )

    expect(result).toEqual({ citationRange: null, citationSeconds: 12.5 })
  })
})
