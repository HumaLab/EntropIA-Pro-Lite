/**
 * Predicate guards for the writingImage node view's ProseMirror lifecycle
 * (Task 5 fix round: one Critical, two Important, all in the node view's
 * lifecycle — absent from the original reference implementation because this
 * is the first `contentDOM`-bearing node view in the repository).
 *
 * Extracted as pure functions, not inlined into the view closure, so each is
 * directly reachable and assertable with plain DOM nodes — no editor, no
 * mounted node view, no dragging required.
 */

/**
 * `ignoreMutation` (Critical). ProseMirror's own default is
 * `!this.contentDOM && mutation.type != "selection"` (prosemirror-view
 * 1.41.8, dist/index.js:1107-1108). Because this view sets `contentDOM`,
 * `!this.contentDOM` is always `false`, so the default returns `false` for
 * every mutation — nothing is ignored. This view writes `img.width` on every
 * `pointermove` during a drag, plus `figure.dataset.align`/`img.alt`/
 * `img.title` in `update()`; left unguarded, each of those self-caused
 * writes would be treated as an external DOM mutation and drive
 * reconciliation across the whole node range, caption included — the
 * failure mode is keystrokes vanishing from a caption while a writer types.
 *
 * A selection-type mutation is always let through (matches the framework's
 * own default for that type; a caret change is cheap to reconcile and
 * ProseMirror owns interpreting it, not this view). For every other
 * mutation type, only one whose target lies inside the caption (contentDOM)
 * is a real document edit and gets through; anything outside it (the img,
 * the chrome this view itself writes to) is this view's own doing and is
 * ignored.
 */
export function shouldIgnoreWritingImageMutation(
  contentDOM: Node,
  mutation: { type: string; target: Node }
): boolean {
  if (mutation.type === 'selection') return false
  return !contentDOM.contains(mutation.target)
}

/**
 * `stopEvent` (Important). A contentDOM-bearing node view makes
 * prosemirror-view mark `nodeDOM` (the figure) as a native HTML5 drag
 * source once the node is selected — `if (this.contentDOM || ...)
 * this.nodeDOM.draggable = true` (dist/index.js:1490-1493) — regardless of
 * this node's own `draggable` spec value; that flag on the schema controls
 * a different thing and cannot turn this off. Selecting the image is the
 * normal path to reaching for the resize handle, so without this, that grab
 * can start a whole-figure drag instead of the pointer-based resize this
 * view implements.
 *
 * Claims every event whose target lies inside one of the given interactive
 * roots — the alignment/alt toolbar and the resize handle — so ProseMirror
 * never tries to interpret it as a node-level interaction (selection,
 * drag); each root's own listeners, wired where its elements are built,
 * handle it instead. Takes a *list* of roots, not one container, because
 * the handle is no longer nested inside the same chrome as the toolbar
 * (defect 3's fix): it lives in the image's own shrink-wrapped frame so it
 * can sit on the image's corner instead of the far edge of the full-width
 * figure, while the toolbar stays a separate, independently shown panel. An
 * event whose target is the image itself, the caption, or anything outside
 * every given root is left alone: ProseMirror still needs those (e.g. a
 * click on the image establishing a node selection).
 */
export function shouldStopWritingImageEvent(roots: readonly Node[], event: Event): boolean {
  return event.target instanceof Node && roots.some((root) => root.contains(event.target as Node))
}
