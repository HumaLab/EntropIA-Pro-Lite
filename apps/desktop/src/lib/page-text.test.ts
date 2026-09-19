import { describe, it, expect, vi } from 'vitest'
import { readPageSource, readPageText } from './page-text'

function storeWith(extraction: string | null, transcription: string | null) {
  return {
    extractions: {
      findByAsset: vi.fn(async () => (extraction === null ? null : { textContent: extraction })),
    },
    transcriptions: {
      findByAsset: vi.fn(async () =>
        transcription === null ? null : { textContent: transcription }
      ),
    },
  }
}

describe('readPageText', () => {
  it('reads what the OCR extracted from a page', async () => {
    expect(await readPageText(storeWith('el molino', null), 'as1')).toBe('el molino')
  })

  it('reads the transcription of an audio, which has no extraction', async () => {
    expect(await readPageText(storeWith(null, 'Hablante 1: Crosito'), 'as1')).toBe(
      'Hablante 1: Crosito'
    )
  })

  it('falls through an extraction that holds no text', async () => {
    expect(await readPageText(storeWith('   ', 'la transcripcion'), 'as1')).toBe('la transcripcion')
  })

  it('prefers the extraction when a page has both, so offsets never change meaning', async () => {
    expect(await readPageText(storeWith('ocr', 'audio'), 'as1')).toBe('ocr')
  })

  it('says there is no text when nothing was ever read', async () => {
    expect(await readPageText(storeWith(null, null), 'as1')).toBeNull()
  })
})

describe('readPageSource', () => {
  it('says the text came from OCR, which is rendered', async () => {
    expect(await readPageSource(storeWith('# Acta', null), 'as1')).toEqual({
      text: '# Acta',
      kind: 'extraction',
    })
  })

  it('says the text came from a transcription, which is shown as it is', async () => {
    expect(await readPageSource(storeWith(null, 'Hablante 1: hola'), 'as1')).toEqual({
      text: 'Hablante 1: hola',
      kind: 'transcription',
    })
  })

  it('has nothing to say about a page never read', async () => {
    expect(await readPageSource(storeWith(null, null), 'as1')).toBeNull()
  })
})
