import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * The agent's command grid.
 *
 * jsdom performs no layout, so what it cannot see is a track rule that stops
 * reflowing and a label that stops wrapping — and `.btn` sets `white-space:
 * nowrap`, so a long action name would push its own button past the panel
 * rather than take a second line.
 */
const SOURCE = readFileSync(resolve(import.meta.dirname, 'WritingAgentTab.svelte'), 'utf-8')

/** Comments stripped: these rules are documented in prose naming the very
 *  declarations under test. */
const STYLES = SOURCE.replace(/\/\*[\s\S]*?\*\//g, '')

function ruleFor(selector: string): string {
  const at = STYLES.indexOf(selector)
  expect(at, `${selector} is no longer in the stylesheet`).toBeGreaterThan(-1)
  const rule = STYLES.slice(at)
  return rule.slice(0, rule.indexOf('}'))
}

describe('the action grid reflows with the panel', () => {
  it('sizes its columns from the panel width, not from a column count', () => {
    expect(ruleFor('.agent__actions {')).toMatch(
      /grid-template-columns:\s*repeat\(auto-(fit|fill),\s*minmax\(\d+px,\s*1fr\)\)/
    )
  })

  it('keeps a floor two columns fit behind at the panel default width', () => {
    // RESEARCH_BOUNDS defaults to 280px and the body pads it by 8 on each side,
    // so a floor above 128 leaves one column at the width most writers never
    // change. This is the arithmetic that chose the number, kept as an
    // assertion so raising it has to be deliberate.
    const floor = Number(ruleFor('.agent__actions {').match(/minmax\((\d+)px/)?.[1])
    const gap = 8
    const usableAtDefault = 280 - 16

    expect(Math.floor((usableAtDefault + gap) / (floor + gap))).toBeGreaterThanOrEqual(2)
  })
})

describe('a long action name takes a second line instead of the panel', () => {
  it('undoes the nowrap every button carries', () => {
    const button = ruleFor('.agent__actions :global(.agent__action-btn) {')

    // `.btn` in the shared component sets `white-space: nowrap`. Without this
    // override `Buscar evidencia en contra` renders on one line and widens its
    // own column past the grid.
    expect(button).toMatch(/white-space:\s*normal/)
    expect(button).toMatch(/height:\s*auto/)
  })

  it('leaves every interaction state to the shared button', () => {
    const button = ruleFor('.agent__actions :global(.agent__action-btn) {')

    // The variant already carries hover, focus-visible and disabled. Restating
    // any of them here is how a control starts to look like it came from
    // somewhere else, so the override is geometry only.
    expect([
      /background/.test(button),
      /border-color/.test(button),
      /box-shadow/.test(button),
      /opacity/.test(button),
    ]).toEqual([false, false, false, false])
  })
})

describe('an unavailable action still says why', () => {
  it('puts the reason on the item, because a disabled button takes no hover', () => {
    // The title sits on the <li>, and actionTitle is what folds the reason into
    // it. The Button must not carry a title of its own: it is disabled exactly
    // when there is something to explain, and a disabled button receives no
    // pointer events, so that tooltip would never appear.
    expect(SOURCE).toContain('<li class="agent__action" use:tooltip={actionTitle(action)}>')
    expect(SOURCE).toMatch(/function actionTitle[\s\S]*?writing\.agentUnavailable/)

    // Neither a native title nor a tooltip of its own: the reason belongs to
    // the item, which stays hoverable while the button is disabled.
    const button = SOURCE.slice(SOURCE.indexOf('<Button'), SOURCE.indexOf('</Button>'))
    expect([button.includes('title='), button.includes('use:tooltip')]).toEqual([false, false])
  })

  it('keeps the words in the document for a screen reader', () => {
    expect(SOURCE).toContain(`<span class="agent__off">{t('writing.agentUnavailable')}</span>`)
    expect(ruleFor('.agent__off {')).toMatch(/clip-path:\s*inset\(50%\)/)
  })
})

/**
 * The two label families have to cover the same actions.
 *
 * `en` is typed `Record<keyof typeof es, string>`, so a key missing from one
 * LOCALE is already a compile error — that is checked and needs no test here.
 * What no compiler sees is the pair of FAMILIES: the tab reads its labels
 * through `` t(`writing.agentActionShort.${action.id}` as I18nKey) ``, and the
 * cast is what makes that call typecheck for an id that has a long name and no
 * short one. The button would then render the raw key.
 */
describe('every action has both a face and a full name', () => {
  const I18N = readFileSync(resolve(import.meta.dirname, '../lib/i18n.ts'), 'utf-8')
  const EN_AT = I18N.indexOf('const en:')

  /**
   * The action ids declared under one key family, in one locale block.
   *
   * Plain string work rather than a built regex: the prefix ends in a literal
   * dot, and `'writing.agentAction.` cannot match a `'writing.agentActionShort.`
   * line by construction — which is the whole distinction this test rests on.
   */
  function ids(source: string, family: string): string[] {
    const prefix = `'writing.${family}.`
    return source
      .split('\n')
      .map((line) => line.trim())
      .filter((line) => line.startsWith(prefix))
      .map((line) => line.slice(prefix.length).split("'")[0] as string)
      .sort()
  }

  const es = I18N.slice(0, EN_AT)
  const en = I18N.slice(EN_AT)

  it('pairs a short label with every full one, in both locales', () => {
    const expected = ids(es, 'agentAction')

    expect(expected.length).toBeGreaterThan(0)
    expect([
      ids(es, 'agentActionShort'),
      ids(en, 'agentAction'),
      ids(en, 'agentActionShort'),
    ]).toEqual([expected, expected, expected])
  })

  it('shows the short label and keeps the full one for hover', () => {
    expect(SOURCE).toContain('t(`writing.agentActionShort.${action.id}` as I18nKey)')
    expect(SOURCE).toContain('t(`writing.agentAction.${action.id}` as I18nKey)')
    expect(SOURCE).toContain('use:tooltip={actionTitle(action)}')
  })

  it('does not put the full name in an aria-label', () => {
    // The visible label is "Contraevidencia" and the full name is "Buscar
    // evidencia en contra". An accessible name that does not contain its
    // visible label breaks WCAG 2.5.3, so the full name stays a tooltip.
    expect(SOURCE).not.toMatch(/aria-label=\{[^}]*agentAction\./)
  })
})
