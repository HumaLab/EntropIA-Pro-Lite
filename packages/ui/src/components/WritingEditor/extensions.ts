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

/** Words from which a quotation is long enough to be set off as a block. */
const BLOCK_QUOTE_WORDS = 40

/**
 * Whether a quote is set off as a block: it keeps line breaks from the page, or
 * it runs to forty words or more — the usual threshold for a block quotation.
 */
function isLongQuote(quoted: string): boolean {
  if (quoted.includes('\n')) return true
  return quoted.split(/\s+/).filter(Boolean).length >= BLOCK_QUOTE_WORDS
}
import { SearchHighlight } from './search-highlight'
import { UniqueCitationIds } from './unique-citation-ids'
import { FontSize, WritingTextStyle } from './font-size'
import { TextColor } from './text-color'
import { WritingHighlight } from './highlight'
import { WritingSubscript, WritingSuperscript } from './script-marks'
import { TextCaseCommands } from './text-case'
import { ClearFormatting } from './clear-formatting'
import { ParagraphFormat, WritingTextAlign } from './paragraph-format'

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
      // Kept out of the DOM: it is an object, and Tiptap renders every other
      // attribute into an HTML attribute, where it would land as
      // "[object Object]". It travels in the JSON, which is the canonical form.
      metadataSnapshot: { default: null, rendered: false },
    }
  },

  parseHTML() {
    return [{ tag: 'span[data-document-citation]' }]
  },

  /**
   * An atom with no content renders as an empty span — an invisible citation.
   * So the node draws what it cites: the quoted fragment, with its page when
   * there is one. §10.1 also allows a reference without the transcription, and
   * that is the case with no `quotedText`, which shows a marker instead of
   * nothing at all.
   */
  renderHTML({ node, HTMLAttributes }) {
    const quoted = typeof node.attrs.quotedText === 'string' ? node.attrs.quotedText : ''
    const page = node.attrs.pageNumber
    const suffix = typeof page === 'number' ? ` (p. ${page})` : ''
    const label = quoted ? `«${quoted}»${suffix}` : `[cita${suffix}]`
    // A long quote is set off as a block, as academic prose sets off a long
    // quotation; a short one stays inside the sentence it was written into.
    const block = isLongQuote(quoted) ? { 'data-block-quote': '' } : {}
    return [
      'span',
      mergeAttributes({ 'data-document-citation': '', ...block }, HTMLAttributes),
      label,
    ]
  },
})

/**
 * A live link to a research note (§13, §13.1).
 *
 * An atom, like a corpus citation, and for the same reason: its text belongs to
 * the note, not to the manuscript, and its identity is what makes the link
 * traceable. It carries the snapshot the writer inserted and a hash of it —
 * never a reference to be re-read, because re-reading is the automatic
 * overwrite §13 forbids.
 *
 * Copying a note takes a different path entirely: plain text, no node. There is
 * deliberately no way to turn that text into one of these.
 */
export const NoteLink = Node.create({
  name: 'noteLink',
  group: 'inline',
  inline: true,
  atom: true,

  addAttributes() {
    return {
      noteLinkNodeId: { default: null },
      noteId: { default: null },
      itemId: { default: null },
      contentSnapshot: { default: null },
      contentHash: { default: null },
    }
  },

  parseHTML() {
    return [{ tag: 'span[data-note-link]' }]
  },

  /**
   * Draws the snapshot rather than nothing. A citation taught this lesson
   * already: an atom with an empty `renderHTML` is invisible on the page while
   * sitting intact in the database.
   */
  renderHTML({ node, HTMLAttributes }) {
    const snapshot =
      typeof node.attrs.contentSnapshot === 'string' ? node.attrs.contentSnapshot : ''
    const label = snapshot ? `«${snapshot}»` : '[nota]'
    return ['span', mergeAttributes({ 'data-note-link': '' }, HTMLAttributes), label]
  },
})

/**
 * A bibliographic citation (§9.5, §11.5).
 *
 * The node carries everything the projection needs, because the projection is
 * derived from the manuscript — the same rule the corpus citations follow. What
 * it deliberately does **not** carry is the rendered string: §11.5 requires
 * data equivalent to CSL, and storing the rendering is what would leave old
 * text behind when someone changes citation style.
 *
 * `metadataSnapshot` is the CSL-JSON as it was when the work was cited. It is
 * what §11.3 renders from when Zotero is closed, and what survives the work
 * being deleted from the library.
 */
export const ZoteroCitation = Node.create({
  name: 'zoteroCitation',
  group: 'inline',
  inline: true,
  atom: true,

  addAttributes() {
    return {
      citationNodeId: { default: null },
      /**
       * The works cited together here, in the order they should read.
       *
       * A citation is a *cluster* in CSL, not a work: `(Acha, 2015; Acha,
       * 2008)` is one citation of two works and belongs inside one pair of
       * brackets. Modelling it as one node per work is what produced
       * `(Acha, 2015)(Acha, 2008)` — three separate citations that happened to
       * be adjacent.
       *
       * The projection already expected this: `writing_zotero_citations` is
       * unique on `(document, cluster, position)`, which is a cluster of many
       * rows described from the start.
       *
       * An array, so it stays out of the DOM like the snapshots it holds.
       */
      items: { default: () => [], rendered: false },
      /** Affixes for the whole cluster, not for one of its works. */
      prefix: { default: null },
      suffix: { default: null },
      // A citation used to be one work, with these on the node. They are kept
      // so a manuscript written then still parses and still knows what it
      // cited; `worksOf` reads them as a cluster of one, and the next save
      // writes it in the current shape.
      itemKey: { default: null },
      libraryType: { default: null },
      libraryId: { default: null },
      itemVersion: { default: null },
      locator: { default: null },
      locatorType: { default: null },
      suppressAuthor: { default: false },
      metadataSnapshot: { default: null, rendered: false },
      /** The last rendering, held only so the page is not blank while the
       *  engine is asked again. Never the source of truth. */
      renderedText: { default: null },
    }
  },

  parseHTML() {
    return [{ tag: 'span[data-zotero-citation]' }]
  },

  /**
   * Draws the last rendering, or a marker. An atom with an empty `renderHTML`
   * is invisible on the page while sitting intact in the database — the corpus
   * citation taught that lesson already.
   */
  renderHTML({ node, HTMLAttributes }) {
    const rendered = typeof node.attrs.renderedText === 'string' ? node.attrs.renderedText : ''
    return [
      'span',
      mergeAttributes({ 'data-zotero-citation': '' }, HTMLAttributes),
      rendered || '[cita]',
    ]
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
    // The typography marks. Adding a mark changes what a manuscript may hold
    // but not the schema version: a build without them refuses a document
    // carrying one, safely and without writing (see document-contract.ts).
    WritingSubscript,
    WritingSuperscript,
    WritingTextStyle,
    FontSize,
    TextColor,
    WritingHighlight,
    TextCaseCommands,
    ClearFormatting,
    // Paragraph formatting: attributes on paragraphs and headings, not marks.
    // An older build opens a document carrying them and drops them, so a
    // manuscript saved there loses its alignment, indent and spacing.
    WritingTextAlign,
    ParagraphFormat,
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
    ZoteroCitation,
    NoteLink,
    UniqueCitationIds,
    TrailingParagraph,
    SearchHighlight,
  ]
}
