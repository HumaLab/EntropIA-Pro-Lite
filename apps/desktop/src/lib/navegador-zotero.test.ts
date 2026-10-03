import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import {
  describeCopy,
  hasPending,
  isPending,
  navegadorZoteroCancel,
  navegadorZoteroLaunch,
  navegadorZoteroLibraries,
  navegadorZoteroOpenItem,
  navegadorZoteroStatus,
  describeStatus,
  navegadorZoteroList,
  navegadorZoteroRequest,
  navegadorZoteroRun,
  parseZoteroError,
  type ZoteroCopy,
  type ZoteroCopyDetail,
} from './navegador-zotero'

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn() }))

const copy = (patch: Partial<ZoteroCopy> = {}): ZoteroCopy => ({
  id: 'k1',
  sourceId: 's1',
  captureId: null,
  libraryType: 'user',
  libraryId: '0',
  libraryName: null,
  state: 'queued',
  itemKey: null,
  detail: null,
  errorCode: null,
  errorMessage: null,
  attempts: 0,
  createdAt: 1,
  updatedAt: 1,
  ...patch,
})

beforeEach(() => vi.mocked(invoke).mockReset())

describe('commands', () => {
  it('asks by id and library, never by path or by title', async () => {
    vi.mocked(invoke).mockResolvedValue(copy())
    await navegadorZoteroRequest('s1', 'c1', {
      libraryType: 'group',
      libraryId: '7',
      libraryName: 'prueba',
    })
    expect(invoke).toHaveBeenCalledWith('navegador_zotero_copy_request', {
      sourceId: 's1',
      captureId: 'c1',
      library: { libraryType: 'group', libraryId: '7', libraryName: 'prueba' },
    })
  })

  it('a copy of the page alone sends no capture', async () => {
    vi.mocked(invoke).mockResolvedValue(copy())
    await navegadorZoteroRequest('s1', null, {
      libraryType: 'user',
      libraryId: '0',
      libraryName: null,
    })
    expect(vi.mocked(invoke).mock.calls[0]![1]).toMatchObject({ captureId: null })
  })

  it('lists, runs, cancels and launches through their own commands', async () => {
    vi.mocked(invoke).mockResolvedValue([])
    await navegadorZoteroList('s1')
    expect(invoke).toHaveBeenLastCalledWith('navegador_zotero_copy_list', { sourceId: 's1' })
    await navegadorZoteroList()
    expect(invoke).toHaveBeenLastCalledWith('navegador_zotero_copy_list', { sourceId: null })
    await navegadorZoteroRun()
    expect(invoke).toHaveBeenLastCalledWith('navegador_zotero_copy_run')
    await navegadorZoteroCancel('k1')
    expect(invoke).toHaveBeenLastCalledWith('navegador_zotero_copy_cancel', { copyId: 'k1' })
    await navegadorZoteroLaunch()
    expect(invoke).toHaveBeenLastCalledWith('navegador_zotero_launch')
  })
})

