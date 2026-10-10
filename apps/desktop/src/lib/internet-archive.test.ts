import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  iaBeginFileImport,
  iaCancelImport,
  iaConfirmFileImported,
  iaFailFileImport,
  iaFinishReconcile,
  iaGetImportStatus,
  iaGetItem,
  iaListCollection,
  iaListJobs,
  iaListReconcile,
  iaListVerifiedFiles,
  iaPrepareImport,
  iaResolveIdentifier,
  iaSearch,
  iaStartImport,
  onIaJobUpdated,
  parseIaError,
  type IaSelection,
} from './internet-archive'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn() }))

beforeEach(() => {
  vi.mocked(invoke).mockReset()
  vi.mocked(listen).mockReset()
})

describe('read commands', () => {
  it('resolve, search, collection page and item metadata name identifiers, never URLs', async () => {
    vi.mocked(invoke).mockResolvedValue('some-id')
    await expect(iaResolveIdentifier('https://archive.org/details/some-id')).resolves.toBe(
      'some-id'
    )
    expect(invoke).toHaveBeenCalledWith('ia_resolve_identifier', {
      input: 'https://archive.org/details/some-id',
    })

    vi.mocked(invoke).mockResolvedValue({ numFound: 1, items: [] })
    await iaSearch({ text: 'chaplin', textsOnly: true }, 2, 25)
    expect(invoke).toHaveBeenCalledWith('ia_search', {
      query: { text: 'chaplin', textsOnly: true },
      page: 2,
      rows: 25,
    })

    await iaListCollection('opensource', 1, 50)
    expect(invoke).toHaveBeenCalledWith('ia_list_collection', {
      collection: 'opensource',
      page: 1,
      rows: 50,
    })

    await iaGetItem('some-id')
    expect(invoke).toHaveBeenCalledWith('ia_get_item', { identifier: 'some-id' })
  })
})

describe('prepare and run commands', () => {
  const selections: IaSelection[] = [
    { identifier: 'some-id', fileNames: ['some-id_text.pdf'] },
  ]

  it('prepare names the collection and the selections', async () => {
    vi.mocked(invoke).mockResolvedValue({ jobId: 'job-1', summary: null, rejected: [] })
    await iaPrepareImport('col-1', selections)
    expect(invoke).toHaveBeenCalledWith('ia_prepare_import', {
      collectionId: 'col-1',
      selections,
    })
  })

  it('start, cancel and status name the job by id', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)
    await iaStartImport('job-1')
    expect(invoke).toHaveBeenCalledWith('ia_start_import', { jobId: 'job-1' })

    await iaCancelImport('job-1')
    expect(invoke).toHaveBeenCalledWith('ia_cancel_import', { jobId: 'job-1' })

    vi.mocked(invoke).mockResolvedValue({ id: 'job-1' })
    await iaGetImportStatus('job-1')
    expect(invoke).toHaveBeenCalledWith('ia_get_import_status', { jobId: 'job-1' })
  })

  it('list jobs and verified files', async () => {
    vi.mocked(invoke).mockResolvedValue([])
    await iaListJobs()
    expect(invoke).toHaveBeenCalledWith('ia_list_jobs')

    await iaListVerifiedFiles('job-1')
    expect(invoke).toHaveBeenCalledWith('ia_list_verified_files', { jobId: 'job-1' })
  })
})

describe('file handoff commands', () => {
  it('begin, confirm and fail name the job and the file index', async () => {
    vi.mocked(invoke).mockResolvedValue({ action: 'import' })
    await iaBeginFileImport('job-1', 0)
    expect(invoke).toHaveBeenCalledWith('ia_begin_file_import', {
      jobId: 'job-1',
      index: 0,
    })

    vi.mocked(invoke).mockResolvedValue(undefined)
    await iaConfirmFileImported('job-1', 0, 'item-1')
    expect(invoke).toHaveBeenCalledWith('ia_confirm_file_imported', {
      jobId: 'job-1',
      index: 0,
      itemId: 'item-1',
    })

    await iaFailFileImport('job-1', 0)
    expect(invoke).toHaveBeenCalledWith('ia_fail_file_import', {
      jobId: 'job-1',
      index: 0,
    })
  })

  it('reconcile list takes no arguments, finish names the leftover', async () => {
    vi.mocked(invoke).mockResolvedValue([])
    await iaListReconcile()
    expect(invoke).toHaveBeenCalledWith('ia_list_reconcile')

    vi.mocked(invoke).mockResolvedValue(undefined)
    await iaFinishReconcile('job-1', 2)
    expect(invoke).toHaveBeenCalledWith('ia_finish_reconcile', {
      jobId: 'job-1',
      index: 2,
    })
  })
})

