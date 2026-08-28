import { afterEach, describe, expect, it, vi } from 'vitest'
import { FtsSearchController, getFtsTerms, splitHighlightedSegments } from './item-view-search'

describe('item view search helpers', () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  it('extracts unique lower-case FTS terms while removing operators and punctuation', () => {
    expect(getFtsTerms('Acta AND (OCR) NOT pdf:* acta')).toEqual(['acta', 'ocr', 'pdf'])
  })

  it('returns a single non-match segment for empty queries or empty text', () => {
    expect(splitHighlightedSegments('Acta secreta', '')).toEqual([
      { text: 'Acta secreta', isMatch: false },
    ])
    expect(splitHighlightedSegments('', 'acta')).toEqual([{ text: '', isMatch: false }])
  })

  it('splits text into highlighted and non-highlighted segments case-insensitively', () => {
    expect(splitHighlightedSegments('Acta secreta de archivo', 'secreta archivo')).toEqual([
      { text: 'Acta ', isMatch: false },
      { text: 'secreta', isMatch: true },
      { text: ' de ', isMatch: false },
      { text: 'archivo', isMatch: true },
    ])
  })

  it('prefers longer terms before shorter overlapping terms', () => {
    expect(splitHighlightedSegments('metadata meta', 'meta metadata')).toEqual([
      { text: 'metadata', isMatch: true },
      { text: ' ', isMatch: false },
      { text: 'meta', isMatch: true },
    ])
  })

  it('debounces FTS input and searches only the latest query', () => {
    vi.useFakeTimers()
    let query = ''
    const search = vi.fn()
    const reset = vi.fn()
    const controller = new FtsSearchController({
      getQuery: () => query,
      setQuery: (value) => {
        query = value
      },
      reset,
      search,
    })

    controller.handleInput('ca')
    vi.advanceTimersByTime(200)
    controller.handleInput('cabildo')
    vi.advanceTimersByTime(249)

    expect(search).not.toHaveBeenCalled()

    vi.advanceTimersByTime(1)

    expect(search).toHaveBeenCalledTimes(1)
    expect(search).toHaveBeenCalledWith('cabildo')
    expect(reset).not.toHaveBeenCalled()
  })

  it('runs an immediate FTS search on Enter and cancels the pending debounce', () => {
    vi.useFakeTimers()
    let query = ''
    const search = vi.fn()
    const preventDefault = vi.fn()
    const controller = new FtsSearchController({
      getQuery: () => query,
      setQuery: (value) => {
        query = value
      },
      reset: vi.fn(),
      search,
    })

    controller.handleInput('cabildo')
    controller.handleKeydown({ key: 'Enter', preventDefault })
    vi.advanceTimersByTime(250)

    expect(preventDefault).toHaveBeenCalledTimes(1)
    expect(search).toHaveBeenCalledTimes(1)
    expect(search).toHaveBeenCalledWith('cabildo')
  })

  it('clears a pending FTS query without starting an empty search', () => {
    vi.useFakeTimers()
    let query = ''
    const setQuery = vi.fn((value: string) => {
      query = value
    })
    const reset = vi.fn()
    const search = vi.fn()
    const controller = new FtsSearchController({
      getQuery: () => query,
      setQuery,
      reset,
      search,
    })

    controller.handleInput('alpha')
    controller.handleInput('')
    vi.advanceTimersByTime(250)

    expect(setQuery).toHaveBeenLastCalledWith('')
    expect(reset).toHaveBeenCalledOnce()
    expect(search).not.toHaveBeenCalled()
  })

  it('resets blank input and Escape while cancelling pending searches', () => {
    vi.useFakeTimers()
    let query = ''
    const search = vi.fn()
    const reset = vi.fn()
    const preventDefault = vi.fn()
    const controller = new FtsSearchController({
      getQuery: () => query,
      setQuery: (value) => {
        query = value
      },
      reset,
      search,
    })

    controller.handleInput('cabildo')
    controller.handleInput('   ')
    vi.advanceTimersByTime(250)

    expect(query).toBe('   ')
    expect(reset).toHaveBeenCalledTimes(1)
    expect(search).not.toHaveBeenCalled()

    controller.handleInput('acta')
    controller.handleKeydown({ key: 'Escape', preventDefault })
    vi.advanceTimersByTime(250)

    expect(preventDefault).toHaveBeenCalledTimes(1)
    expect(query).toBe('')
    expect(reset).toHaveBeenCalledTimes(2)
    expect(search).not.toHaveBeenCalled()
  })

  it('cancels pending FTS search cleanup', () => {
    vi.useFakeTimers()
    const search = vi.fn()
    const controller = new FtsSearchController({
      getQuery: () => 'cabildo',
      setQuery: vi.fn(),
      reset: vi.fn(),
      search,
    })

    controller.handleInput('cabildo')
    controller.cancel()
    vi.advanceTimersByTime(250)

    expect(search).not.toHaveBeenCalled()
  })
})