describe('states', () => {
  it('queued and waiting copies are pending; finished ones are not', () => {
    for (const state of ['queued', 'waiting', 'running'] as const) {
      expect(isPending(copy({ state }))).toBe(true)
    }
    for (const state of ['copied', 'linked', 'failed', 'cancelled'] as const) {
      expect(isPending(copy({ state }))).toBe(false)
    }
    expect(hasPending([copy({ state: 'copied' }), copy({ state: 'waiting' })])).toBe(true)
    expect(hasPending([copy({ state: 'failed' })])).toBe(false)
  })

  it('every state has its own label key and a tone', () => {
    expect(describeCopy(copy({ state: 'queued' })).stateKey).toBe('navegador.zotero.state.queued')
    expect(describeCopy(copy({ state: 'waiting' })).stateKey).toBe('navegador.zotero.state.waiting')
    expect(describeCopy(copy({ state: 'copied' })).tone).toBe('good')
    expect(describeCopy(copy({ state: 'failed' })).tone).toBe('bad')
    expect(describeCopy(copy({ state: 'waiting' })).tone).toBe('pending')
  })

  it('a waiting copy can start Zotero, a failed one can be retried, a pending one cancelled', () => {
    expect(describeCopy(copy({ state: 'waiting' }))).toMatchObject({
      canLaunch: true,
      canCancel: true,
      canRetry: false,
    })
    expect(describeCopy(copy({ state: 'failed' }))).toMatchObject({
      canLaunch: false,
      canCancel: false,
      canRetry: true,
    })
    expect(describeCopy(copy({ state: 'cancelled' })).canRetry).toBe(true)
    expect(describeCopy(copy({ state: 'copied' }))).toMatchObject({
      canLaunch: false,
      canCancel: false,
      canRetry: false,
    })
  })

  it('names the library, the personal one by its own word', () => {
    expect(describeCopy(copy()).libraryKey).toBe('personal')
    expect(describeCopy(copy({ libraryType: 'group', libraryId: '7' })).libraryName).toBe('group/7')
    expect(
      describeCopy(copy({ libraryType: 'group', libraryId: '7', libraryName: 'prueba' }))
        .libraryName
    ).toBe('prueba')
  })

  it('turns what the run found into messages, in a fixed order', () => {
    const linked = describeCopy(
      copy({
        state: 'linked',
        detail: {
          existing: true,
          pdf: 'parent_exists',
          pendingFields: ['accessDate'],
          keptFields: ['title'],
        },
      })
    )
    expect(linked.notes).toEqual([
      'navegador.zotero.note.existing',
      'navegador.zotero.note.pdf.parent_exists',
      'navegador.zotero.note.differs',
    ])
    const created = describeCopy(
      copy({
        state: 'copied',
        detail: { existing: false, pdf: 'attached', pendingFields: [], keptFields: [] },
      })
    )
    expect(created.notes).toEqual(['navegador.zotero.note.pdf.attached'])
    expect(describeCopy(copy({ state: 'copied', detail: null })).notes).toEqual([])
  })

  describe('what the Web API did for an existing item', () => {
    const linked = (web: ZoteroCopyDetail['web']) =>
      describeCopy(
        copy({
          state: 'linked',
          detail: {
            existing: true,
            pdf: 'none',
            pendingFields: [],
            keptFields: [],
            web,
          },
        })
      )

    it('says what was completed and that it reaches Zotero after it syncs', () => {
      const view = linked({ state: 'completed', completed: ['accessDate', 'websiteTitle'] })
      expect(view.notes).toEqual([
        'navegador.zotero.note.existing',
        'navegador.zotero.note.web.completed',
      ])
      expect(view.completedKeys).toEqual([
        'navegador.zotero.field.accessDate',
        'navegador.zotero.field.websiteTitle',
      ])
    })

    it('has a note for each outcome that changed nothing for a reason worth knowing', () => {
      for (const state of [
        'nothing_missing',
        'conflict',
        'failed',
        'invalid_key',
        'other_account',
        'account_unknown',
        'not_synced_yet',
      ] as const) {
        expect(linked({ state, completed: [] }).notes).toContain(
          `navegador.zotero.note.web.${state}`
        )
      }
    })

    it('says nothing extra without a usable key, as before', () => {
      for (const state of ['no_key', 'no_write'] as const) {
        const view = linked({ state, completed: [] })
        expect(view.notes).toEqual(['navegador.zotero.note.existing'])
        expect(view.completedKeys).toEqual([])
      }
    })

    describe('the PDF', () => {
      const withPdf = (pdf: 'attached' | 'already_there' | 'quota' | 'failed') =>
        describeCopy(
          copy({
            state: 'linked',
            detail: {
              existing: true,
              pdf: pdf === 'attached' || pdf === 'already_there' ? pdf : 'parent_exists',
              pendingFields: [],
              keptFields: [],
              web: { state: 'nothing_missing', completed: [], pdf },
            },
          })
        )

      it('says it reaches Zotero after Zotero syncs, files included', () => {
        expect(withPdf('attached').notes).toEqual([
          'navegador.zotero.note.existing',
          'navegador.zotero.note.pdf.attached',
          'navegador.zotero.note.web.nothing_missing',
          'navegador.zotero.note.web.pdf.attached',
        ])
      })

      it('does not claim a sync for a PDF that was already there', () => {
        expect(withPdf('already_there').notes).not.toContain(
          'navegador.zotero.note.web.pdf.attached'
        )
      })

      it('explains a full quota or a failure instead of the old attach-on-create note', () => {
        for (const pdf of ['quota', 'failed'] as const) {
          const notes = withPdf(pdf).notes
          expect(notes).toContain(`navegador.zotero.note.web.pdf.${pdf}`)
          expect(notes).not.toContain('navegador.zotero.note.pdf.parent_exists')
        }
      })
    })

    it('shows where a failure stopped, compactly', () => {
      const view = linked({
        state: 'failed',
        completed: [],
        reason: 'patch:500',
        pdf: 'failed',
        pdfReason: 'upload:500',
      })
      expect(view.reasons).toEqual(['patch:500', 'upload:500'])
      expect(linked({ state: 'completed', completed: [] }).reasons).toEqual([])
      expect(linked(undefined).reasons).toEqual([])
    })

    it('knows what was really added to the existing item', () => {
      expect(linked({ state: 'completed', completed: ['title'] }).added).toEqual({
        fields: true,
        pdf: false,
      })
      expect(linked({ state: 'nothing_missing', completed: [], pdf: 'attached' }).added).toEqual({
        fields: false,
        pdf: true,
      })
      expect(linked({ state: 'failed', completed: [], pdf: 'already_there' }).added).toEqual({
        fields: false,
        pdf: false,
      })
      expect(linked({ state: 'completed', completed: [] }).added.fields).toBe(false)
      expect(linked(undefined).added).toEqual({ fields: false, pdf: false })
    })

    it('reads a row from before the Web API existed', () => {
      expect(linked(undefined).completedKeys).toEqual([])
    })
  })

  it('a pdf that was not asked for says nothing', () => {
    const plain = describeCopy(
      copy({
        state: 'copied',
        detail: { existing: false, pdf: 'none', pendingFields: [], keptFields: [] },
      })
    )
    expect(plain.notes).toEqual([])
  })
})

