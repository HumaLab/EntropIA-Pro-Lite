import type { Annotation as StoreAnnotation } from '@entropia/store'
import type { ViewerAnnotationKind, ViewerAnnotation } from '@entropia/ui'

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
    this.pendingSaves.set(key, { assetId, page, annotations })

    const timer = setTimeout(async () => {
      const saveJob = this.pendingSaves.get(key)
      this.pendingSaves.delete(key)
      this.timers.delete(key)

      if (!saveJob) {
        return
      }

      try {
        await this.options.persist(saveJob.assetId, saveJob.page, saveJob.annotations)
      } catch (error) {
        this.pendingSaves.set(key, saveJob)
        this.options.onError?.(error)
      }
    }, this.options.delayMs)
    this.timers.set(key, timer)
  }

  async flushPending() {
    for (const key of [...this.timers.keys()]) this.clearTimer(key)
    const saveJobs = [...this.pendingSaves.values()]
    this.pendingSaves.clear()
    await Promise.all(
      saveJobs.map(async (saveJob) => {
        try {
          await this.options.persist(saveJob.assetId, saveJob.page, saveJob.annotations)
        } catch (error) {
          this.pendingSaves.set(this.scopeKey(saveJob.assetId, saveJob.page), saveJob)
          this.options.onError?.(error)
        }
      })
    )
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
