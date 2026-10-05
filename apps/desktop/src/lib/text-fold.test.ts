import { describe, expect, it } from 'vitest'
import { foldText, matchesQuery } from './text-fold'

describe('foldText', () => {
  it('drops accents and case', () => {
    expect(foldText('Producción ÑÚÑEZ')).toBe('produccion nunez')
  })
})

describe('matchesQuery', () => {
  const title = 'La producción del espacio'

  it('ignores accents and case, always', () => {
    expect(matchesQuery(title, 'LA PRODUCCION del espacio', false)).toBe(true)
    expect(matchesQuery('La produccion del espacio', 'producción', false)).toBe(true)
  })

  it('needs the exact words when approximate matching is off', () => {
    expect(matchesQuery(title, 'La produción del espasio', false)).toBe(false)
  })

  it('tolerates typos in each word when approximate matching is on', () => {
    expect(matchesQuery(title, 'La produción del espasio', true)).toBe(true)
  })

  it('still rejects words that are not close', () => {
    expect(matchesQuery(title, 'la revolucion del espacio', true)).toBe(false)
  })

  it('does not blur short words', () => {
    expect(matchesQuery('Ruiz 2015', 'ruis', true)).toBe(false)
  })

  it('matches everything for an empty query', () => {
    expect(matchesQuery(title, '  ', true)).toBe(true)
  })
})
