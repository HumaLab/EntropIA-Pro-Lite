import { describe, expect, it } from 'vitest'
import { firstVisibleAsset, visibleAssets } from './asset-visibility'

/**
 * Which of an item's assets a person ever sees.
 *
 * The defect that produced this module: a note filed against
 * `findByItem(...)[0]` landed on the container of a scanned legajo. The note
 * was written and the confirmation appeared, and it showed up on none of the
 * pages — because the one asset holding it is the one asset the viewer never
 * draws. A thing attached to something invisible cannot be told apart from a
 * thing that was never saved.
 */

const page = (id: string, parent: string) => ({ id, parentAssetId: parent })
const container = (id: string) => ({ id, parentAssetId: null })

describe('what is shown', () => {
  it('hides the container and keeps its pages', () => {
    const assets = [container('pdf'), page('p1', 'pdf'), page('p2', 'pdf')]

    expect(visibleAssets(assets).map((asset) => asset.id)).toEqual(['p1', 'p2'])
  })

  /**
   * Derived from the set, not from a flag: an item whose PDF was never split
   * has no children naming it, so it is shown because it is all there is.
   */
  it('shows a lone document, which is nobody’s parent', () => {
    expect(visibleAssets([container('solo')]).map((a) => a.id)).toEqual(['solo'])
  })

  it('keeps the order it was given', () => {
    const assets = [page('p2', 'pdf'), container('pdf'), page('p1', 'pdf')]

    expect(visibleAssets(assets).map((asset) => asset.id)).toEqual(['p2', 'p1'])
  })

  it('has nothing to show for nothing', () => {
    expect(visibleAssets([])).toEqual([])
  })
})

describe('the first one someone could open', () => {
  /**
   * The bug in one assertion: the container sorts before its own pages by path,
   * so taking the head of the list put the note where nobody would find it.
   */
  it('skips the container that sorts ahead of its pages', () => {
    const assets = [container('149.pdf'), page('149_page_1', '149.pdf')]

    expect(firstVisibleAsset(assets)?.id).toBe('149_page_1')
  })

  it('is the caller’s first, so it is the same asset every time', () => {
    const assets = [page('p1', 'pdf'), page('p2', 'pdf'), container('pdf')]

    expect(firstVisibleAsset(assets)?.id).toBe('p1')
  })

  it('reports nothing when an item has no assets', () => {
    expect(firstVisibleAsset([])).toBeNull()
  })

  /**
   * Every asset claiming a parent that is not in the list — a partial read, a
   * deletion part-way through. Hiding them all would file the note nowhere and
   * reintroduce the invisible note this exists to prevent, so a list whose
   * every member is a page is a list of pages.
   */
  it('does not hide everything when the parents are not in the list', () => {
    const assets = [page('p1', 'elsewhere'), page('p2', 'elsewhere')]

    expect(firstVisibleAsset(assets)?.id).toBe('p1')
  })
})