describe('parseIaError', () => {
  it('reads every known code the backend can produce and keeps the message', () => {
    const cases: Array<[string, string, string]> = [
      ['invalid_query: empty search', 'invalid_query', 'empty search'],
      ['not_found: import job not found', 'not_found', 'import job not found'],
      ['blocked: robots refuse', 'blocked', 'robots refuse'],
      ['too_many_redirects: loop', 'too_many_redirects', 'loop'],
      ['too_large: over the cap', 'too_large', 'over the cap'],
      ['network: connection reset', 'network', 'connection reset'],
      ['decode: bad json', 'decode', 'bad json'],
      ['already_exists: already exists', 'already_exists', 'already exists'],
      ['io: local storage error', 'io', 'local storage error'],
      ['verify_failed: bad header', 'verify_failed', 'bad header'],
      ['too_many_pages: 600 > 500', 'too_many_pages', '600 > 500'],
      ['invalid_request: duplicate file', 'invalid_request', 'duplicate file'],
      ['collection_not_found: "col-9"', 'collection_not_found', '"col-9"'],
      ['nothing_to_import: no requested file', 'nothing_to_import', 'no requested file'],
      ['db: no such table: items', 'db', 'no such table: items'],
      ['job_running: another import', 'job_running', 'another import'],
      ['invalid_state: cannot start', 'invalid_state', 'cannot start'],
      ['staging_missing: staging file gone', 'staging_missing', 'staging file gone'],
      ['item_not_found: imported item not found', 'item_not_found', 'imported item not found'],
      ['wrong_collection: another collection', 'wrong_collection', 'another collection'],
      [
        'provenance_mismatch: different provenance',
        'provenance_mismatch',
        'different provenance',
      ],
      ['incomplete_import: item is not complete', 'incomplete_import', 'item is not complete'],
      ['partial_remains: partial still there', 'partial_remains', 'partial still there'],
      ['network: down', 'network', 'down'],
    ]
    for (const [reason, code, message] of cases) {
      expect(parseIaError(reason), reason).toEqual({ code, message })
    }
    expect(parseIaError(new Error('not_found: gone'))).toEqual({
      code: 'not_found',
      message: 'gone',
    })
  })

  it('keeps the status number of the http_<status> family', () => {
    expect(parseIaError('http_429: http error 429 (retry after 60s)')).toEqual({
      code: 'http_429',
      status: 429,
      message: 'http error 429 (retry after 60s)',
    })
    expect(parseIaError('http_500: http error 500')).toEqual({
      code: 'http_500',
      status: 500,
      message: 'http error 500',
    })
  })

  it('treats unknown codes as unknown and keeps the whole message', () => {
    expect(parseIaError('bogus: something broke')).toEqual({
      code: 'unknown',
      message: 'bogus: something broke',
    })
    expect(parseIaError('Command not found')).toEqual({
      code: 'unknown',
      message: 'Command not found',
    })
  })

  it('keeps messages that carry extra colons', () => {
    expect(parseIaError('db: no such table: items')).toEqual({
      code: 'db',
      message: 'no such table: items',
    })
    expect(parseIaError('invalid_request: expected 1..=100 selections, got 0: extra')).toEqual({
      code: 'invalid_request',
      message: 'expected 1..=100 selections, got 0: extra',
    })
  })

  it('tolerates non-string reasons', () => {
    expect(parseIaError(new Error('network: down'))).toEqual({
      code: 'network',
      message: 'down',
    })
    expect(parseIaError(42)).toEqual({ code: 'unknown', message: '42' })
    expect(parseIaError(null)).toEqual({ code: 'unknown', message: 'null' })
    expect(parseIaError(undefined)).toEqual({ code: 'unknown', message: 'undefined' })
  })
})

describe('onIaJobUpdated', () => {
  it('listens on ia://job-updated, hands the job id over and returns the unlisten function', async () => {
    const unlisten = vi.fn()
    vi.mocked(listen).mockImplementation(() => Promise.resolve(unlisten))
    const seen: string[] = []
    const stop = await onIaJobUpdated((jobId) => {
      seen.push(jobId)
    })
    expect(listen).toHaveBeenCalledWith('ia://job-updated', expect.any(Function))
    const handler = vi.mocked(listen).mock.calls[0]?.[1] as unknown as
      | ((event: { payload: { jobId: string } }) => void)
      | undefined
    expect(handler).toBeTypeOf('function')
    handler?.({ payload: { jobId: 'job-9' } })
    expect(seen).toEqual(['job-9'])
    expect(stop).toBe(unlisten)
  })
})
