/**
 * The text of one page (one asset) — what a citation quotes and anchors into.
 *
 * A page is read by OCR or, for audio, by transcription; the search index
 * already holds both. Everything that turns a page into text for citing has to
 * agree on which one it reads: a citation stores character offsets, so the
 * panel that makes it and the check that later follows it must read the same
 * string, or every citation of an audio would be reported as broken.
 *
 * The extraction wins when a page somehow has both, so offsets into an
 * existing citation never start meaning a different text.
 */

interface TextRow {
  textContent: string | null
}

export interface PageTextSource {
  extractions: { findByAsset(assetId: string): Promise<TextRow | null> }
  transcriptions: { findByAsset(assetId: string): Promise<TextRow | null> }
}

function nonEmpty(row: TextRow | null): string | null {
  const text = row?.textContent ?? null
  return text && text.trim() ? text : null
}

export async function readPageText(store: PageTextSource, assetId: string): Promise<string | null> {
  const extracted = nonEmpty(await store.extractions.findByAsset(assetId))
  if (extracted !== null) return extracted
  return nonEmpty(await store.transcriptions.findByAsset(assetId))
}
