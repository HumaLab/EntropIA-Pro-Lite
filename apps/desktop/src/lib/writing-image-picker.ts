import { open } from '@tauri-apps/plugin-dialog'
import { readFile } from '@tauri-apps/plugin-fs'
import { appendLog } from './logs'
import { importWritingImage, type WritingImageIo } from './writing-images'
import { imageSize } from './image-dimensions'

/**
 * The toolbar's picker (writing-image-node-design.md, Task 6), pulled out of
 * WritingView.svelte for the same reason `transcribeDictation` lives in
 * transcription.ts rather than inline in the view: a plain async function
 * over the Tauri dialog/fs seam is unit-testable, a Svelte component wired
 * to a store is not.
 */

export const WRITING_IMAGE_EXTENSIONS = ['png', 'jpg', 'jpeg', 'gif']

export interface PickedWritingImage {
  src: string
  alt: null
  title: null
  width: number | null
  height: number | null
  align: 'center'
}

/** Fires the diagnostic log and swallows its own rejection (`appendLog`
 *  fires-and-forgets, matching the refusal-logging this function already did
 *  for an unsupported format). */
function reportAndReturnNull(message: string): null {
  void appendLog('error', 'writing-image', message).catch((error) => {
    console.error('[writing-image-picker] Failed to append diagnostic log:', error)
  })
  return null
}

/**
 * Opens the native dialog, reads the picked file's bytes, imports them into
 * managed storage (writing-images.ts) and reads their intrinsic size
 * (image-dimensions.ts) — the same two steps every entry path takes.
 *
 * Returns `null` when the user cancels the dialog, when the dialog or the
 * read itself fails (I5), or when the picked file is not an accepted format:
 * nothing is written and nothing is returned to insert in any case (spec,
 * Failure Handling). `io` is injectable for testing, exactly as
 * `importWritingImage` itself takes one — production callers omit it and get
 * the real Tauri-backed storage.
 */
export async function pickWritingImage(io?: WritingImageIo): Promise<PickedWritingImage | null> {
  // Wrapped like file-import.ts's pickFiles/pickAndImportFiles wrap the same
  // two calls (I5): a file on a USB drive unplugged mid-pick, a network
  // share, or a path outside the fs capability scope rejects here instead of
  // resolving, and this function reports it and changes nothing — the
  // policy the spec states for every unreadable file (Failure Handling).
  let selected: string | string[] | null
  try {
    selected = await open({
      multiple: false,
      // A language-neutral format acronym, like every other native dialog
      // filter in this codebase (file-import.ts's 'Documents'), not a
      // localized word.
      filters: [{ name: 'Images', extensions: WRITING_IMAGE_EXTENSIONS }],
    })
  } catch (error) {
    return reportAndReturnNull(
      `No se pudo abrir el selector de imágenes: ${error instanceof Error ? error.message : String(error)}`
    )
  }
  if (!selected || Array.isArray(selected)) return null

  let bytes: Uint8Array
  try {
    bytes = await readFile(selected)
  } catch (error) {
    return reportAndReturnNull(
      `No se pudo leer la imagen seleccionada (${selected}): ${error instanceof Error ? error.message : String(error)}`
    )
  }

  const imported = await importWritingImage(bytes, io)
  if (!imported) {
    return reportAndReturnNull(`Formato de imagen no admitido: ${selected}`)
  }

  // C1: `width` is the author's *chosen* width in CSS pixels (spec, Node
  // Shape) — an intrinsic width decoded from the file (often thousands of
  // pixels for a photo) is not that, and storing it here is what made the
  // very first resize drag a no-op: the drag started from the stored
  // intrinsic width while the figure was already rendered at the column
  // width by the stylesheet's `max-width: 100%`, so shrinking it read as
  // "still smaller than what's already there." Left null, the node view's
  // first drag starts from the actually-rendered width (extensions.ts,
  // dragStartWidth) instead. `height` still carries the intrinsic value:
  // never used to stretch the image, only to compute the aspect ratio a
  // resize preserves.
  const size = imageSize(bytes)
  return {
    src: imported.path,
    alt: null,
    title: null,
    width: null,
    height: size?.height ?? null,
    align: 'center',
  }
}
