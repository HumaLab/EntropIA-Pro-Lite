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

/**
 * Opens the native dialog, reads the picked file's bytes, imports them into
 * managed storage (writing-images.ts) and reads their intrinsic size
 * (image-dimensions.ts) — the same two steps every entry path takes.
 *
 * Returns `null` when the user cancels the dialog, or when the picked file
 * is not an accepted format: nothing is written and nothing is returned to
 * insert in either case (spec, Failure Handling). `io` is injectable for
 * testing, exactly as `importWritingImage` itself takes one — production
 * callers omit it and get the real Tauri-backed storage.
 */
export async function pickWritingImage(io?: WritingImageIo): Promise<PickedWritingImage | null> {
  const selected = await open({
    multiple: false,
    filters: [{ name: 'Imágenes', extensions: WRITING_IMAGE_EXTENSIONS }],
  })
  if (!selected || Array.isArray(selected)) return null

  const bytes = await readFile(selected)
  const imported = await importWritingImage(bytes, io)
  if (!imported) {
    void appendLog('error', 'writing-image', `Formato de imagen no admitido: ${selected}`).catch(
      (error) => {
        console.error('[writing-image-picker] Failed to append diagnostic log:', error)
      }
    )
    return null
  }

  const size = imageSize(bytes)
  return {
    src: imported.path,
    alt: null,
    title: null,
    width: size?.width ?? null,
    height: size?.height ?? null,
    align: 'center',
  }
}
