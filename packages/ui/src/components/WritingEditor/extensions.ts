import { Node, mergeAttributes } from '@tiptap/core'
import { Plugin, PluginKey } from '@tiptap/pm/state'
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

import { isLongQuote } from './citations'
import { SearchHighlight } from './search-highlight'
import { UniqueCitationIds } from './unique-citation-ids'
import { FontSize, WritingTextStyle } from './font-size'
import { TextColor } from './text-color'
import { WritingHighlight } from './highlight'
import { WritingSubscript, WritingSuperscript } from './script-marks'
import { TextCaseCommands } from './text-case'
import { ClearFormatting } from './clear-formatting'
import { ParagraphFormat, WritingTextAlign } from './paragraph-format'
import { clampWritingImageWidth } from './writing-image-resize'
import {
  isSelectionInsideWritingImageNode,
  shouldIgnoreWritingImageMutation,
  shouldStopWritingImageEvent,
} from './writing-image-node-view'
import { DEFAULT_WRITING_IMAGE_LABELS, type WritingImageLabels } from './writing-image-labels'

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
export const DocumentCitation = Node.create<{
  resolveImage: ((source: string) => string) | null
}>({
  name: 'documentCitation',
  group: 'inline',
  inline: true,
  atom: true,

  addOptions() {
    // How a stored crop becomes something the webview can show. The app knows
    // where the archive is; this package does not, and a citation with no
    // resolver simply shows its words.
    return { resolveImage: null }
  },

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
      // Text and images in the order they were quoted, when the quote took in a
      // region the OCR marked as an image (rendered-selection.ts). An array, so
      // it is kept out of the DOM like the snapshot above and travels in the
      // JSON, which is the canonical form.
      quotedParts: { default: null, rendered: false },
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
   *
   * When the quote took in an image, the parts are drawn in the order they were
   * read, so the picture stands where it stood on the page. Without a resolver
   * the words are drawn alone: a broken image would say less than nothing.
   */
  renderHTML({ node, HTMLAttributes }) {
    const quoted = typeof node.attrs.quotedText === 'string' ? node.attrs.quotedText : ''
    const page = node.attrs.pageNumber
    const suffix = typeof page === 'number' ? ` (p. ${page})` : ''
    // A long quote is set off as a block, as academic prose sets off a long
    // quotation; a short one stays inside the sentence it was written into.
    const block = isLongQuote(quoted) ? { 'data-block-quote': '' } : {}
    const attributes = mergeAttributes({ 'data-document-citation': '', ...block }, HTMLAttributes)

    const drawn = drawParts(node.attrs.quotedParts, this.options.resolveImage)
    if (drawn.length > 0) return ['span', attributes, '«', ...drawn, `»${suffix}`]

    return ['span', attributes, quoted ? `«${quoted}»${suffix}` : `[cita${suffix}]`]
  },
})

/** What a quote holds, drawn in reading order; empty when it is only words. */
function drawParts(
  value: unknown,
  resolveImage: ((source: string) => string) | null
): (string | [string, Record<string, string>])[] {
  if (!Array.isArray(value)) return []
  const drawn: (string | [string, Record<string, string>])[] = []
  for (const part of value) {
    if (!part || typeof part !== 'object') continue
    const kind = (part as { kind?: unknown }).kind
    if (kind === 'text' && typeof (part as { text?: unknown }).text === 'string') {
      drawn.push((part as { text: string }).text)
      continue
    }
    if (kind !== 'image' || typeof (part as { source?: unknown }).source !== 'string') continue
    if (!resolveImage) continue
    drawn.push(['img', { src: resolveImage((part as { source: string }).source), alt: '' }])
  }
  return drawn
}

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

export interface WritingImageAttrs {
  src: string
  alt: string | null
  title: string | null
  width: number | null
  height: number | null
  align: 'left' | 'center' | 'right'
}

