import { createDrizzleClient } from '../client'
import { runMigrations } from '../runner'
import type { DbClient } from '../types'
import { CollectionRepo } from './collection.repo'
import { ItemRepo } from './item.repo'
import { AssetRepo } from './asset.repo'
import { NoteRepo } from './note.repo'
import { AnnotationRepo } from './annotation.repo'
import { ExtractionRepo } from './extraction.repo'
import { LayoutRepo } from './layout.repo'
import { EntityRepo } from './entity.repo'
import { FtsRepo } from './fts.repo'
import { TripleRepo } from './triple.repo'
import { TranscriptionRepo } from './transcription.repo'
import { TopicRepo } from './topic.repo'

export interface StoreApi {
  collections: CollectionRepo
  items: ItemRepo
  assets: AssetRepo
  notes: NoteRepo
  annotations: AnnotationRepo
  extractions: ExtractionRepo
  layouts: LayoutRepo
  entities: EntityRepo
  fts: FtsRepo
  triples: TripleRepo
  transcriptions: TranscriptionRepo
  topics: TopicRepo
}

export async function initStore(client: DbClient): Promise<StoreApi> {
  await runMigrations(client)
  const db = createDrizzleClient(client)
  return {
    collections: new CollectionRepo(db, client),
    items: new ItemRepo(db, client),
    assets: new AssetRepo(db, client),
    notes: new NoteRepo(db),
    annotations: new AnnotationRepo(db),
    extractions: new ExtractionRepo(db),
    layouts: new LayoutRepo(db),
    entities: new EntityRepo(db),
    fts: new FtsRepo(client),
    triples: new TripleRepo(db),
    transcriptions: new TranscriptionRepo(db),
    topics: new TopicRepo(db),
  }
}
