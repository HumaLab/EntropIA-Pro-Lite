import { readdirSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * The accessibility rules of §18 a test can hold (plan-editor.md §18).
 *
 * # Which half this is
 *
 * §18 asks for six things: keyboard, visible focus, labels on icon buttons,
 * contrast, zoom, and no overflow. Three of those are facts about pixels and
 * need eyes on the running application — this file does not pretend otherwise,
 * and `plan-editor.md` §9 records them as pending human verification.
 *
 * The other three are facts about the source, and the worst of them is the
 * unlabelled icon button: a screen reader announces it as "button", the writer
 * who depends on one cannot tell the outline toggle from the export, and
 * nothing about the running application looks wrong to anyone who can see it.
 * That is exactly the kind of defect a test should be holding.
 */

const VIEWS = resolve(import.meta.dirname)

function writingViews(): { name: string; source: string }[] {
  return readdirSync(VIEWS)
    .filter((name) => name.startsWith('Writing') && name.endsWith('.svelte'))
    .map((name) => ({ name, source: readFileSync(resolve(VIEWS, name), 'utf-8') }))
}

/** Each `<IconButton …>` opening tag, attributes and all. */
function iconButtons(source: string): string[] {
  return [...source.matchAll(/<IconButton\b[\s\S]*?>/g)].map(([tag]) => tag)
}

describe('the writing views exist to be checked', () => {
  it('finds them', () => {
    const names = writingViews().map((view) => view.name)

    expect(names).toContain('WritingView.svelte')
    expect(names.length).toBeGreaterThanOrEqual(6)
  })
})

describe('§18: every icon button says what it is', () => {
  /**
   * An icon button with no accessible name is announced as "button". Someone
   * using a screen reader cannot tell the outline toggle from the export, and
   * to everyone else the interface looks perfectly fine.
   */
  it('labels every icon button in every writing view', () => {
    const unlabelled = writingViews().flatMap((view) =>
      iconButtons(view.source)
        .filter((tag) => !/\blabel=/.test(tag) && !/\baria-label=/.test(tag))
        .map((tag) => `${view.name}: ${tag.replace(/\s+/g, ' ').slice(0, 80)}`)
    )

    expect(unlabelled, 'icon buttons with no accessible name').toEqual([])
  })

  /** And the label is translated, never a Spanish string baked into the view. */
  it('takes every label from the translations', () => {
    const untranslated = writingViews().flatMap((view) =>
      iconButtons(view.source)
        .filter((tag) => /\blabel="/.test(tag))
        .map((tag) => `${view.name}: ${tag.replace(/\s+/g, ' ').slice(0, 80)}`)
    )

    expect(untranslated, 'icon button labels that are not translated').toEqual([])
  })
})

describe('§18: focus can be seen', () => {
  /**
   * Whether a ring is *perceptible* is a fact about pixels and needs eyes. That
   * one is declared at all is a fact about the source, and it is the one that
   * disappears quietly: a component refactored to a new stylesheet loses its
   * focus rule and nothing about the running application looks wrong to anyone
   * using a mouse.
   *
   * `:focus-within` counts. For a composite control like the search bar, the
   * input carries `outline: none` on purpose and the wrapper paints the ring —
   * which is the right way round, not an omission.
   */
  it('gives every interactive primitive the writing views use a focus style', () => {
    const PRIMITIVES = [
      'Button/Button.svelte',
      'IconButton/IconButton.svelte',
      'Tabs/TabButton.svelte',
      'Input/Input.svelte',
      'Checkbox/Checkbox.svelte',
      'SearchBar/SearchBar.svelte',
      'ResizeHandle/ResizeHandle.svelte',
    ]
    const root = resolve(VIEWS, '../../../../packages/ui/src/components')

    const unfocusable = PRIMITIVES.filter((path) => {
      const source = readFileSync(resolve(root, path), 'utf-8')
      return !/:focus-visible|:focus-within/.test(source)
    })

    expect(unfocusable, 'primitives with no focus style at all').toEqual([])
  })

  /**
   * The editor surface is the deliberate exception, and the comment saying so
   * has to survive: `:focus-visible` always matches an element that accepts
   * text, so a ring there is permanently on rather than shown on arrival. The
   * caret is the indicator a text surface already has.
   */
  it('keeps the reason the editing surface has no ring', () => {
    const editor = readFileSync(
      resolve(VIEWS, '../../../../packages/ui/src/components/WritingEditor/WritingEditor.svelte'),
      'utf-8'
    )

    expect(editor).toMatch(/No focus ring[\s\S]{0,200}focus-visible/)
  })
})

describe('§18: nothing is announced only by colour or position', () => {
  /**
   * A `role="alert"` or `role="status"` is how a change that is not where the
   * cursor is gets announced at all. The writing views report a good deal that
   * way — a save that failed, a target that moved, a citation that would not
   * render — and a bare red paragraph reaches nobody who is not looking at it.
   */
  it('gives the failure and status messages a role', () => {
    const view = readFileSync(resolve(VIEWS, 'WritingAgentTab.svelte'), 'utf-8')
    const notice = readFileSync(resolve(VIEWS, 'WritingExportNotice.svelte'), 'utf-8')

    for (const [name, source] of [
      ['WritingAgentTab', view],
      ['WritingExportNotice', notice],
    ] as const) {
      const alerts = [...source.matchAll(/class="[^"]*__error[^"]*"([^>]*)>/g)].map(
        ([, rest]) => rest ?? ''
      )
      expect(alerts.length, `${name} has no error line to check`).toBeGreaterThan(0)
      expect(
        alerts.filter((rest) => !/role="alert"/.test(rest)),
        `${name}: error messages with no role`
      ).toEqual([])
    }
  })
})
