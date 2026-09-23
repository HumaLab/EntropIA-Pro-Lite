import { describe, expect, it } from 'vitest'
import { formatRelativeDate } from './formatRelativeDate'

describe('formatRelativeDate', () => {
  it('formats a timestamp from days ago', () => {
    const now = Date.now()
    expect(formatRelativeDate(now - 2 * 24 * 60 * 60 * 1000)).toBe('2 days ago')
  })

  it('formats a timestamp from hours ago', () => {
    const now = Date.now()
    expect(formatRelativeDate(now - 3 * 60 * 60 * 1000)).toBe('3 hours ago')
  })

  it('formats a timestamp from minutes ago', () => {
    const now = Date.now()
    expect(formatRelativeDate(now - 5 * 60 * 1000)).toBe('5 minutes ago')
  })

  it('formats a timestamp under a minute old as now', () => {
    const now = Date.now()
    expect(formatRelativeDate(now - 10 * 1000)).toBe('now')
  })

  it('honors the given locale', () => {
    const now = Date.now()
    expect(formatRelativeDate(now - 2 * 24 * 60 * 60 * 1000, 'es')).toBe('hace 2 días')
  })

  it('defaults the locale to English when not given', () => {
    const now = Date.now()
    expect(formatRelativeDate(now - 24 * 60 * 60 * 1000)).toBe('1 day ago')
  })
})
