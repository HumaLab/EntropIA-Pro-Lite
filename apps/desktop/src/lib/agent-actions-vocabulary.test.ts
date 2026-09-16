import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * The agent's action list, held equal to the three places that must agree.
 *
 * The capability matrix in `agent_actions.rs` decides which actions exist, the
 * prompt in `agent_prompt.rs` decides what each one asks the model, and the
 * locales decide what the writer reads on the button. Nothing at compile time
 * connects them, and each way of drifting fails quietly in its own way:
 *
 * - an action with no label shows its bare id, `find_counter_evidence`, on a
 *   button in a Spanish interface;
 * - an action with no instruction still runs, and silently asks the model the
 *   generic fallback instead of what the button promised — the worst of the
 *   three, because it looks like it worked.
 *
 * So the matrix is the source and the other two are checked against it.
 */

const at = (path: string) => resolve(import.meta.dirname, path)

const MATRIX = readFileSync(at('../../src-tauri/src/writing/agent_actions.rs'), 'utf-8')
const PROMPT = readFileSync(at('../../src-tauri/src/writing/agent_prompt.rs'), 'utf-8')
const I18N = readFileSync(at('i18n.ts'), 'utf-8')

/** Every action the matrix publishes. */
function actionsInMatrix(): string[] {
  const list = MATRIX.slice(MATRIX.indexOf('const ACTIONS'))
  const ids = [...list.slice(0, list.indexOf('\n];')).matchAll(/\("([a-z_]+)",/g)].map(
    ([, id]) => id!
  )
  expect(ids.length, 'no actions found in the matrix').toBeGreaterThan(0)
  return ids
}

/** Every action the prompt module has an instruction for. */
function actionsInPrompt(): string[] {
  const body = PROMPT.slice(PROMPT.indexOf('fn instruction('))
  return [...body.slice(0, body.indexOf('\n}')).matchAll(/^\s*"([a-z_]+)" =>/gm)].map(
    ([, id]) => id!
  )
}

/** Every locale that declares a label for a given action. */
function labelledIn(id: string): number {
  return [...I18N.matchAll(new RegExp(`'writing\\.agentAction\\.${id}':`, 'g'))].length
}

describe('the agent action vocabulary', () => {
  it('finds the actions the matrix publishes', () => {
    expect(actionsInMatrix()).toContain('improve_clarity')
    expect(actionsInMatrix()).toHaveLength(14)
  })

  /**
   * A missing instruction is the dangerous one: the action still runs and asks
   * the model the generic fallback, so the button lies about what it did.
   */
  it('gives every published action its own instruction', () => {
    const missing = actionsInMatrix().filter((id) => !actionsInPrompt().includes(id))

    expect(missing, 'actions with no instruction of their own').toEqual([])
  })

  /** Both locales, because a half-translated button is a bare id in the other. */
  it('labels every published action in both locales', () => {
    const unlabelled = actionsInMatrix().filter((id) => labelledIn(id) !== 2)

    expect(unlabelled, 'actions without a label in each locale').toEqual([])
  })

  /** And nothing is instructed that is not offered, so dead arms get noticed. */
  it('instructs nothing the matrix does not publish', () => {
    const orphans = actionsInPrompt().filter((id) => !actionsInMatrix().includes(id))

    expect(orphans, 'instructions for actions that do not exist').toEqual([])
  })
})
