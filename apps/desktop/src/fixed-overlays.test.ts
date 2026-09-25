import { readdirSync, readFileSync } from 'node:fs'
import { relative, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * Every `position: fixed` element that can render inside a work pane is
 * portalled out of it.
 *
 * `.work-pane` is a CSS size container (`container-type: inline-size`) so the
 * views can reflow with `@container pane` rules. Size containment makes the
 * pane the containing block of its fixed descendants, so a full-window overlay
 * left inside it covers the pane only. The fix is to move the overlay root out
 * with `{@attach portal}`; this test keeps a new fixed rule from forgetting it.
 *
 * happy-dom computes no containing blocks, so this is a source check. What it
 * cannot see (a fixed element inside a portalled root is fine) is the reason
 * for the per-file granularity: a file with a fixed rule must attach a portal.
 */
const APP_SRC = import.meta.dirname
const UI_SRC = resolve(import.meta.dirname, '../../../packages/ui/src')

function componentsUnder(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = resolve(dir, entry.name)
    if (entry.isDirectory()) {
      return /__tests__|__mocks__|__fixtures__/.test(entry.name) ? [] : componentsUnder(full)
    }
    return entry.name.endsWith('.svelte') ? [full] : []
  })
}

/**
 * Fixed elements that never render inside a pane. Each entry says why, because
 * an exception without a reason is just a hole.
 */
const NEVER_IN_A_PANE: Record<string, string> = {
  'packages/ui/src/components/Tooltip/TooltipLayer.svelte':
    'mounted once by AppShell at the shell root, outside every work pane',
  'apps/desktop/src/layout/EntropicConstellation.svelte':
    'the window background, mounted by AppShell before the shell',
}

const FIXED = /position:\s*fixed|style:position=["']fixed["']/
const PORTALLED = /\{@attach portal\}/

function key(file: string): string {
  return relative(resolve(APP_SRC, '../../..'), file).replaceAll('\\', '/')
}

describe('fixed overlays', () => {
  const files = [
    ...componentsUnder(resolve(APP_SRC, 'views')),
    ...componentsUnder(resolve(APP_SRC, 'components')),
    ...componentsUnder(resolve(APP_SRC, 'layout')),
    ...componentsUnder(resolve(UI_SRC, 'components')),
  ]

  it('scans the views and the shared components', () => {
    expect(files.some((file) => key(file).endsWith('views/DbBrowserView.svelte'))).toBe(true)
    expect(files.some((file) => key(file).endsWith('ConfirmDialog/ConfirmDialog.svelte'))).toBe(
      true
    )
  })

  it('portals every fixed element out of the work pane', () => {
    const offenders = files
      .filter((file) => !(key(file) in NEVER_IN_A_PANE))
      .filter((file) => {
        const source = readFileSync(file, 'utf-8')
        return FIXED.test(source) && !PORTALLED.test(source)
      })
      .map(key)

    expect(offenders).toEqual([])
  })

  it('keeps the allowlist honest', () => {
    for (const path of Object.keys(NEVER_IN_A_PANE)) {
      const file = files.find((candidate) => key(candidate) === path)
      expect(file, `${path} no longer exists`).toBeDefined()
      expect(FIXED.test(readFileSync(file!, 'utf-8')), `${path} is no longer fixed`).toBe(true)
    }
  })
})
