import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import {
  CITATION_FIDELITY,
  MARK_FIDELITY,
  NODE_FIDELITY,
  REQUIRED_BY_SPEC,
} from './export-fidelity'

/**
 * The export vocabulary, held equal to the matrix that produces it.
 *
 * An element only ever reaches the screen through a warning or a refusal, and
 * both of those look up `writing.exportElement.<name>`. Nothing at compile time
 * connects the matrix to the locales — a template literal cast to `I18nKey` is
 * exactly a hole in the type system — so a node marked as a fallback without a
 * label would tell the writer that `noteLink: 3 casos` go out another way.
 *
 * The same guard as `agent-actions-vocabulary.test.ts`, for the same reason: a
 * list that must match another list, with nothing but a test to hold them.
 */

const I18N = readFileSync(resolve(import.meta.dirname, 'i18n.ts'), 'utf-8')

/** How many locales declare a label for this element. */
function labelledIn(element: string): number {
  return [...I18N.matchAll(new RegExp(`'writing\\.exportElement\\.${element}':`, 'g'))].length
}

/**
 * Everything that can ever be named on screen: anything a format does not carry
 * natively, plus everything §17.4 lets a DOCX refusal name.
 */
function nameable(): string[] {
  const names = new Set<string>(REQUIRED_BY_SPEC)
  for (const [element, support] of [
    ...Object.entries(NODE_FIDELITY),
    ...Object.entries(MARK_FIDELITY),
  ]) {
    if (Object.values(support).some((value) => value !== 'native')) names.add(element)
  }
  return [...names]
}

describe('the export vocabulary', () => {
  it('finds the elements that can be named on screen', () => {
    // The two that are a fallback today, and the obligatory list a refusal
    // draws from. If this shrinks to nothing the guard below proves nothing.
    expect(nameable()).toContain('noteLink')
    expect(nameable()).toContain('underline')
    expect(nameable()).toEqual(
      expect.arrayContaining(['subscript', 'superscript', 'textStyle', 'highlight'])
    )
    expect(nameable().length).toBeGreaterThanOrEqual(REQUIRED_BY_SPEC.length)
  })

  it('labels every nameable element in both locales', () => {
    const unlabelled = nameable().filter((element) => labelledIn(element) !== 2)

    expect(unlabelled, 'elements with no label in each locale').toEqual([])
  })

  /**
   * The citation representations are labelled by the dialog's own list, so what
   * is checked here is that the matrix and that list hold the same four.
   */
  it('knows the same four representations the dialog offers', () => {
    const dialog = readFileSync(
      resolve(import.meta.dirname, '../views/WritingExportDialog.svelte'),
      'utf-8'
    )
    const offered = [...dialog.matchAll(/\{ id: '([a-z_]+)', label: 'writing\.exportCite/g)].map(
      ([, id]) => id!
    )

    expect(offered.sort()).toEqual(Object.keys(CITATION_FIDELITY).sort())
  })
})