describe('what the status says about the PDF and completing', () => {
  const base = {
    state: 'present' as const,
    source: 'zotero' as const,
    itemKey: 'K',
    pdf: 'parent_exists' as const,
    pendingFields: [],
    keptFields: [],
  }

  it('names the PDF that would go along, or says none would', () => {
    const named = describeStatus({
      ...base,
      pdfCapture: { id: 'c1', savedAt: '2026-10-02T09:30:00Z' },
      canComplete: false,
    })
    expect(named.pdfCapture).toEqual({ id: 'c1', savedAt: '2026-10-02T09:30:00Z' })
    expect(describeStatus({ ...base, pdfCapture: null, canComplete: false }).pdfCapture).toBeNull()
  })

  it('offers completing only when the backend says there is something to do', () => {
    expect(describeStatus({ ...base, pdfCapture: null, canComplete: true }).canComplete).toBe(true)
    expect(describeStatus({ ...base, pdfCapture: null, canComplete: false }).canComplete).toBe(
      false
    )
  })
})

describe('library list and status', () => {
  it('asks Zotero for its libraries and for the status of a source in one of them', async () => {
    vi.mocked(invoke).mockResolvedValue({ reachable: true, libraries: [] })
    await navegadorZoteroLibraries()
    expect(invoke).toHaveBeenLastCalledWith('navegador_zotero_libraries')
    await navegadorZoteroStatus('s1', 'c1', {
      libraryType: 'group',
      libraryId: '7',
      libraryName: 'prueba',
    })
    expect(invoke).toHaveBeenLastCalledWith('navegador_zotero_status', {
      sourceId: 's1',
      captureId: 'c1',
      library: { libraryType: 'group', libraryId: '7', libraryName: 'prueba' },
    })
  })

  it('opens an item through the existing select command, by key and library', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)
    await navegadorZoteroOpenItem('group', '7', 'ABCD2345')
    expect(invoke).toHaveBeenLastCalledWith('writing_zotero_open_item', {
      libraryType: 'group',
      libraryId: '7',
      itemKey: 'ABCD2345',
    })
  })

  it('describes a present status with its field labels and PDF note', () => {
    const view = describeStatus({
      state: 'present',
      source: 'zotero',
      itemKey: 'ABCD2345',
      pdf: 'parent_exists',
      pendingFields: ['accessDate', 'websiteTitle'],
      keptFields: ['title'],
    })
    expect(view.present).toBe(true)
    expect(view.fromRecord).toBe(false)
    expect(view.pendingKeys).toEqual([
      'navegador.zotero.field.accessDate',
      'navegador.zotero.field.websiteTitle',
    ])
    expect(view.keptKeys).toEqual(['navegador.zotero.field.title'])
    expect(view.pdfKey).toBe('navegador.zotero.note.pdf.parent_exists')
  })

  it('an absent or unreachable status is not present, and a record is flagged', () => {
    const base = {
      itemKey: null,
      pdf: 'none' as const,
      pendingFields: [] as string[],
      keptFields: [] as string[],
    }
    expect(describeStatus({ ...base, state: 'absent', source: 'none' }).present).toBe(false)
    expect(describeStatus({ ...base, state: 'unreachable', source: 'none' }).present).toBe(false)
    const record = describeStatus({
      ...base,
      state: 'present',
      source: 'record',
      itemKey: 'K',
      pdf: 'unknown',
    })
    expect(record.fromRecord).toBe(true)
    expect(record.pdfKey).toBeNull()
  })
})

describe('errors', () => {
  it('reads the stable code and the detail', () => {
    expect(parseZoteroError(new Error('not_found: no such source'))).toEqual({
      code: 'not_found',
      detail: 'no such source',
    })
    expect(parseZoteroError('invalid_library: blank')).toEqual({
      code: 'invalid_library',
      detail: 'blank',
    })
    expect(parseZoteroError(new Error('boom'))).toEqual({ code: 'unknown', detail: 'boom' })
  })
})
