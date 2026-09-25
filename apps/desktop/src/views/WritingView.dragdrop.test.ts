import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * An OS file drop is wired to the same Tauri event CollectionView already
 * listens to, scoped to the manuscript, and released on unmount — the same
 * facts CollectionView.test.ts proves by rendering that component with a
 * mocked `getCurrentWebview` and firing a captured handler. Doing that here
 * too would mean mounting the whole writing store and Tauri behind it
 * (WritingView.image.test.ts's and WritingView.dictation.test.ts's own
 * reason for staying at the source level), so this file checks the wiring
 * the same way those do: as a fact about the source. The actual decision —
 * what a drop does to the manuscript — is proven directly, with no Tauri or
 * Svelte in sight, by writing-image-drop.test.ts; this file only has to
 * show that WritingView hands that function the real event.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'WritingView.svelte'), 'utf-8')

describe('drag-and-drop in the writing view', () => {
  it('listens to the same Tauri webview event CollectionView uses', () => {
    expect(SOURCE).toMatch(
      /import\s*\{\s*getCurrentWebview,\s*type DragDropEvent\s*\}\s*from\s*'@tauri-apps\/api\/webview'/
    )
    expect(SOURCE).toMatch(/getCurrentWebview\(\)\s*\n?\s*\.onDragDropEvent/)
  })

  it('only acts on the drop variant, not enter/over/leave', () => {
    expect(SOURCE).toMatch(/if \(event\.payload\.type !== 'drop'\) return/)
  })

  it('delegates the decision to handleWritingImageDrop, with the manuscript editor, the event paths, the event position and the device pixel ratio', () => {
    expect(SOURCE).toMatch(/import \{ handleWritingImageDrop \} from '\$lib\/writing-image-drop'/)
    expect(SOURCE).toMatch(
      /handleWritingImageDrop\(\s*editorRef \?\? null,\s*event\.payload\.paths,\s*event\.payload\.position,\s*window\.devicePixelRatio \|\| 1\s*\)/
    )
  })

  // Task 3.5: Tauri's drag-drop event is webview-wide, so every mounted
  // WritingView instance receives every drop. A drop must only act in the
  // pane whose rect actually contains it — checked at the source level for
  // the same reason the rest of this file is: mounting a real WritingView
  // needs the whole writing store and Tauri behind it. The decision itself
  // is proven directly, with no Svelte in sight, by
  // pane-drop-target.test.ts's `resolveDropPaneId` coverage; this only has
  // to show WritingView actually gates on it before acting.
  it('gates the drop on resolveDropPaneId, using this pane, its own rect registry and the live devicePixelRatio', () => {
    expect(SOURCE).toMatch(/import \{ resolveDropPaneId \} from '\$lib\/pane-drop-target'/)
    expect(SOURCE).toMatch(/import \{ currentPaneRects \} from '\$lib\/pane-rects'/)
    expect(SOURCE).toMatch(
      /resolveDropPaneId\(\s*event\.payload\.position,\s*currentPaneRects\(\),\s*workspace\.activeTabId,\s*window\.devicePixelRatio \|\| 1\s*\)\s*!==\s*paneId/
    )

    // The gate must run before handleWritingImageDrop, and after the
    // `type !== 'drop'` guard (so `event.payload.position` is guaranteed to
    // exist by then).
    const notDropIndex = SOURCE.indexOf("if (event.payload.type !== 'drop') return")
    const gateIndex = SOURCE.indexOf('resolveDropPaneId(')
    const actIndex = SOURCE.indexOf('await handleWritingImageDrop(')
    expect(notDropIndex).toBeGreaterThan(-1)
    expect(gateIndex).toBeGreaterThan(notDropIndex)
    expect(actIndex).toBeGreaterThan(gateIndex)
  })

  it('releases the listener on unmount, like CollectionView does', () => {
    expect(SOURCE).toMatch(/unlistenDragDrop\?\.\(\)/)
    // Registered in onMount, unlistened in onDestroy — never the other way
    // round, which would unsubscribe before the listener is ever captured.
    const onMountIndex = SOURCE.indexOf('onMount(async () => {')
    const registerIndex = SOURCE.indexOf('getCurrentWebview()')
    const onDestroyIndex = SOURCE.indexOf('onDestroy(() => {')
    const releaseIndex = SOURCE.indexOf('unlistenDragDrop?.()')
    expect(onMountIndex).toBeGreaterThan(-1)
    expect(registerIndex).toBeGreaterThan(onMountIndex)
    expect(releaseIndex).toBeGreaterThan(onDestroyIndex)
  })
})