declare module '@tiptap/core' {
  interface Commands<ReturnType> {
    writingImage: {
      /** Inserts a manuscript image, leaving any existing text selection
       *  untouched rather than replacing it. Inserts at the cursor
       *  (`state.selection.to`) unless `at` names an explicit position — the
       *  one thing a drop needs and a toolbar insert or a paste never do: a
       *  drop lands where it was dropped, not wherever the caret happens to
       *  be. One command for all three entry paths (spec, Entry Paths) is
       *  what this parameter buys: paste, drop and the toolbar all construct
       *  the node exactly the same way, with the same attribute defaults. */
      insertWritingImage: (
        attrs: { src: string } & Partial<Omit<WritingImageAttrs, 'src'>>,
        options?: { at?: number }
      ) => ReturnType
    }
  }
}

/**
 * A manuscript image (writing-image-node-design.md). Content-bearing, not an
 * atom: `content: 'inline*'` is the caption, kept editable like any other
 * prose rather than an attribute, so it is searched, counted and undone
 * character by character. `src` is always a path relative to the shared data
 * directory (writing-images.ts), never an absolute one.
 */
export const WritingImage = Node.create<{
  resolveImage: ((source: string) => string) | null
  importImage:
    | ((bytes: Uint8Array) => Promise<{ path: string; width: number | null; height: number | null } | null>)
    | null
  labels: WritingImageLabels | null
}>({
  name: 'writingImage',
  group: 'block',
  content: 'inline*',
  draggable: true,
  selectable: true,

  addOptions() {
    return { resolveImage: null, importImage: null, labels: null }
  },

  addAttributes() {
    return {
      src: { default: null },
      alt: { default: null },
      title: { default: null },
      width: { default: null },
      height: { default: null },
      align: { default: 'center' },
    }
  },

  // Matches only what this node itself emits, so a copy within or between
  // manuscripts round-trips exactly. Deliberately not `img[src]`: HTML from
  // elsewhere carries remote or foreign-filesystem sources this archive does
  // not hold, and the paste plugin (Task 7) — not this rule — is what turns
  // those into a managed copy or drops them.
  parseHTML() {
    return [{ tag: 'figure[data-writing-image]' }]
  },

  renderHTML({ node, HTMLAttributes }) {
    const attributes = mergeAttributes(
      { 'data-writing-image': '', 'data-align': node.attrs.align },
      HTMLAttributes
    )
    const src = typeof node.attrs.src === 'string' ? node.attrs.src : ''
    const imgAttrs: Record<string, unknown> = {
      src: this.options.resolveImage ? this.options.resolveImage(src) : src,
      alt: node.attrs.alt ?? '',
      title: node.attrs.title ?? '',
    }
    if (typeof node.attrs.width === 'number') imgAttrs.width = node.attrs.width
    return ['figure', attributes, ['img', imgAttrs], ['figcaption', 0]]
  },

  // The first node view in this repository (writing-image-node-design.md,
  // Node View and Resizing). Plain ProseMirror DOM, not Svelte — the schema's
  // other nodes render through renderHTML alone and stay that way; only this
  // node needs live interaction (the resize handle, the alignment buttons).
  // `dom` is the figure; `contentDOM` is the figcaption — the caption text is
  // the node's own editable content. Everything else inside `dom` (the img,
  // the chrome) is marked contentEditable="false" so the caret and
  // click-to-select never land on non-editable chrome instead of the caption.
  addNodeView() {
    return ({ node, getPos, editor }) => {
      const labels = this.options.labels ?? DEFAULT_WRITING_IMAGE_LABELS

      const figure = document.createElement('figure')
      figure.dataset.writingImage = ''
      figure.dataset.align = node.attrs.align ?? 'center'

      const attrPos = () => (typeof getPos === 'function' ? getPos() : null)
      // Defect 1: this node is `content: 'inline*'`, not an atom, so
      // ProseMirror's own default click handling never produces a
      // NodeSelection for it (`selectClickedLeaf`, prosemirror-view
      // dist/index.js:3224-3233, requires `node.isAtom`) — a click instead
      // resolves to a document position and drops the caret into the
      // caption. Wired onto the image and its broken-image placeholder
      // below; never onto the caption itself, which must keep taking a
      // real caret.
      const selectFigure = () => {
        const pos = attrPos()
        if (pos === null || pos === undefined) return
        editor.chain().focus().setNodeSelection(pos).run()
      }

      const img = document.createElement('img')
      img.contentEditable = 'false'
      img.draggable = false
      img.alt = node.attrs.alt ?? ''
      img.title = node.attrs.title ?? ''
      if (typeof node.attrs.width === 'number') img.width = node.attrs.width
      const src = typeof node.attrs.src === 'string' ? node.attrs.src : ''
      img.src = this.options.resolveImage ? this.options.resolveImage(src) : src
      img.addEventListener('click', (event) => {
        event.preventDefault()
        selectFigure()
      })

      // A stored file missing at render time (spec, Failure Handling): the
      // node is kept — the manuscript still records that an image belongs
      // here — but the browser's own broken-image glyph is replaced with a
      // placeholder that says so. `onload` clears it if a later attribute
      // change (a different resolved src) succeeds.
      const placeholder = document.createElement('div')
      placeholder.className = 'writing-editor__image-placeholder'
      placeholder.contentEditable = 'false'
      placeholder.setAttribute('role', 'img')
      placeholder.textContent = labels.missingImage
      placeholder.addEventListener('click', (event) => {
        event.preventDefault()
        selectFigure()
      })
      img.addEventListener('error', () => {
        figure.dataset.broken = ''
      })
      img.addEventListener('load', () => {
        delete figure.dataset.broken
      })

      // The image's own shrink-wrapped frame (defect 3): `dom` (the figure)
      // is a block spanning the whole column, so a handle positioned
      // against *it* lands at the column's own corner, not the image's —
      // exactly the far-off handle the user saw. `frame` wraps only the
      // image (and its broken-file placeholder, and the resize handle) and
      // shrink-wraps to whichever of those is visible, so `position:
      // relative` on it (WritingEditor.svelte) gives the handle the
      // image's own corner as its positioning context, for all three
      // alignments. The toolbar and the caption stay outside it — the
      // toolbar because it is free to be its own width, and the caption
      // because it must never affect how wide this frame shrinks to (a
      // caption line is often the widest content in the figure once
      // defect 4 gives it something to show).
      const frame = document.createElement('div')
      frame.contentEditable = 'false'
      frame.draggable = false
      frame.className = 'writing-editor__image-frame'

      // Everything outside contentDOM (the figcaption below) must refuse the
      // caret, or click-to-select on the image becomes unreliable.
      const chrome = document.createElement('div')
      chrome.contentEditable = 'false'
      chrome.draggable = false
      chrome.className = 'writing-editor__image-chrome'

      const toolbar = document.createElement('div')
      toolbar.className = 'writing-editor__image-toolbar'

      const alignGroup = document.createElement('div')
      alignGroup.draggable = false
      alignGroup.className = 'writing-editor__image-align'
      alignGroup.setAttribute('role', 'group')
      const ALIGN_LABELS = {
        left: labels.alignLeft,
        center: labels.alignCenter,
        right: labels.alignRight,
      } as const
      const alignButtons = (['left', 'center', 'right'] as const).map((align) => {
        const button = document.createElement('button')
        button.type = 'button'
        button.draggable = false
        button.textContent = ALIGN_LABELS[align]
        button.setAttribute('aria-pressed', String((node.attrs.align ?? 'center') === align))
        button.addEventListener('click', () => {
          const pos = attrPos()
          if (pos === null || pos === undefined) return
          editor.view.dispatch(editor.state.tr.setNodeAttribute(pos, 'align', align))
        })
        alignGroup.appendChild(button)
        return { align, button }
      })

      // Alt text, edited inline rather than through `window.prompt` (I3):
      // WebKitGTK — wry's Linux backend — implements no native prompt
      // dialog at all, so a prompt-based control is a silent no-op there.
      // A plain input works on every platform and needs no dialog.
      //
      // `title` used to have a second field right here. Defect 4: that
      // field wrote `attrs.title`, which renders only as `img.title` — an
      // invisible HTML tooltip — while the *real* caption (the figcaption
      // below, the node's own document content) had no affordance at all.
      // The field is gone; the attribute stays, still written to `img.title`
      // below and in `update()`, for serialization and export.
      const fields = document.createElement('div')
      fields.className = 'writing-editor__image-fields'

      const altInput = document.createElement('input')
      altInput.type = 'text'
      altInput.className = 'writing-editor__image-field'
      altInput.placeholder = labels.altLabel
      altInput.setAttribute('aria-label', labels.altLabel)
      altInput.value = node.attrs.alt ?? ''
      altInput.addEventListener('input', () => {
        const pos = attrPos()
        if (pos === null || pos === undefined) return
        editor.view.dispatch(editor.state.tr.setNodeAttribute(pos, 'alt', altInput.value))
      })

      fields.append(altInput)
      toolbar.append(alignGroup, fields)

      const handle = document.createElement('button')
      handle.type = 'button'
      handle.draggable = false
      handle.className = 'writing-editor__image-handle'
      handle.setAttribute('aria-label', labels.resizeHandle)

      let dragStartX = 0
      let dragStartWidth = typeof node.attrs.width === 'number' ? node.attrs.width : img.naturalWidth

      function currentAspect(): number {
        const width = typeof node.attrs.width === 'number' ? node.attrs.width : img.naturalWidth || 1
        const height = typeof node.attrs.height === 'number' ? node.attrs.height : img.naturalHeight || 1
        return width / (height || 1)
      }

      // Defect 2: prosemirror-view marks `nodeDOM` (the figure) a native
      // HTML5 drag source (`draggable = true`) whenever this node is
      // selected, because it has a `contentDOM`
      // (dist/index.js:1490-1493) — regardless of `img.draggable` /
      // `handle.draggable`, both already `false` above, which do not stop
      // it: the browser's drag-initiation walk finds the *nearest
      // draggable ancestor* of the pointerdown target, skipping past a
      // `draggable="false"` descendant rather than being blocked by it, and
      // that ancestor is the figure. So grabbing the handle while the
      // figure is selected — the only time the handle is even visible —
      // starts a native whole-figure drag at the same time as this
      // pointer-based resize, and the two fight: pointermove stops firing,
      // the cursor goes to "not allowed", and the resize dies mid-gesture.
      //
      // The fix is not `figure.draggable = false`: prosemirror-view resets
      // that on every `selectNode`. Instead, a `dragstart` listener on the
      // figure (below) cancels the browser's drag outright, but only while
      // a resize gesture is actually in progress — grabbing the figure
      // anywhere else still starts its own legitimate drag, to reposition
      // it in the document.
      let resizeGestureActive = false
      let dragPointerId = 0

      // A gesture that ends in pointercancel (the OS takes over a touch
      // gesture, lost pointer capture) must tear down exactly like a normal
      // pointerup, minus committing a resize — otherwise these window
      // listeners outlive the gesture and the next unrelated pointerup
      // anywhere in the document resizes the image from stale
      // dragStartX/dragStartWidth.
      const stopDragTracking = () => {
        resizeGestureActive = false
        try {
          handle.releasePointerCapture(dragPointerId)
        } catch {
          // Pointer capture was never acquired (unsupported environment,
          // or already released/lost) — nothing to release.
        }
        window.removeEventListener('pointermove', onPointerMove)
        window.removeEventListener('pointerup', onPointerUp)
        window.removeEventListener('pointercancel', onPointerCancel)
      }
      const onPointerMove = (event: PointerEvent) => {
        const available = figure.parentElement?.clientWidth ?? dragStartWidth
        const clamped = clampWritingImageWidth(
          dragStartWidth + (event.clientX - dragStartX),
          currentAspect(),
          available
        )
        if (clamped) img.width = clamped.width
      }
      const onPointerUp = (event: PointerEvent) => {
        stopDragTracking()
        const pos = attrPos()
        if (pos === null || pos === undefined) return
        const available = figure.parentElement?.clientWidth ?? dragStartWidth
        const clamped = clampWritingImageWidth(
          dragStartWidth + (event.clientX - dragStartX),
          currentAspect(),
          available
        )
        if (!clamped) return
        editor.view.dispatch(
          editor.state.tr
            .setNodeAttribute(pos, 'width', clamped.width)
            .setNodeAttribute(pos, 'height', clamped.height)
        )
      }
      // Cancellation aborts the gesture: tear down the same listeners
      // onPointerUp would, but never dispatch a resize from it.
      const onPointerCancel = () => {
        stopDragTracking()
      }
      handle.addEventListener('pointerdown', (event) => {
        // getPos alone is not "the figure is selected" — dragStartWidth must
        // still come from the *current* node, which is why C1's fix
        // (reassigning `node` in update() below) matters here specifically.
        dragStartX = event.clientX
        dragStartWidth = typeof node.attrs.width === 'number' ? node.attrs.width : img.width
        resizeGestureActive = true
        dragPointerId = event.pointerId
        // Bounds the gesture to this handle regardless of where the pointer
        // physically travels — deferred when the resize handle was first
        // built, now paired with the dragstart guard above since both exist
        // to keep this gesture from being hijacked mid-flight. Unsupported
        // in some environments (older WebKitGTK, this package's own
        // happy-dom test environment); the window-level pointermove/
        // pointerup listeners below already track the gesture correctly
        // without it.
        try {
          handle.setPointerCapture(event.pointerId)
        } catch {
          // See above — capture is a defensive extra, not load-bearing.
        }
        window.addEventListener('pointermove', onPointerMove)
        window.addEventListener('pointerup', onPointerUp)
        window.addEventListener('pointercancel', onPointerCancel)
      })

      // Cancels the browser's native drag in two cases, and leaves it alone
      // otherwise — grabbing the figure anywhere else must keep starting its
      // own legitimate drag, to reposition the image in the document.
      //
      // 1. A resize gesture is actually in progress (defect 2, as before).
      //
      // 2. The gesture began inside the caption (defect 5, half 1). This
      // node's schema declares `draggable: true`, so prosemirror-view arms
      // `figure.draggable = true` independently of any selection state —
      // once via its own per-mousedown `mightDrag` bookkeeping on the very
      // first click anywhere in this node's range, and persistently via
      // `selectNode()` once the node is the selected node (nodeDOM for a
      // contentDOM-bearing node view is always the outer `dom`, i.e. this
      // figure, never the contentDOM). Only the resize handle's own gesture
      // was ever exempted from the native drag that produces; a click
      // landing inside the caption — the only way to place a caret in it —
      // was not, so the browser hijacks that click into a native drag before
      // a caret can land, and the caption is unreachable. `dragstart`
      // bubbles, so `event.target` here still names whichever element the
      // gesture actually started on, even though the listener sits on the
      // figure.
      figure.addEventListener('dragstart', (event) => {
        if (resizeGestureActive || shouldStopWritingImageEvent([figcaption], event)) {
          event.preventDefault()
        }
      })

      frame.append(img, placeholder, handle)
      chrome.append(toolbar)

      const figcaption = document.createElement('figcaption')
      figcaption.dataset.placeholder = labels.captionPlaceholder
      const syncCaptionEmpty = (current: typeof node) => {
        if (current.textContent.length === 0) figcaption.dataset.empty = ''
        else delete figcaption.dataset.empty
      }
      syncCaptionEmpty(node)

      // Defect 5, half 2: the empty caption's placeholder used to be shown
      // only under `.ProseMirror-selectednode` (WritingEditor.svelte) — a
      // NodeSelection of the whole figure. Placing a caret in it (now
      // reachable at all thanks to half 1, above) replaces that
      // NodeSelection with a TextSelection and clears the class, so the
      // placeholder would vanish the instant a caret lands. This tracks a
      // second, independent condition — the current selection sits inside
      // this node's own document range — and reflects it as
      // `data-caret-inside` on the figure, which WritingEditor.svelte's CSS
      // also shows the placeholder for. `editor.on('selectionUpdate', ...)`
      // is the only way to observe *this* node's relationship to the current
      // selection: `update()` alone only fires on a change to *this node*,
      // not on every selection change (e.g. moving the caret elsewhere in
      // the document must clear this attribute too, and does not touch this
      // node at all).
      const syncCaretInside = () => {
        const pos = attrPos()
        if (pos === null || pos === undefined) {
          delete figure.dataset.caretInside
          return
        }
        const { from, to } = editor.state.selection
        if (isSelectionInsideWritingImageNode(pos, node.nodeSize, from, to)) {
          figure.dataset.caretInside = ''
        } else {
          delete figure.dataset.caretInside
        }
      }
      syncCaretInside()
      editor.on('selectionUpdate', syncCaretInside)

      figure.append(frame, chrome, figcaption)

      return {
        dom: figure,
        contentDOM: figcaption,
        ignoreMutation: (mutation) => shouldIgnoreWritingImageMutation(figcaption, mutation),
        stopEvent: (event) => shouldStopWritingImageEvent([chrome, handle], event),
        destroy: () => {
          editor.off('selectionUpdate', syncCaretInside)
        },
        update: (updated) => {
          if (updated.type.name !== 'writingImage') return false
          // C1: every closure above reads `node`, not just this function's
          // own `updated` parameter — dragStartWidth, currentAspect() and the
          // alt prefill on a second edit all go stale without this
          // reassignment, because they run *after* this update() returns,
          // from a later event, with whatever `node` last pointed at.
          node = updated
          figure.dataset.align = updated.attrs.align ?? 'center'
          img.alt = updated.attrs.alt ?? ''
          img.title = updated.attrs.title ?? ''
          if (typeof updated.attrs.width === 'number') img.width = updated.attrs.width
          else img.removeAttribute('width')
          if (document.activeElement !== altInput) altInput.value = updated.attrs.alt ?? ''
          const align = updated.attrs.align ?? 'center'
          alignButtons.forEach(({ align: candidate, button }) =>
            button.setAttribute('aria-pressed', String(candidate === align))
          )
          const nextSrc = typeof updated.attrs.src === 'string' ? updated.attrs.src : ''
          const nextResolved = this.options.resolveImage ? this.options.resolveImage(nextSrc) : nextSrc
          if (img.src !== nextResolved) img.src = nextResolved
          syncCaptionEmpty(updated)
          syncCaretInside()
          return true
        },
      }
    }
  },

  addCommands() {
    return {
      // Inserts at a collapsed position, never a range: `commands.insertContent`
      // replaces whatever is currently selected (`tr.selection.from` to
      // `.to`), so an existing text selection would be deleted along with the
      // image landing in its place. `selection.to` — not `.from` — so the
      // image lands after the selected words, continuing the manuscript
      // rather than interrupting it.
      insertWritingImage:
        (attrs: { src: string } & Partial<Omit<WritingImageAttrs, 'src'>>, options) =>
        ({ commands, state }) =>
          commands.insertContentAt(options?.at ?? state.selection.to, {
            type: this.name,
            attrs: {
              alt: null,
              title: null,
              width: null,
              height: null,
              align: 'center',
              ...attrs,
            },
          }),
    }
  },

  addKeyboardShortcuts() {
    return {
      // content: 'inline*' makes this a textblock, so ProseMirror's default
      // splitBlock would create a second writingImage with no src — a node
      // pointing at no stored bytes. Enter exits the figure instead.
      Enter: () => {
        if (!this.editor.isActive(this.name)) return false
        const { $from } = this.editor.state.selection
        const after = $from.after()
        return this.editor
          .chain()
          .insertContentAt(after, { type: 'paragraph' })
          .setTextSelection(after + 1)
          .run()
      },
      // At the start of the caption, Backspace would otherwise join the
      // figure into the block before it. Select the figure instead.
      Backspace: () => {
        if (!this.editor.isActive(this.name)) return false
        const { $from, empty } = this.editor.state.selection
        if (!empty || $from.parentOffset !== 0) return false
        return this.editor.commands.setNodeSelection($from.before())
      },
    }
  },

  addProseMirrorPlugins() {
    const importImage = this.options.importImage
    const editorRef = this.editor
    const ACCEPTED = new Set(['image/png', 'image/jpeg', 'image/gif'])

    // Constructs the node exactly the way the toolbar does — through
    // insertWritingImage itself, not a second, bespoke
    // `schema.nodes.writingImage.create` + `tr.insert` (I4/spec, Entry
    // Paths: "the same import function and the same insert command"). A drop
    // still lands where it was dropped (`pos`, from `posAtCoords`); a paste
    // has no such position and falls back to the command's own default, the
    // cursor. `width` is deliberately left for the command's own null
    // default: the intrinsic pixel size a file decodes to is not "the
    // author's chosen width" the spec defines (C1) — only `height` is kept,
    // for the aspect-ratio arithmetic a resize needs later.
    async function importAndInsert(file: File, pos: number | null) {
      if (!importImage) return
      const bytes = new Uint8Array(await file.arrayBuffer())
      const imported = await importImage(bytes)
      if (!imported) return
      editorRef.commands.insertWritingImage(
        { src: imported.path, height: imported.height ?? null },
        pos === null ? undefined : { at: pos }
      )
    }

    return [
      new Plugin({
        key: new PluginKey('writingImagePasteDrop'),
        props: {
          // Claims the event only when it actually carries an accepted image
          // file. Every other paste — plain text, or HTML with no
          // accompanying bytes such as a remote <img> — falls through
          // untouched, and the schema's own lack of an img[src] parse rule is
          // what keeps that fallthrough from creating a broken reference.
          handlePaste(_view, event) {
            const files = Array.from(event.clipboardData?.files ?? [])
            const image = files.find((file) => ACCEPTED.has(file.type))
            if (!image) return false
            event.preventDefault()
            void importAndInsert(image, null)
            return true
          },
          handleDrop(view, event) {
            const files = Array.from(event.dataTransfer?.files ?? [])
            const image = files.find((file) => ACCEPTED.has(file.type))
            if (!image) return false
            event.preventDefault()
            const coords = view.posAtCoords({ left: event.clientX, top: event.clientY })
            void importAndInsert(image, coords?.pos ?? null)
            return true
          },
        },
      }),
    ]
  },
})

