import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'

import { locale, t } from './i18n'
import { loadWritingSyncNotices, selectWritingSyncNotices, type WritingSyncNotice } from './writing'

/**
 * The writing-sync notice over the manuscript list.
 *
 * The list gets its notice text from the `writing_sync_notices` command and a
 * pure copy selection, so these tests stand in for the view: they prove the
 * notice text appears for a document the command reported, and only then. The
 * view renders exactly these strings with `t()`.
 */
const mockInvoke = vi.mocked(invoke)

function notice(overrides: Partial<WritingSyncNotice> = {}): WritingSyncNotice {
  return {
    document_id: 'doc-1',
    conflict_copy: false,
    pending_outbox: false,
    pending_assets: false,
    queued_receive: false,
    last_error: null,
    ...overrides,
  }
}

/** The notice text the manuscript list renders for one document. */
function renderedLines(rows: WritingSyncNotice[], documentId: string): string[] {
  const view = selectWritingSyncNotices(rows).get(documentId)
  return view ? view.lines.map((line) => t(line.key, line.params)) : []
}

describe('writing sync notices', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    locale.set('es')
  })

  it('says nothing when the command reports no notice', async () => {
    mockInvoke.mockResolvedValueOnce([])

    const rows = await loadWritingSyncNotices()

    expect(mockInvoke).toHaveBeenCalledWith('writing_sync_notices')
    expect(rows).toEqual([])
    // No notice, no banner and no row cue.
    expect(selectWritingSyncNotices(rows).size).toBe(0)
    expect(renderedLines(rows, 'doc-1')).toEqual([])
  })

  it('says nothing when the command cannot be read', async () => {
    mockInvoke.mockRejectedValueOnce(new Error('database unavailable'))

    expect(await loadWritingSyncNotices()).toEqual([])
  })

  it('shows the conflict copy text only for the marked document', async () => {
    mockInvoke.mockResolvedValueOnce([notice({ document_id: 'doc-9', conflict_copy: true })])

    const rows = await loadWritingSyncNotices()

    expect(renderedLines(rows, 'doc-9')).toEqual([
      'Es una copia guardada después de un conflicto de sincronización.',
    ])
    // The same copy selection gives the row its compact cue...
    expect(t(selectWritingSyncNotices(rows).get('doc-9')!.cue!)).toBe('Copia por conflicto')
    // ...and a document the command did not report gets no notice text at all.
    expect(renderedLines(rows, 'doc-1')).toEqual([])
  })

  it('names every kind of pending transfer and the stored error', async () => {
    mockInvoke.mockResolvedValueOnce([
      notice({
        document_id: 'doc-2',
        pending_outbox: true,
        pending_assets: true,
        queued_receive: true,
        last_error: 'Falta una colección destino',
      }),
    ])

    const rows = await loadWritingSyncNotices()

    expect(renderedLines(rows, 'doc-2')).toEqual([
      'Hay cambios todavía sin enviar.',
      'Faltan imágenes o archivos por transferir.',
      'Hay una actualización de otro equipo esperando para aplicarse.',
      'Último error: Falta una colección destino',
    ])
    expect(t(selectWritingSyncNotices(rows).get('doc-2')!.cue!)).toBe('Sincronización pendiente')
  })

  it('translates the notice into English', async () => {
    locale.set('en')
    mockInvoke.mockResolvedValueOnce([
      notice({ document_id: 'doc-3', conflict_copy: true, pending_outbox: true }),
    ])

    const rows = await loadWritingSyncNotices()

    expect(renderedLines(rows, 'doc-3')).toEqual([
      'Kept as a copy after a sync conflict.',
      'Some changes are still waiting to be sent.',
    ])
    expect(t(selectWritingSyncNotices(rows).get('doc-3')!.cue!)).toBe('Conflict copy')
  })
})
