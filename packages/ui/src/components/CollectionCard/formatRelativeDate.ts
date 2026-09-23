/**
 * Formats a timestamp as a relative time string ("2 days ago", "hace 2 días"),
 * bucketed to the coarsest whole unit (days, then hours, then minutes) with
 * anything under a minute reported as "now". Extracted from CollectionCard so
 * other views (the home overview's recent-activity list) can reuse the exact
 * same formatting without depending on the card component.
 */
export function formatRelativeDate(timestamp: number, locale = 'en'): string {
  const now = Date.now()
  const diff = now - timestamp
  const seconds = Math.floor(diff / 1000)
  const minutes = Math.floor(seconds / 60)
  const hours = Math.floor(minutes / 60)
  const days = Math.floor(hours / 24)
  const formatter = new Intl.RelativeTimeFormat(locale, { numeric: 'always' })

  if (days > 0) return formatter.format(-days, 'day')
  if (hours > 0) return formatter.format(-hours, 'hour')
  if (minutes > 0) return formatter.format(-minutes, 'minute')
  return new Intl.RelativeTimeFormat(locale, { numeric: 'auto' }).format(0, 'second')
}
