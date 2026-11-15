import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

const SOURCE = readFileSync(resolve(import.meta.dirname, 'InvestigationView.svelte'), 'utf-8')
const MARKUP = SOURCE.slice(0, SOURCE.indexOf('<style'))
  .replace(/<script[\s\S]*?<\/script>/g, '')
  .replace(/<!--[\s\S]*?-->/g, '')
const STYLES = SOURCE.slice(SOURCE.indexOf('<style')).replace(/\/\*[\s\S]*?\*\//g, '')

describe('InvestigationView uses the system form controls', () => {
  it('draws every multi-line field with Textarea, never a raw textarea (R2)', () => {
    expect(MARKUP).not.toMatch(/<textarea[\s>]/)
    expect(MARKUP.match(/<Textarea[\s>]/g)).toHaveLength(8)
    expect(STYLES).not.toMatch(/\.investigation-round__input/)
  })

  it('draws the section title field with Input, never a raw input', () => {
    expect(MARKUP).not.toMatch(/<input[\s>]/)
    expect(MARKUP).toMatch(/<Input[\s>]/)
  })

  it('shows the technical detail with Disclosure, never details/summary (N11)', () => {
    expect(MARKUP).not.toMatch(/<details[\s>]|<summary[\s>]/)
    expect(MARKUP).toMatch(/<Disclosure[\s>]/)
    expect(STYLES).not.toMatch(/\bsummary\b/)
  })
})

describe('InvestigationView styles stay on the token scales', () => {
  it('sizes text with font-size tokens, never em/px/rem literals (F5)', () => {
    expect(STYLES).not.toMatch(/font-size:\s*[\d.]+(em|px|rem)/)
  })

  it('takes line-height and letter-spacing from their tokens', () => {
    expect(STYLES).not.toMatch(/line-height:\s*[\d.]/)
    expect(STYLES).not.toMatch(/letter-spacing:\s*[\d.-]/)
  })

  it('times transitions with the motion tokens and honours reduced motion', () => {
    expect(STYLES).not.toMatch(/\d+m?s\s+ease/)
    expect(STYLES).toMatch(/@media \(prefers-reduced-motion: reduce\)/)
  })

  it('spaces inline code with spacing tokens, not em literals', () => {
    expect(STYLES).not.toMatch(/padding:[^;]*\d+(\.\d+)?em/)
  })
})
