import { beforeEach, expect, it, vi } from 'vitest'

const invoke = vi.hoisted(() => vi.fn())
vi.mock('@tauri-apps/api/core', () => ({ invoke }))

import { buildAnalysisCorpus } from './analysis-corpus'

beforeEach(() => invoke.mockReset())

const row = (assetId: string, textContent: string, createdAt = 1) => ({
  assetId,
  textContent,
  createdAt,
})

it('analyses OCR text without its layout markup', async () => {
  invoke.mockResolvedValueOnce(['Huelga general'])

  const corpus = await buildAnalysisCorpus(
    [row('a', '<div align="center">Huelga general</div>\n![](page=0,bbox=[1, 2, 3, 4])')],
    [row('t', 'Entrevistador: ¿y la huelga?')]
  )

  expect(invoke).toHaveBeenCalledWith('ocr_plain_texts', {
    texts: ['<div align="center">Huelga general</div>\n![](page=0,bbox=[1, 2, 3, 4])'],
  })
  expect(corpus).toEqual([
    { text: 'Huelga general', kind: 'extraction' },
    { text: 'Entrevistador: ¿y la huelga?', kind: 'transcription' },
  ])
})

it('keeps only the newest text of each asset', async () => {
  invoke.mockImplementationOnce(async (_command: string, args: { texts: string[] }) => args.texts)

  const corpus = await buildAnalysisCorpus([row('a', 'vieja', 1), row('a', 'nueva', 2)], [])

  expect(corpus).toEqual([{ text: 'nueva', kind: 'extraction' }])
})

it('analyses the stored text when the conversion fails', async () => {
  invoke.mockRejectedValueOnce('task failed')
  vi.spyOn(console, 'error').mockImplementation(() => {})

  const corpus = await buildAnalysisCorpus([row('a', '<div>Paro</div>')], [])

  expect(corpus).toEqual([{ text: '<div>Paro</div>', kind: 'extraction' }])
})

it('asks the backend nothing without OCR text', async () => {
  await buildAnalysisCorpus([], [row('t', 'Hola')])
  expect(invoke).not.toHaveBeenCalled()
})
