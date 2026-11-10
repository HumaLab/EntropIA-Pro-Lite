import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { AssetFileFacts } from './asset-file-facts'
import {
  checkLayoutVersion,
  layoutRerunMode,
  layoutVersionNeeds,
  layoutVersionVerdict,
  type LayoutVersionNeeds,
} from './layout-version'

const { assetFileFactsMock } = vi.hoisted(() => ({ assetFileFactsMock: vi.fn() }))

vi.mock('./asset-file-facts', () => ({ assetFileFacts: assetFileFactsMock }))

interface RuleCase {
  name: string
  assetType: string
  layout: { sourceSha256: string | null; imageWidth: number; imageHeight: number }
  file: AssetFileFacts | null
  needs: LayoutVersionNeeds
  verdict: 'matches' | 'other_version'
}

// The same table drives the Rust rule: `derived_source.rs` reads this very file
// in its tests, so the two implementations cannot drift apart unnoticed.
const CASES: RuleCase[] = JSON.parse(
  readFileSync(resolve(import.meta.dirname, 'layout-version.cases.json'), 'utf-8')
)

describe('layout version rule (table shared with Rust)', () => {
  it('covers every branch of the rule', () => {
    expect(CASES.length).toBeGreaterThanOrEqual(15)
    expect(new Set(CASES.map((c) => c.name)).size).toBe(CASES.length)
  })

  it.each(CASES)('$name', ({ assetType, layout, file, needs, verdict }) => {
    expect(layoutVersionNeeds(assetType, layout)).toBe(needs)
    expect(layoutVersionVerdict(assetType, layout, file).status).toBe(verdict)
  })

  it('reports both sizes so the notice can show them', () => {
    expect(
      layoutVersionVerdict(
        'image',
        { sourceSha256: null, imageWidth: 747, imageHeight: 1169 },
        { orientation: 1, width: 3456, height: 4608, sha256: null }
      )
    ).toEqual({
      status: 'other_version',
      layout: { width: 747, height: 1169 },
      current: { width: 3456, height: 4608 },
    })
  })

  it('has no current size to report for a file that is not an image', () => {
    expect(
      layoutVersionVerdict(
        'image',
        { sourceSha256: 'aaa', imageWidth: 747, imageHeight: 1169 },
        { orientation: 1, width: null, height: null, sha256: 'bbb' }
      )
    ).toEqual({
      status: 'other_version',
      layout: { width: 747, height: 1169 },
      current: null,
    })
  })

  it('does not vouch for an image layout when there are no facts at all', () => {
    expect(
      layoutVersionVerdict(
        'image',
        { sourceSha256: null, imageWidth: 747, imageHeight: 1169 },
        null
      ).status
    ).toBe('other_version')
  })
})

describe('layoutRerunMode', () => {
  it('runs the engine that computed the stale layout again', () => {
    expect(layoutRerunMode('glm_ocr', true)).toBe('high')
    expect(layoutRerunMode('paddle_vl', true)).toBe('light')
  })

  it('falls back to the engine every build has when the local one is not there', () => {
    expect(layoutRerunMode('paddle_vl', false)).toBe('high')
    expect(layoutRerunMode('glm_ocr', false)).toBe('high')
  })
})

describe('checkLayoutVersion', () => {
  const stamped = { sourceSha256: 'aaa', imageWidth: 747, imageHeight: 1169 }
  const unstamped = { ...stamped, sourceSha256: null }

  beforeEach(() => {
    assetFileFactsMock.mockReset()
  })

  it('asks for the hash only when the layout carries one', async () => {
    assetFileFactsMock.mockResolvedValue({
      orientation: 1,
      width: 747,
      height: 1169,
      sha256: 'aaa',
    })

    await expect(checkLayoutVersion({ id: 'a1', type: 'image' }, stamped)).resolves.toEqual({
      status: 'matches',
    })
    expect(assetFileFactsMock).toHaveBeenLastCalledWith('a1', { withSha: true })

    await expect(checkLayoutVersion({ id: 'a2', type: 'image' }, unstamped)).resolves.toEqual({
      status: 'matches',
    })
    expect(assetFileFactsMock).toHaveBeenLastCalledWith('a2', { withSha: false })
  })

  it('reads nothing for a PDF asset', async () => {
    await expect(checkLayoutVersion({ id: 'p1', type: 'pdf' }, stamped)).resolves.toEqual({
      status: 'matches',
    })
    expect(assetFileFactsMock).not.toHaveBeenCalled()
  })

  it('answers unknown when the file cannot be read', async () => {
    assetFileFactsMock.mockRejectedValue(new Error('Failed to read assets/c1/gone.jpg'))
    await expect(checkLayoutVersion({ id: 'a1', type: 'image' }, unstamped)).resolves.toEqual({
      status: 'unknown',
    })

    assetFileFactsMock.mockResolvedValue(null)
    await expect(checkLayoutVersion({ id: 'a1', type: 'image' }, unstamped)).resolves.toEqual({
      status: 'unknown',
    })
  })
})
