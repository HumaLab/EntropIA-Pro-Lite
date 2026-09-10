import type { Annotation as StoreAnnotation } from '@entropia/store'
import type { ViewerAnnotationKind, ViewerAnnotation } from '@entropia/ui'
import { cloneViewerAnnotations } from './item-view-image-edit'

type Timer = ReturnType<typeof setTimeout>

export type AnnotationPersistenceInput = Pick<
  ViewerAnnotation,
  'kind' | 'color' | 'x' | 'y' | 'width' | 'height'
>

export interface PendingAnnotationSave {
  assetId: string
  page: number
  annotations: ViewerAnnotation[]
}

export type AnnotationFinder = (assetId: string, page: number) => Promise<StoreAnnotation[]>

export function toAnnotationPersistenceInputs(
  annotations: ViewerAnnotation[]
): AnnotationPersistenceInput[] {
  return annotations.map((annotation) => ({
    kind: annotation.kind,
    color: annotation.color,
    x: annotation.x,
    y: annotation.y,
    width: annotation.width,
    height: annotation.height,
  }))
}

export function toViewerAnnotations(annotations: StoreAnnotation[]): ViewerAnnotation[] {
  return annotations.map((annotation) => ({
    ...annotation,
    kind: annotation.kind as ViewerAnnotationKind,
  }))
}

export async function loadViewerAnnotationsForAsset(
  assetId: string,
  page: number,
  findByAsset: AnnotationFinder
): Promise<ViewerAnnotation[]> {
  return toViewerAnnotations(await findByAsset(assetId, page))
}

export class DebouncedAnnotationPersistor {
  private timers = new Map<string, Timer>()
  private pendingSaves = new Map<string, PendingAnnotationSave>()
  private inFlight = new Map<string, Promise<void>>()

  constructor(
    private readonly options: {
      delayMs: number
      persist: (assetId: string, page: number, annotations: ViewerAnnotation[]) => Promise<void>
      onError?: (error: unknown) => void
    }
  ) {}

  schedule(assetId: string, page: number, annotations: ViewerAnnotation[]) {
    const key = this.scopeKey(assetId, page)
    this.clearTimer(key)
    this.pendingSaves.set(key, { assetId, page, annotations: cloneViewerAnnotations(annotations) })

    const timer = setTimeout(() => {
      const saveJob = this.pendingSaves.get(key)
      this.pendingSaves.delete(key)
      this.timers.delete(key)

      if (!saveJob) {
        return
      }

      void this.startSave(key, saveJob)
    }, this.options.delayMs)
    this.timers.set(key, timer)
  }

  async flushPending() {
    for (const key of [...this.timers.keys()]) this.clearTimer(key)
    const saveJobs = [...this.pendingSaves.values()]
    this.pendingSaves.clear()
    for (const saveJob of saveJobs) {
      void this.startSave(this.scopeKey(saveJob.assetId, saveJob.page), saveJob)
    }
    await Promise.all(this.inFlight.values())
  }

  private startSave(key: string, saveJob: PendingAnnotationSave): Promise<void> {
    const previous = this.inFlight.get(key) ?? Promise.resolve()
    const task = previous
      .then(() => this.options.persist(saveJob.assetId, saveJob.page, saveJob.annotations))
      .catch((error) => {
        // A failed old save must never replace a newer queued edit.
        if (this.inFlight.get(key) === task && !this.pendingSaves.has(key)) {
          this.pendingSaves.set(key, saveJob)
        }
        this.options.onError?.(error)
      })
      .finally(() => {
        if (this.inFlight.get(key) === task) this.inFlight.delete(key)
      })
    this.inFlight.set(key, task)
    return task
  }

  getPendingAssetId() {
    return this.pendingSaves.values().next().value?.assetId ?? null
  }

  cancelAll() {
    for (const key of [...this.timers.keys()]) this.clearTimer(key)
    this.pendingSaves.clear()
  }

  private scopeKey(assetId: string, page: number) {
    return `${assetId}\u0000${page}`
  }

  private clearTimer(key: string) {
    const timer = this.timers.get(key)
    if (!timer) return
    clearTimeout(timer)
    this.timers.delete(key)
  }
}
