import { describe, it, expect } from 'vitest'
import { resolveDropPaneId } from './pane-drop-target'

const left = { paneId: 'left', rect: { left: 0, top: 0, right: 500, bottom: 400 } }
const right = { paneId: 'right', rect: { left: 500, top: 0, right: 1000, bottom: 400 } }

describe('resolveDropPaneId', () => {
  it('resolves to the pane whose rect contains the drop position', () => {
    expect(resolveDropPaneId({ x: 100, y: 100 }, [left, right], 'left')).toBe('left')
    expect(resolveDropPaneId({ x: 700, y: 100 }, [left, right], 'left')).toBe('right')
  })

  it('falls back to the active pane when the position is outside every rect (e.g. dropped over the tab strip)', () => {
    expect(resolveDropPaneId({ x: 100, y: 900 }, [left, right], 'right')).toBe('right')
  })

  it('a single mounted pane always resolves to it', () => {
    expect(resolveDropPaneId({ x: 5000, y: 5000 }, [left], 'left')).toBe('left')
  })

  it('a shared boundary resolves to the first matching rect in encounter order', () => {
    expect(resolveDropPaneId({ x: 500, y: 100 }, [left, right], 'right')).toBe('left')
  })
})

// Tauri's `onDragDropEvent` reports the drop position in *physical* pixels
// (`DragDropEvent.position` is a `PhysicalPosition`, per @tauri-apps/api's
// webview.d.ts), while `PaneRect` is measured with `getBoundingClientRect()`,
// which is CSS/logical pixels. On any display where devicePixelRatio !== 1
// those two units disagree, so `resolveDropPaneId` divides by
// `devicePixelRatio` before comparing — the same conversion
// `writing-image-drop.ts`'s `handleWritingImageDrop` already applies to the
// in-editor drop target.
describe('resolveDropPaneId — devicePixelRatio conversion', () => {
  it('converts a physical-pixel position to logical pixels before matching a rect', () => {
    // DPR 2: physical x=900 -> logical 450, inside `left` (0-500 logical).
    // Left uncorrected, 900 would fall inside `right` (500-1000) instead.
    expect(resolveDropPaneId({ x: 900, y: 200 }, [left, right], 'right', 2)).toBe('left')
  })

  it('falls back to the active pane once the converted position is outside every rect', () => {
    // DPR 2: physical y=2000 -> logical 1000, below every rect's bottom (400).
    expect(resolveDropPaneId({ x: 100, y: 2000 }, [left, right], 'left', 2)).toBe('left')
  })

  it('treats an omitted devicePixelRatio as 1 (no conversion)', () => {
    expect(resolveDropPaneId({ x: 700, y: 100 }, [left, right], 'left')).toBe('right')
  })

  it('a physical position exactly on the converted boundary resolves to the first matching rect', () => {
    // DPR 2: physical x=1000 -> logical 500, the shared boundary.
    expect(resolveDropPaneId({ x: 1000, y: 100 }, [left, right], 'right', 2)).toBe('left')
  })
})
