import { hashSourceText } from './source-selection'

/**
 * What can still be honoured when someone clicks a citation (plan-editor.md
 * §10.2).
 *
 * §10.2 asks for five steps and then, in step 5, for a graceful answer when the
 * anchor no longer resolves. This is the decision behind that: given the
 * citation and the source as it stands today, what is safe to open, and what
 * has to be shown from the record instead.
 *
 * The vocabulary is the one the schema already speaks —
 * `writing_document_citations.integrity_status` is constrained to exactly these
 * four values — so the answer computed here is also the value that column
 * wants, rather than a second set of names for the same states.
 *
 * Why a content hash rather than the stored positions: spike S2 measured that a
 * persisted range still *resolves* after the source is edited. It just resolves
 * to different words. Only comparing the content can tell the difference, which
 * is why an unverifiable citation is its own case instead of being treated as
 * good.
 */

export type CitationIntegrity = 'valid' | 'source_modified' | 'source_missing' | 'unverifiable'

export interface CitationAnchor {
  assetId: string | null
  pageNumber: number | null
  startChar: number | null
  endChar: number | null
  quotedText: string | null
  sourceTextHash: string | null
}

/** What the source looks like now. `null` text means the page has none. */
export interface SourceToday {
  assetExists: boolean
  extractedText: string | null
}

export interface CitationTarget {
  integrity: CitationIntegrity
  /** Whether the viewer should be opened at all (§10.2 steps 1–3). */
  canOpen: boolean
  /** Whether the range may be highlighted (§10.2 step 4). */
  canHighlight: boolean
  assetId: string | null
  pageNumber: number | null
  start: number | null
  end: number | null
}

function missing(anchor: CitationAnchor): CitationTarget {
  return {
    integrity: 'source_missing',
    canOpen: false,
    canHighlight: false,
    assetId: anchor.assetId,
    pageNumber: anchor.pageNumber,
    start: null,
    end: null,
  }
}

/**
 * Decides what clicking this citation can do.
 *
 * A source that is gone is not an error and never removes the citation: §10.3
 * is explicit that a citation outlives its source, which is why the corpus ids
 * are snapshots without a foreign key. The quoted text and the metadata
 * snapshot on the node are what step 5 has left to show.
 */
export async function resolveCitationTarget(
  anchor: CitationAnchor,
  today: SourceToday
): Promise<CitationTarget> {
  if (!anchor.assetId || !today.assetExists) return missing(anchor)

  const open = {
    canOpen: true,
    assetId: anchor.assetId,
    pageNumber: anchor.pageNumber,
  }
  const withoutRange = { ...open, canHighlight: false, start: null, end: null }

  const { startChar, endChar } = anchor
  const text = today.extractedText
  // The asset is there but has no text yet, or the citation never recorded a
  // range. Either way there is a page to open and nothing to highlight on it.
  if (text === null || startChar === null || endChar === null) {
    return { ...withoutRange, integrity: 'unverifiable' }
  }
  // The range falls outside the text the page holds now, so it certainly is
  // not the fragment that was quoted.
  if (startChar < 0 || endChar > text.length || endChar <= startChar) {
    return { ...withoutRange, integrity: 'source_modified' }
  }

  const range = { canHighlight: true, start: startChar, end: endChar }

  // Nothing recorded to compare against. The range is opened and highlighted
  // because it is all the citation has, but it is not claimed to be verified.
  if (!anchor.sourceTextHash) return { ...open, ...range, integrity: 'unverifiable' }

  const hash = await hashSourceText(text.slice(startChar, endChar))
  // The environment could not hash. That is not the source having changed, and
  // saying so would be a claim this build cannot make.
  if (hash === null) return { ...open, ...range, integrity: 'unverifiable' }

  if (hash !== anchor.sourceTextHash) {
    // The words at that range are not the ones that were cited. Opening the
    // page is still useful; highlighting would point at the wrong sentence and
    // assert it was the quoted one.
    return { ...withoutRange, integrity: 'source_modified' }
  }

  return { ...open, ...range, integrity: 'valid' }
}
