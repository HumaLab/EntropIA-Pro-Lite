/**
 * Lifecycle of the pdf.js text layer (docs/zotero-annotations-plan.md,
 * section 9): one layer per `(attachment, page, viewport scale)`. Any other
 * identity cancels and removes the current layer and builds a new one, even
 * between two pages of the same size at the same zoom; the same identity
 * keeps it.
 */

export interface TextLayerIdentity {
  /** The attachment, e.g. its asset URL. */
  document: string
  page: number
  scale: number
}

export interface TextLayerHandle {
  /** Stops a render still in progress. */
  cancel(): void
  /** Removes the layer's DOM. */
  remove(): void
}

export function sameTextLayerIdentity(a: TextLayerIdentity, b: TextLayerIdentity): boolean {
  return a.document === b.document && a.page === b.page && Math.abs(a.scale - b.scale) < 1e-6
}

export class TextLayerController {
  private current: { identity: TextLayerIdentity; handle: TextLayerHandle } | null = null

  get identity(): TextLayerIdentity | null {
    return this.current?.identity ?? null
  }

  /** Builds the layer for `identity` unless it is already the current one. */
  sync(identity: TextLayerIdentity, build: () => TextLayerHandle): 'kept' | 'built' {
    if (this.current && sameTextLayerIdentity(this.current.identity, identity)) return 'kept'
    this.clear()
    this.current = { identity, handle: build() }
    return 'built'
  }

  /** Cancels and removes the current layer, if any. */
  clear(): void {
    const current = this.current
    this.current = null
    if (!current) return
    current.handle.cancel()
    current.handle.remove()
  }
}
