import type { I18nKey } from './i18n'

/**
 * The word on a job badge. `idle` only says nothing runs right now, so the
 * resting word comes from the stored result: a document indexed in an earlier
 * session reads "done", not "idle" (finding 8).
 */
export function jobStatusLabelKey(status: string, hasStoredResult = false): I18nKey {
  switch (status) {
    case 'pending':
      return 'item.jobStatus.pending'
    case 'running':
      return 'item.jobStatus.running'
    case 'done':
      return 'item.jobStatus.done'
    case 'error':
      return 'item.jobStatus.error'
    default:
      return hasStoredResult ? 'item.jobStatus.stored' : 'item.jobStatus.idle'
  }
}
