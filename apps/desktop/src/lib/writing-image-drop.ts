import { readDroppedWritingImage, type PickedWritingImage } from './writing-image-picker'

/**
 * The three things WritingView.svelte's `editorRef` needs to offer for an OS
 * file drop to land in the right place — exactly the seam
 * WritingEditor.svelte exports (`containsPoint`, `posAtCoords`,
 * `insertImage`), named as an interface here so this function stays testable
 * without pulling in the Svelte component or Tauri.
 */
export interface WritingImageDropTarget {
  /** Whether a viewport point falls over the manuscript surface, as
   *  opposed to the outline, the research panel or the toolbar. */
  containsPoint(x: number, y: number): boolean
  /** Maps a viewport point to a document position, or `null` when it
   *  cannot be resolved. */
  posAtCoords(x: number, y: number): number | null
  /** Inserts the image, at `at` when given, at the caret otherwise. */
  insertImage(attrs: PickedWritingImage, at?: number): boolean
}

export interface PhysicalPoint {
  x: number
  y: number
}

/**
 * Decides what a dropped file does to the manuscript. WritingView.svelte's
 * Tauri `onDragDropEvent` handler is thin wiring over this function (see
 * WritingView.dragdrop.test.ts) — this is where the actual behaviour is
 * proven, because rendering the whole view needs the whole store and Tauri
 * behind it (the same reason every other WritingView feature keeps its
 * logic out of the component; writing-image-picker.ts's own comment).
 *
 * Two decisions, in order, matching the spec ("Only drop into the editor",
 * "Where the image lands"):
 *
 * 1. Scoping. `target` is `null` when the writing view is not currently
 *    showing an editable, open manuscript (editorRef only binds in that
 *    branch of WritingView.svelte) — nothing happens. Otherwise,
 *    `containsPoint` decides whether the drop landed on the manuscript
 *    surface itself, as opposed to the outline, the research panel or the
 *    toolbar sitting beside it in the same view; landing outside it also
 *    does nothing, and — deliberately — reads no bytes before deciding
 *    that, so a drop meant for another panel never touches storage.
 *
 * 2. Placement. Tauri's own drop position is in *physical* pixels (the
 *    device's actual pixel density), while `containsPoint`/`posAtCoords`,
 *    like every other DOM coordinate in this app, want CSS/logical pixels;
 *    `devicePixelRatio` converts between them. `posAtCoords` then maps the
 *    (converted) point to an exact document position. Tauri documents this
 *    position as unreliable while a debugger is attached, and DPI
 *    conversion has its own edges (a multi-monitor setup with differing
 *    scale factors, chiefly) — when `posAtCoords` cannot resolve a
 *    position, the image still lands: `insertImage` is called with no `at`,
 *    which falls through to its own default, the current caret, exactly
 *    where the toolbar picker and a paste already put it.
 *
 * Returns whether an image was inserted — useful for tests; the caller does
 * not need the value.
 */
export async function handleWritingImageDrop(
  target: WritingImageDropTarget | null,
  paths: string[],
  position: PhysicalPoint,
  devicePixelRatio: number,
  readImage: (path: string) => Promise<PickedWritingImage | null> = readDroppedWritingImage
): Promise<boolean> {
  if (!target) return false

  const x = position.x / devicePixelRatio
  const y = position.y / devicePixelRatio
  if (!target.containsPoint(x, y)) return false

  const path = paths[0]
  if (!path) return false

  const attrs = await readImage(path)
  if (!attrs) return false

  const pos = target.posAtCoords(x, y)
  return target.insertImage(attrs, pos ?? undefined)
}
