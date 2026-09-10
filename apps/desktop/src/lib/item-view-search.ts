export type HighlightSegment = {
  text: string
  isMatch: boolean
}

type FtsSearchKeyEvent = Pick<KeyboardEvent, 'key' | 'preventDefault'>

export type FtsSearchControllerOptions = {
  debounceMs?: number
  getQuery: () => string
  setQuery: (query: string) => void
  reset: () => void
  search: (query: string) => void | Promise<void>
}

export class FtsSearchController {
  private timer: ReturnType<typeof setTimeout> | null = null
  private readonly debounceMs: number
  private readonly getQuery: () => string
  private readonly setQuery: (query: string) => void
  private readonly reset: () => void
  private readonly search: (query: string) => void | Promise<void>

  constructor({ debounceMs = 250, getQuery, setQuery, reset, search }: FtsSearchControllerOptions) {
    this.debounceMs = debounceMs
    this.getQuery = getQuery
    this.setQuery = setQuery
    this.reset = reset
    this.search = search
  }

  handleInput(value: string) {
    this.setQuery(value)
    this.cancel()

    if (!value.trim()) {
      this.reset()
      return
    }

    this.timer = setTimeout(() => {
      this.timer = null
      void this.search(value)
    }, this.debounceMs)
  }

  handleKeydown(event: FtsSearchKeyEvent) {
    if (event.key === 'Enter') {
      event.preventDefault()
      this.cancel()
      void this.search(this.getQuery())
      return
    }

    if (event.key === 'Escape') {
      event.preventDefault()
      this.cancel()
      this.setQuery('')
      this.reset()
    }
  }

  cancel() {
    if (this.timer) {
      clearTimeout(this.timer)
      this.timer = null
    }
  }
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

export function getFtsTerms(rawQuery: string): string[] {
  if (!rawQuery.trim()) return []

  const noOperators = rawQuery.replace(/\b(AND|OR|NOT|NEAR)\b/gi, ' ')
  const terms = noOperators
    .split(/\s+/)
    .map((token) => token.replace(/[()"\-*^:,./\\]/g, '').trim())
    .filter((token) => token.length > 0)

  return Array.from(new Set(terms.map((token) => token.toLocaleLowerCase())))
}

export function splitHighlightedSegments(text: string, rawQuery: string): HighlightSegment[] {
  const terms = getFtsTerms(rawQuery)
  if (terms.length === 0 || !text) return [{ text, isMatch: false }]

  const pattern = terms
    .slice()
    .sort((a, b) => b.length - a.length)
    .map((term) => escapeRegExp(term))
    .join('|')

  if (!pattern) return [{ text, isMatch: false }]

  const regex = new RegExp(pattern, 'gi')
  const segments: HighlightSegment[] = []
  let lastIndex = 0

  for (const match of text.matchAll(regex)) {
    const index = match.index ?? 0
    const value = match[0] ?? ''
    if (index > lastIndex) {
      segments.push({ text: text.slice(lastIndex, index), isMatch: false })
    }
    if (value) {
      segments.push({ text: value, isMatch: true })
    }
    lastIndex = index + value.length
  }

  if (lastIndex < text.length) {
    segments.push({ text: text.slice(lastIndex), isMatch: false })
  }

  return segments.length > 0 ? segments : [{ text, isMatch: false }]
}
