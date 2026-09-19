import { describe, it, expect } from 'vitest'
import { editDistance, maxEditsFor, normalizeTerm, pickVariants } from './fuzzy'

describe('editDistance', () => {
  it('counts substitutions, insertions and deletions', () => {
    expect(editDistance('sindicato', 'sindigato', 2)).toBe(1)
    expect(editDistance('obrero', 'obreo', 2)).toBe(1)
    expect(editDistance('ortiz', 'ortizs', 2)).toBe(1)
  })

  it('counts a transposition as one edit, as OCR and typing both produce it', () => {
    expect(editDistance('trabajadores', 'trabaajdores', 2)).toBe(1)
  })

  it('gives up past the bound instead of computing the full distance', () => {
    expect(editDistance('sindicato', 'gobernador', 2)).toBe(3)
  })
})

describe('maxEditsFor', () => {
  it('never expands short terms: one edit turns them into other words', () => {
    expect(maxEditsFor('mar')).toBe(0)
    expect(maxEditsFor('otiz')).toBe(0)
  })

  it('allows one edit for mid-length terms and two for long ones', () => {
    expect(maxEditsFor('ortiz')).toBe(1)
    expect(maxEditsFor('obrero')).toBe(1)
    expect(maxEditsFor('sindicato')).toBe(2)
  })

  it('never expands anything with a digit: dates and folios are exact', () => {
    expect(maxEditsFor('1946')).toBe(0)
    expect(maxEditsFor('fs12345')).toBe(0)
  })
})

describe('normalizeTerm', () => {
  it('folds case and accents the way the index tokenizer does', () => {
    expect(normalizeTerm('ZÁRATE')).toBe('zarate')
  })
})

// Document frequencies below are taken from a real EntropIA corpus.
describe('pickVariants', () => {
  const vocab = new Map<string, number>([
    ['sindicato', 269],
    ['sindigato', 3],
    ['sinicato', 2],
    ['sindicatos', 119],
    ['trabajadores', 284],
    ['trabajdores', 1],
    ['obrero', 240],
    ['obreros', 266],
    ['obreo', 1],
    ['ortiz', 23],
    ['otiz', 1],
    ['mar', 50],
    ['mas', 400],
  ])

  it('finds the rare misreadings of a word the corpus mostly spells right', () => {
    expect(pickVariants('sindicato', vocab)).toEqual(['sindigato', 'sinicato'])
    expect(pickVariants('ortiz', vocab)).toEqual(['otiz'])
  })

  it('leaves out real words of similar weight: those are morphology, not OCR', () => {
    expect(pickVariants('obrero', vocab)).toEqual(['obreo'])
    expect(pickVariants('sindicato', vocab)).not.toContain('sindicatos')
  })

  it("corrects the writer's typo toward the spelling the corpus supports", () => {
    expect(pickVariants('trabajdores', vocab)).toEqual(['trabajadores'])
    expect(pickVariants('sindicto', vocab)).toEqual(['sindicato'])
  })

  it('expands nothing for short terms', () => {
    expect(pickVariants('mar', vocab)).toEqual([])
  })

  it('caps how many variants one term can add', () => {
    const noisy = new Map<string, number>()
    for (const letter of 'abcdefghijklmnpqrstuvwxyz') noisy.set(`sindicat${letter}`, 1)
    noisy.set('sindicato', 1000)
    expect(pickVariants('sindicato', noisy).length).toBe(8)
  })
})
