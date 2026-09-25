import { render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import WorkPane from './WorkPane.svelte'
import CollectionRouteProbe from './__fixtures__/CollectionRouteProbe.svelte'
import { workspace } from '$lib/workspace'
import { writing, type WritingDocumentRow } from '$lib/writing'
import { locale } from '$lib/i18n'

/**
 * A pane leaving Writing for another route in one flush tears the view down.
 * Its onDestroy flushes what is pending, and the document stays open in the
 * store on purpose (navigating back returns to it). The reconciling effect's
 * close branch must not ALSO flush-and-close on that same navigation: two
 * overlapping saves would both see status `saving`, the second would reuse
 * the same `expected_revision`, and the backend would answer
 * `revision_conflict` for an edit that was in fact saved.
 *
 * Mounted for real — WorkPane, the real WritingView and the real `writing`
 * store — with only Tauri's `invoke` answered below, because the hazard is
 * the order two lifecycles run in, which a source-level check cannot show.
 */

vi.mock('$lib/db', () => ({
  getStore: () => ({
    items: {
      findPreviousCardSummary: vi.fn().mockResolvedValue(null),
      findNextCardSummary: vi.fn().mockResolvedValue(null),
    },
  }),
}))

vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({ onDragDropEvent: vi.fn().mockResolvedValue(vi.fn()) }),
}))

vi.mock('$lib/route-loader', async (importOriginal) => {
  const actual = await importOriginal<typeof import('$lib/route-loader')>()
  return {
    ...actual,
    loadRouteView: (name: Parameters<typeof actual.loadRouteView>[0]) =>
      name === 'collection'
        ? Promise.resolve({ default: CollectionRouteProbe })
        : actual.loadRouteView(name),
  }
})

const CONTENT = { schemaVersion: 1, doc: { type: 'doc', content: [{ type: 'paragraph' }] } }
const EDITED = {
  schemaVersion: 1,
  doc: {
    type: 'doc',
    content: [{ type: 'paragraph', content: [{ type: 'text', text: 'unsaved' }] }],
  },
}

const ROW: WritingDocumentRow = {
  id: 'd1',
  title: 'Articulo',
  document_type: 'article',
  status: 'active',
  schema_version: 1,
  current_content_json: JSON.stringify(CONTENT),
  revision: 3,
  created_at: 1,
  updated_at: 1,
}

describe('leaving Writing for another route with a pending edit', () => {
  let serverRevision = 3
  const saves: Array<{ expected_revision: number }> = []

  beforeEach(() => {
    locale.set('es')
    serverRevision = 3
    saves.length = 0
    vi.mocked(invoke).mockImplementation((async (command: string, args?: unknown) => {
      if (command === 'resolve_data_dir') return '/mock/app-data'
      if (command === 'writing_is_ready') return true
      if (command === 'writing_list_documents') return [ROW]
      if (command === 'writing_load_document') return { ...ROW, revision: serverRevision }
      if (command === 'writing_append_journal') return 1
      if (command === 'writing_save_document') {
        const save = (args as { save: { expected_revision: number } }).save
        saves.push({ expected_revision: save.expected_revision })
        // Let a concurrent save overlap this one, the way a real IPC would.
        await new Promise((resolve) => setTimeout(resolve, 5))
        if (save.expected_revision !== serverRevision) {
          throw { code: 'revision_conflict', message: 'revision conflict' }
        }
        serverRevision += 1
        return serverRevision
      }
      return undefined
    }) as typeof invoke)
    while (workspace.tabs.length > 1) workspace.closeTab(workspace.tabs.at(-1)!.id)
    workspace.activeNavigation.resetToPath([{ name: 'home' }])
  })

  afterEach(() => {
    writing.closeDocument()
    vi.mocked(invoke).mockReset()
  })

  it('saves the edit once, without a revision conflict, and keeps the document open', async () => {
    const nav = workspace.activeNavigation
    nav.navigate({ name: 'writing', documentId: 'd1', documentTitle: 'Articulo' })
    render(WorkPane, { paneId: workspace.activeTabId })
    await waitFor(() => expect(writing.snapshot.open?.id).toBe('d1'), { timeout: 5000 })

    writing.applyEdit(EDITED)
    expect(writing.snapshot.status).toBe('pending')

    nav.navigate({ name: 'collection', id: 'col-1', collectionName: 'Archivo' })
    await waitFor(() => expect(screen.getByTestId('collection-route-probe')).toBeInTheDocument())
    await waitFor(() => expect(writing.snapshot.status).toBe('saved'))
    await new Promise((resolve) => setTimeout(resolve, 30))

    expect(saves).toEqual([{ expected_revision: 3 }])
    expect(writing.snapshot.error).toBeNull()
    expect(writing.snapshot.open?.id).toBe('d1')
  })
})
