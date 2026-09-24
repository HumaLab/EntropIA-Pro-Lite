export interface BreadcrumbEntry {
  label: string
  /** Index into the original `crumbs` array this entry maps to a click on,
   *  or `-1` for the synthetic, non-clickable `…` placeholder. */
  index: number
  collapsed: boolean
}

/**
 * Collapse the middle of an overflowing breadcrumb into a single `…`,
 * keeping the first and last crumb visible and clickable (spec, Responsive:
 * "when the breadcrumb overflows, middle crumbs collapse into …").
 */
export function collapseBreadcrumb(crumbs: string[], maxVisible: number): BreadcrumbEntry[] {
  if (crumbs.length <= maxVisible || crumbs.length <= 2) {
    return crumbs.map((label, index) => ({ label, index, collapsed: false }))
  }
  return [
    { label: crumbs[0]!, index: 0, collapsed: false },
    { label: '…', index: -1, collapsed: true },
    { label: crumbs[crumbs.length - 1]!, index: crumbs.length - 1, collapsed: false },
  ]
}
