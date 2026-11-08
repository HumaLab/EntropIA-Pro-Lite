import { describe, expect, it } from 'vitest'
import { readFileSync, readdirSync } from 'node:fs'
import { join, resolve } from 'node:path'

// Every numeric field's label reserves the same two-line slot, so the − value +
// controls of a row line up whatever the label length (one shared rule in app.css).
const src = resolve(import.meta.dirname, '..')

function svelteFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name)
    if (entry.isDirectory()) return svelteFiles(path)
    return entry.name.endsWith('.svelte') ? [path] : []
  })
}

describe('numeric field labels', () => {
  it('share one clamped, fixed-height label rule', () => {
    const css = readFileSync(join(src, 'app.css'), 'utf-8')
    const rule = css.slice(css.indexOf('.number-field-label {'))
    expect(rule).toContain('-webkit-line-clamp: 2')
    expect(rule).toContain('line-clamp: 2')
    expect(rule).toContain('min-height: calc(2 * var(--number-field-label-line))')
    expect(rule).toContain('max-height: calc(2 * var(--number-field-label-line))')
  })

  it('label every NumberField with that rule and leave no native number input', () => {
    const users = svelteFiles(src).filter((file) =>
      readFileSync(file, 'utf-8').includes('<NumberField')
    )
    expect(users.length).toBeGreaterThanOrEqual(5)
    for (const file of users) {
      const text = readFileSync(file, 'utf-8')
      const fields = text.split('<NumberField').length - 1
      const labels = text.split('number-field-label').length - 1
      expect(labels, file).toBe(fields)
    }
    for (const file of svelteFiles(src))
      expect(readFileSync(file, 'utf-8'), file).not.toContain('type="number"')
  })
})
