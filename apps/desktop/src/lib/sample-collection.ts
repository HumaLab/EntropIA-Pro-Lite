import { invoke } from '@tauri-apps/api/core'
import { join } from '@tauri-apps/api/path'
import { mkdir, remove, writeFile } from '@tauri-apps/plugin-fs'
import { getStore } from './db'
import { importClassifiedPathsIntoCollection } from './collection-import'
import documents from '../assets/sample-collection/documents.json'

/**
 * A sample collection to try EntropIA on (T-37): nine invented documents
 * from a fictional Mar del Plata archive — letters, a telegram, council
 * minutes, an inventory, a clipping and two photographs — with their
 * transcriptions already in place, so search, entities and the batch tab
 * work at once without paying for OCR.
 *
 * The images ship inside the frontend bundle. Import needs a file on disk,
 * so each one is written to a staging folder in the data directory (inside
 * the fs scope), imported through the normal path and removed afterwards.
 */

interface SampleDocument {
  file: string
  title: string
  text: string
}

const IMAGE_URLS = import.meta.glob<string>('../assets/sample-collection/*.png', {
  query: '?url',
  import: 'default',
  eager: true,
})

/** Fixed so sync can recognise the sample and keep it on this device: the
 *  push skips every row under this collection (`sync::push::SAMPLE_COLLECTION_ID`,
 *  keep both in step). */
export const SAMPLE_COLLECTION_ID = '00000000-0000-4000-8000-000000005a3e'

export const SAMPLE_COLLECTION_NAME = 'Ejemplo — Archivo Bristol 1913-1938 (ficticio)'
const SAMPLE_DESCRIPTION =
  'Documentos inventados para probar EntropIA. Las personas y los hechos son ficticios. Podés borrar esta colección cuando quieras.'

function urlFor(file: string): string {
  const entry = Object.entries(IMAGE_URLS).find(([path]) => path.endsWith(`/${file}`))
  if (!entry) throw new Error(`sample image missing from the bundle: ${file}`)
  return entry[1]
}

/** Creates the sample collection and returns its id and name. */
export async function loadSampleCollection(): Promise<{ id: string; name: string }> {
  const store = getStore()
  const samples = documents as SampleDocument[]
  const dataDir = await invoke<string>('resolve_data_dir')
  const staging = await join(dataDir, 'sample-staging')
  await mkdir(staging, { recursive: true })

  const paths: string[] = []
  for (const sample of samples) {
    const bytes = new Uint8Array(await (await fetch(urlFor(sample.file))).arrayBuffer())
    const path = await join(staging, sample.file)
    await writeFile(path, bytes)
    paths.push(path)
  }

  const collection = await store.collections.create({
    id: SAMPLE_COLLECTION_ID,
    name: SAMPLE_COLLECTION_NAME,
    description: SAMPLE_DESCRIPTION,
  })
  try {
    for (const [index, sample] of samples.entries()) {
      // One file at a time, so each document keeps its own title.
      const result = await importClassifiedPathsIntoCollection([paths[index]!], collection.id, {
        baseErrorMessage: 'sample import failed',
        overrides: { title: sample.title, allowDuplicate: true },
      })
      if (result.importErrors.length > 0) throw new Error(result.importErrors.join('; '))
      const item = result.createdItems[0]
      if (!item) continue
      for (const asset of await store.assets.findByItem(item.id)) {
        await store.extractions.upsert({
          assetId: asset.id,
          textContent: sample.text,
          method: 'sample',
        })
      }
      await store.fts.indexItem(item.id, sample.title, '', sample.text)
    }
  } finally {
    await remove(staging, { recursive: true }).catch(() => {})
  }
  return { id: collection.id, name: collection.name }
}
