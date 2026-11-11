import { describe, expect, it } from 'vitest'
import {
  stripHonorific,
  suggestEntityAliases,
  type AliasSuggestionInput,
} from './entity-alias-suggestions'

function input(
  values: Array<{ value: string; mentions: number }>,
  aliases: string[] = [],
  dismissed: string[] = []
): AliasSuggestionInput {
  return {
    values: values.map(({ value, mentions }) => ({ value, mentions })),
    aliases: aliases.map((alias) => ({
      aliasKey: alias.trim().replace(/\s+/g, ' ').toLowerCase(),
      canonical: 'Carmela Cuevas',
    })),
    dismissedKeys: new Set(dismissed.map((key) => key.trim().replace(/\s+/g, ' ').toLowerCase())),
  }
}

describe('stripHonorific', () => {
  it.each([
    ['Mdme. Cuevas', 'Cuevas'],
    ['Mdme Cuevas', 'Cuevas'],
    ['MDME. CUEVAS', 'CUEVAS'],
    ['Mme. Cuevas', 'Cuevas'],
    ['Madame Cuevas', 'Cuevas'],
    ['Sra. Gómez', 'Gómez'],
    ['Sra Gómez', 'Gómez'],
    ['Sr. Gómez', 'Gómez'],
    ['Srta. Gómez', 'Gómez'],
    ['Dr. Pérez', 'Pérez'],
    ['Dra. Pérez', 'Pérez'],
    ['Dra Pérez', 'Pérez'],
    ['Don Pedro', 'Pedro'],
    ['Doña María', 'María'],
    ['Dña. María', 'María'],
    ['D. Juan', 'Juan'],
    ['Mr. Smith', 'Smith'],
    ['Mrs. Smith', 'Smith'],
    ['Ms Smith', 'Smith'],
  ])('strips the honorific from %s', (value, expected) => {
    expect(stripHonorific(value)).toBe(expected)
  })

  it('leaves names without honorifics untouched', () => {
    expect(stripHonorific('Carmela Cuevas')).toBe('Carmela Cuevas')
    expect(stripHonorific('Cuevas')).toBe('Cuevas')
    expect(stripHonorific('C. Cuevas')).toBe('C. Cuevas')
  })
})

describe('suggestEntityAliases', () => {
  it('groups a bare honorific + surname with the fuller name', () => {
    const suggestions = suggestEntityAliases(
      input([
        { value: 'Carmela Cuevas', mentions: 5 },
        { value: 'Mdme. Cuevas', mentions: 3 },
      ])
    )

    expect(suggestions).toHaveLength(1)
    expect(suggestions[0]!.canonical).toBe('Carmela Cuevas')
    expect(suggestions[0]!.variants).toEqual(['Mdme. Cuevas'])
  })

  it('groups an initial with the fuller name when they share the surname', () => {
    const suggestions = suggestEntityAliases(
      input([
        { value: 'Carmela Cuevas', mentions: 4 },
        { value: 'C. Cuevas', mentions: 2 },
      ])
    )

    expect(suggestions).toHaveLength(1)
    expect(suggestions[0]!.canonical).toBe('Carmela Cuevas')
    expect(suggestions[0]!.variants).toEqual(['C. Cuevas'])
  })

  it('picks the fullest form as canonical, ties broken by most mentions', () => {
    const suggestions = suggestEntityAliases(
      input([
        { value: 'Carmela Cuevas', mentions: 2 },
        { value: 'Carmela María Cuevas', mentions: 1 },
        { value: 'Mdme. Cuevas', mentions: 9 },
      ])
    )

    expect(suggestions).toHaveLength(1)
    expect(suggestions[0]!.canonical).toBe('Carmela María Cuevas')
    expect(suggestions[0]!.variants).toEqual(['Carmela Cuevas', 'Mdme. Cuevas'])
  })

  it('does not attach short forms when two different fuller names share a surname', () => {
    const suggestions = suggestEntityAliases(
      input([
        { value: 'Carmela Cuevas', mentions: 5 },
        { value: 'Juan Cuevas', mentions: 4 },
        { value: 'Cuevas', mentions: 6 },
        { value: 'C. Cuevas', mentions: 2 },
        { value: 'Mdme. Cuevas', mentions: 3 },
      ])
    )

    expect(suggestions).toEqual([])
  })

  it('suggests a bare surname alone only when exactly one fuller name has it', () => {
    const single = suggestEntityAliases(
      input([
        { value: 'Carmela Cuevas', mentions: 5 },
        { value: 'Cuevas', mentions: 2 },
      ])
    )
    expect(single).toHaveLength(1)
    expect(single[0]!.variants).toEqual(['Cuevas'])
  })

  it('proposes nothing without a fuller name to anchor the group', () => {
    expect(
      suggestEntityAliases(
        input([
          { value: 'Mdme. Cuevas', mentions: 2 },
          { value: 'C. Cuevas', mentions: 1 },
        ])
      )
    ).toEqual([])
  })

  it('skips variants that already have an alias', () => {
    const suggestions = suggestEntityAliases(
      input(
        [
          { value: 'Carmela Cuevas', mentions: 5 },
          { value: 'Mdme. Cuevas', mentions: 3 },
          { value: 'C. Cuevas', mentions: 2 },
        ],
        ['Mdme. Cuevas']
      )
    )

    expect(suggestions).toHaveLength(1)
    expect(suggestions[0]!.variants).toEqual(['C. Cuevas'])
  })

  it('drops the suggestion when every variant already has an alias', () => {
    expect(
      suggestEntityAliases(
        input(
          [
            { value: 'Carmela Cuevas', mentions: 5 },
            { value: 'Mdme. Cuevas', mentions: 3 },
          ],
          ['Mdme. Cuevas']
        )
      )
    ).toEqual([])
  })

  it('skips suggestions the owner dismissed in this session', () => {
    expect(
      suggestEntityAliases(
        input(
          [
            { value: 'Carmela Cuevas', mentions: 5 },
            { value: 'Mdme. Cuevas', mentions: 3 },
          ],
          [],
          ['Carmela Cuevas']
        )
      )
    ).toEqual([])
  })

  it('orders by most mentions first, then alphabetically', () => {
    const suggestions = suggestEntityAliases(
      input([
        { value: 'Ana Gómez', mentions: 2 },
        { value: 'Sra. Gómez', mentions: 1 },
        { value: 'Juana Pérez', mentions: 4 },
        { value: 'Sra. Pérez', mentions: 4 },
        { value: 'Carmela Cuevas', mentions: 1 },
        { value: 'Mdme. Cuevas', mentions: 1 },
      ])
    )

    expect(suggestions.map((suggestion) => suggestion.canonical)).toEqual([
      'Juana Pérez',
      'Ana Gómez',
      'Carmela Cuevas',
    ])
  })
})
