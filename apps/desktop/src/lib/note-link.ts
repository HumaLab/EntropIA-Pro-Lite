import type { CitationIntegrity } from './citation-target'
import { plainTextOf } from './note-text'
import { hashSourceText } from './source-selection'

/**
 * Linking a note into the manuscript (plan-editor.md §13, §13.1).
 *
 * # Copying and linking are different things
 *
 * §13 asks for the difference to be explicit. Copying a note produces
 * independent text: it is inserted and then it is the manuscript's, with no
 * relationship to the note it came from and no way for a later edit to reach
 * it. Linking keeps a live relationship — and when the note changes, EntropIA
 * *reports the divergence and never overwrites the article*.
 *
 * That last rule is why a link stores a snapshot and a hash rather than a
 * reference to be re-read. The snapshot is what the writer actually put in
 * their manuscript and must keep saying; the hash is what notices that the note
 * no longer agrees with it.
 *
 * # Where a link lives
 *
 * In the node, not in a table. §8.1 is explicit that the canonical JSON carries
 * "identidad de nodos, citas, vínculos y snapshots", and the projection tables
 * are described there as queryable projections rather than a second editable
 * source of truth. Unit 5 asks only that the manuscript report a missing note,
 * which needs no cross-document query — so no table was added for one.
 */

/** What a `noteLink` node carries. */
export interface NoteLinkAnchor {
  noteId: string | null
  /** The note's text as it was when the link was made. */
  contentSnapshot: string | null
  /** A fingerprint of that text, for noticing the note has moved on. */
  contentHash: string | null
}

/**
 * The note as it stands now.
 *
 * `exists: false` means it was deleted. `exists: true` with a null `content`
 * means it could not be read — a different thing, and one that must not be
 * reported as a deletion.
 */
export interface NoteToday {
  exists: boolean
  content: string | null
}

export interface NoteLinkState {
  /** The same four words the citation integrity uses, for the same reasons. */
  integrity: CitationIntegrity
  /** The note's current text, when it differs from the snapshot. */
  current: string | null
}

/**
 * Whether a linked note still says what the manuscript quotes.
 *
 * Reports; never rewrites. §13 is explicit that a diverged note does not
 * overwrite the article, and §13.1 that a deleted note is reported without
 * removing the text or the snapshot. So every answer here leaves the
 * manuscript exactly as it is, and `current` is offered for the writer to read
 * — not applied.
 */
export async function resolveNoteLink(
  anchor: NoteLinkAnchor,
  today: NoteToday
): Promise<NoteLinkState> {
  // Gone is gone. A link that never named a note has nothing to check either.
  if (!anchor.noteId || !today.exists) {
    return { integrity: 'source_missing', current: null }
  }
  // Present but unreadable is NOT deleted. Reporting a deletion because a read
  // failed would tell the writer their note is gone when it is sitting there.
  if (today.content === null) return { integrity: 'unverifiable', current: null }

  // Nothing recorded to compare against. The snapshot stands, and no claim is
  // made about whether the note still agrees with it.
  if (!anchor.contentHash) return { integrity: 'unverifiable', current: null }

  const hash = await hashSourceText(today.content)
  // The environment could not hash. That is not the note having changed.
  if (hash === null) return { integrity: 'unverifiable', current: null }

  if (hash !== anchor.contentHash) {
    return { integrity: 'source_modified', current: today.content }
  }

  return { integrity: 'valid', current: null }
}

/**
 * The attributes a fresh link carries.
 *
 * A copy does not go through here at all — it has no attributes, because it is
 * text. That asymmetry is the feature: there is no accidental path from pasting
 * a note's words to acquiring a live relationship with it.
 */
export async function buildNoteLink(note: {
  id: string
  itemId: string
  content: string
}): Promise<Record<string, unknown>> {
  return {
    noteId: note.id,
    itemId: note.itemId,
    // What the note *says*, because this is drawn in the manuscript. A note is
    // written in a rich text editor, so its stored content is HTML, and this
    // used to put `<p>Esto es una carta…</p>` into the article.
    contentSnapshot: plainTextOf(note.content),
    // The hash stays over the raw content, on purpose. `resolveNoteLink`
    // compares it against a hash of the note as it stands now, and moving
    // either side to the extracted text would report every link already in a
    // manuscript as "the note changed" — a warning about nothing, on every
    // link, which is how a warning stops being read.
    contentHash: await hashSourceText(note.content),
  }
}
