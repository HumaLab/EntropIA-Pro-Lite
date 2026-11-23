import { invoke } from '@tauri-apps/api/core'
import type { CorpusText } from '$lib/text-analysis'

type TextRow = { assetId: string; textContent: string; createdAt: number }

// Defensive dedupe: upsert guarantees one row per asset, but if older rows
// survive a migration we keep only the newest text per asset.
function dedupeByAsset(rows: TextRow[]): string[] {
  const latest = new Map<string, TextRow>()
  for (const row of rows) {
    const existing = latest.get(row.assetId)
    if (!existing || row.createdAt > existing.createdAt) latest.set(row.assetId, row)
  }
  return [...latest.values()].map((row) => row.textContent)
}

/**
 * OCR text as plain words: GLM's layout markup (`<div align="center">`,
 * `![](page=0,bbox=[…])`) would otherwise count as words and phrases. The
 * conversion is the backend's, the same the search index uses. When it fails
 * the texts are analysed as stored rather than not at all.
 */
async function plainTexts(texts: string[]): Promise<string[]> {
  if (texts.length === 0) return texts
  try {
    return await invoke<string[]>('ocr_plain_texts', { texts })
  } catch (error) {
    console.error('[analysis-corpus] plain text conversion failed:', error)
    return texts
  }
}

/** A collection's texts, ready for frequency, phrase and collocate analysis. */
export async function buildAnalysisCorpus(
  extractionRows: TextRow[],
  transcriptionRows: TextRow[]
): Promise<CorpusText[]> {
  const extractions = await plainTexts(dedupeByAsset(extractionRows))
  return [
    ...extractions.map((text) => ({ text, kind: 'extraction' as const })),
    // Transcriptions carry no OCR markup.
    ...dedupeByAsset(transcriptionRows).map((text) => ({ text, kind: 'transcription' as const })),
  ]
}
