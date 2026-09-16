import { Node, mergeAttributes } from '@tiptap/core'
import StarterKit from '@tiptap/starter-kit'
import Document from '@tiptap/extension-document'
import Link from '@tiptap/extension-link'
import Placeholder from '@tiptap/extension-placeholder'
import Underline from '@tiptap/extension-underline'
import Table from '@tiptap/extension-table'
import TableRow from '@tiptap/extension-table-row'
import TableCell from '@tiptap/extension-table-cell'
import TableHeader from '@tiptap/extension-table-header'
import { Footnote, FootnoteReference, Footnotes } from 'tiptap-footnotes'
import { TrailingParagraph } from './trailing-paragraph'
import { SearchHighlight } from './search-highlight'
import { UniqueCitationIds } from './unique-citation-ids'

/**
 * The academic editor's schema (plan-editor.md §6.2, §8.2).
 *
 * Pinned to the Tiptap 2.26.4 line the repository already carries. Spike S1
 * confirmed tables and `tiptap-footnotes@2.0.4` install alongside it with no
 * peer conflict, so nothing here required moving to Tiptap 3.
 *
 * StarterKit ships only bold, code, italic and strike, so `link` and
 * `underline` are added explicitly — the schema does not grow them by itself.
 *
 * This list is the single definition of what a manuscript may contain. The
 * validator in `document-contract.ts` builds its schema from exactly this, so a
 * document that passes validation is a document this editor can mount.
 */

/**
 * A citation into the EntropIA corpus (§8.2). An atom: its text lives in the
 * source, not in the manuscript, and its identity is what makes the citation
 * traceable back through the projection tables.
 */
export const DocumentCitation = Node.create({
  name: 'documentCitation',
  group: 'inline',
  inline: true,
  atom: true,

  addAttributes() {
    return {
      citationNodeId: { default: null },
      collectionId: { default: null },
      itemId: { default: null },
      assetId: { default: null },
      pageNumber: { default: null },
      startChar: { default: null },
      endChar: { default: null },
      sourceTextHash: { default: null },
      // The quoted text and a minimal metadata snapshot travel in the node
      // itself (10.1). They are what 10.2 step 5 shows when the anchor no
      // longer resolves: without them a citation whose source has moved has
      // nothing left to say.
      quotedText: { default: null },
      metadataSnapshot: { default: null },
    }
  },

  parseHTML() {
    return [{ tag: 'span[data-document-citation]' }]
  },

  renderHTML({ HTMLAttributes }) {
    return ['span', mergeAttributes({ 'data-document-citation': '' }, HTMLAttributes)]
  },
})

export interface WritingExtensionOptions {
  placeholder?: string
}

/**
 * Builds a fresh set of extension instances.
 *
 * A factory rather than a shared constant on purpose: Tiptap extensions carry
 * per-editor state once configured, so handing the same instances to
 * `getSchema()` and to `new Editor()` makes two consumers share one object.
 * It also lets the placeholder be per-editor, which a module constant cannot.
 */
export function createWritingExtensions(options: WritingExtensionOptions = {}) {
  return [
    // StarterKit's Document allows `block+`, which leaves no room for the
    // footnotes container the footnote extension appends at the end. Without
    // this override `addFootnote` runs and silently changes nothing — the
    // command succeeds, the schema refuses the node, and the button looks dead.
    Document.extend({ content: 'block+ footnotes?' }),
    StarterKit.configure({ document: false, heading: { levels: [1, 2, 3, 4] } }),
    // Gapcursor comes from StarterKit, which is what lets the caret sit before
    // or after a table at the edge of the document. Naming it again here is a
    // duplicate registration, not a reinforcement.
    Underline,
    Link.configure({ openOnClick: false, autolink: false }),
    Placeholder.configure({ placeholder: options.placeholder ?? '' }),
    Table.configure({ resizable: false }),
    TableRow,
    TableCell,
    TableHeader,
    Footnotes,
    Footnote,
    FootnoteReference,
    DocumentCitation,
    UniqueCitationIds,
    TrailingParagraph,
    SearchHighlight,
  ]
}