export interface WritingExtensionOptions {
  placeholder?: string
  /**
   * Turns the path of a quoted image into something the webview can show. The
   * app knows where the archive lives; this package does not.
   */
  resolveImage?: (source: string) => string
  /** Imports raw bytes into managed storage and returns the relative path
   *  and intrinsic size, or null when the bytes are not an accepted format.
   *  Only the paste/drop plugin (Task 7) calls this — the toolbar path
   *  (Task 6) imports through the app layer directly and calls
   *  `insertWritingImage` with an already-resolved path. `width`/`height` are
   *  `number | null` — null, like `pickWritingImage`'s own shape, when the
   *  bytes' intrinsic size could not be decoded (I4: one shape for both entry
   *  paths, instead of one returning null and the other 0 for the same
   *  failure). */
  importImage?: (
    bytes: Uint8Array
  ) => Promise<{ path: string; width: number | null; height: number | null } | null>
  /** Strings for the writingImage node view's own chrome: the alignment
   *  buttons, the alt/title fields, the resize handle's accessible name, and
   *  the missing-file placeholder (I2). Falls back to an English default set
   *  so a caller that mounts the schema with no options — every other test
   *  in this package — still gets a node view with real accessible names. */
  imageLabels?: WritingImageLabels
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
    DocumentCitation.configure({ resolveImage: options.resolveImage ?? null }),
    ZoteroCitation,
    NoteLink,
    WritingImage.configure({
      resolveImage: options.resolveImage ?? null,
      importImage: options.importImage ?? null,
      labels: options.imageLabels ?? null,
    }),
    UniqueCitationIds,
    TrailingParagraph,
    SearchHighlight,
  ]
}
