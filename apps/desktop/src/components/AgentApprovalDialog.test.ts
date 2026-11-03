import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import AgentApprovalDialog from './AgentApprovalDialog.svelte'
import type { AgentApproval } from '$lib/agent-access'

/**
 * The owner's answer to paid work an assistant asked for (D3). The backend
 * is the only thing mocked, so these tests also pin the IPC payloads.
 */

type Listener = (event: { payload: unknown }) => void

const { invokeMock, listeners } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listeners: new Map<string, Listener>(),
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn((name: string, callback: Listener) => {
    listeners.set(name, callback)
    return Promise.resolve(() => listeners.delete(name))
  }),
}))

const START: AgentApproval = {
  id: 'apr_1',
  clientId: 'cl_abc',
  clientName: 'Claude Code',
  action: { kind: 'start' },
  jobId: 'batch-1',
  revision: 3,
  operations: ['ocr', 'embeddings'],
  collections: ['Alfa', 'Beta'],
  files: 12,
  status: 'pending',
  createdAt: 1_767_225_600_000,
  expiresAt: 1_767_226_200_000,
  decidedAt: null,
  failure: null,
  preview: null,
  importFiles: null,
  deleteImpact: null,
  documentTitles: null,
  importResult: null,
}

/** Answer the commands the dialog calls; anything else fails loudly. */
function respond(pending: AgentApproval[][], decided?: AgentApproval | Error) {
  const queue = [...pending]
  invokeMock.mockImplementation((command: string) => {
    if (command === 'agent_approvals_pending') {
      return Promise.resolve(queue.length > 1 ? queue.shift() : (queue[0] ?? []))
    }
    if (command === 'agent_approval_decide') {
      return decided instanceof Error ? Promise.reject(decided) : Promise.resolve(decided)
    }
    return Promise.reject(new Error(`unexpected command ${command}`))
  })
}

describe('AgentApprovalDialog', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    listeners.clear()
  })

  it('shows nothing while no assistant is waiting', async () => {
    respond([[]])
    render(AgentApprovalDialog)
    await waitFor(() => expect(invokeMock).toHaveBeenCalledWith('agent_approvals_pending'))
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('shows who asks, what would run and over what, with the focus on Rechazar', async () => {
    respond([[START]])
    render(AgentApprovalDialog)

    const dialog = await screen.findByRole('dialog')
    expect(dialog).toHaveTextContent('Claude Code pide empezar un procesamiento')
    expect(dialog).toHaveTextContent('OCR')
    expect(dialog).toHaveTextContent('Embeddings')
    expect(dialog).toHaveTextContent('Alfa, Beta')
    expect(dialog).toHaveTextContent('12 archivos')
    expect(dialog).toHaveTextContent('puede usar proveedores pagos')
    expect(within(dialog).getByRole('button', { name: 'Rechazar' })).toHaveFocus()
  })

  it('says OCR alone is followed by automatic embeddings, which may also cost', async () => {
    respond([[{ ...START, operations: ['ocr'] }]])
    render(AgentApprovalDialog)

    const dialog = await screen.findByRole('dialog')
    expect(dialog).toHaveTextContent(
      'Después del OCR, EntropIA calcula los embeddings de cada página reconocida'
    )
    expect(dialog).toHaveTextContent('también puede tener costo')
  })

  it('does not repeat the embeddings note when they are already requested', async () => {
    respond([[START]])
    render(AgentApprovalDialog)

    const dialog = await screen.findByRole('dialog')
    expect(dialog).not.toHaveTextContent('Después del OCR')
  })

  it('approves only on the explicit button and closes when nothing is left', async () => {
    respond([[START], []], { ...START, status: 'approved', decidedAt: 1 })
    render(AgentApprovalDialog)

    const dialog = await screen.findByRole('dialog')
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Aprobar' }))

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('agent_approval_decide', {
        approvalId: 'apr_1',
        approve: true,
      })
    )
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
  })

  it('rejects on Rechazar and runs nothing', async () => {
    respond([[START], []], { ...START, status: 'rejected', decidedAt: 1 })
    render(AgentApprovalDialog)

    await fireEvent.click(
      within(await screen.findByRole('dialog')).getByRole('button', { name: 'Rechazar' })
    )
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('agent_approval_decide', {
        approvalId: 'apr_1',
        approve: false,
      })
    )
    expect(invokeMock).not.toHaveBeenCalledWith('agent_approval_decide', {
      approvalId: 'apr_1',
      approve: true,
    })
  })

  it('names resume and retry requests', async () => {
    respond([
      [
        { ...START, id: 'apr_2', action: { kind: 'resume' } },
        { ...START, id: 'apr_3', action: { kind: 'retry', task_id: null, failed_only: true } },
      ],
    ])
    render(AgentApprovalDialog)
    const dialog = await screen.findByRole('dialog')
    expect(dialog).toHaveTextContent('Claude Code pide reanudar un procesamiento')
    expect(dialog).toHaveTextContent('1 pedido más esperando')
  })

  it('shows a proposed text replacement with its preview and its undo, without spending', async () => {
    respond([
      [
        {
          ...START,
          id: 'apr_txt',
          action: {
            kind: 'replace_text',
            item_id: 'ite_1',
            asset_id: 'ast_1',
            text_hash: 'a'.repeat(64),
            new_text_hash: 'b'.repeat(64),
          },
          jobId: '',
          operations: ['replace_text'],
          collections: ['Alfa'],
          files: 1,
          preview: 'Página uno corregida por Sarmiento.',
        },
      ],
    ])
    render(AgentApprovalDialog)
    const dialog = await screen.findByRole('dialog')
    expect(dialog).toHaveTextContent('Claude Code pide reemplazar el texto de una página')
    expect(dialog).toHaveTextContent('Página uno corregida por Sarmiento.')
    expect(dialog).toHaveTextContent('Restaurar original')
    expect(dialog).not.toHaveTextContent('proveedores pagos')
    expect(within(dialog).getByRole('button', { name: 'Rechazar' })).toHaveFocus()
  })

  it('lists the files of an import request and says it spends nothing', async () => {
    respond([
      [
        {
          ...START,
          id: 'apr_imp',
          action: {
            kind: 'import_files',
            collection_id: 'col_1',
            files: 2,
            plan_hash: 'c'.repeat(64),
          },
          jobId: '',
          operations: ['import_files'],
          collections: ['Cartas'],
          files: 2,
          importFiles: [
            { name: 'Carta de Sarmiento.pdf', kind: 'pdf', sizeBytes: 204800 },
            { name: 'Retrato.jpg', kind: 'image', sizeBytes: 512 },
          ],
        },
      ],
    ])
    render(AgentApprovalDialog)
    const dialog = await screen.findByRole('dialog')
    expect(dialog).toHaveTextContent('Claude Code pide importar archivos a una colección')
    expect(dialog).toHaveTextContent('Cartas')
    expect(dialog).toHaveTextContent('Carta de Sarmiento.pdf')
    expect(dialog).toHaveTextContent('Retrato.jpg')
    expect(dialog).toHaveTextContent('200 KB')
    expect(dialog).toHaveTextContent('No gasta')
    expect(dialog).not.toHaveTextContent('proveedores pagos')
    expect(within(dialog).getByRole('button', { name: 'Rechazar' })).toHaveFocus()
  })

  it('shows a document delete with its impact, its titles and a Borrar button', async () => {
    respond([
      [
        {
          ...START,
          id: 'apr_del',
          action: {
            kind: 'delete_items',
            item_ids: ['ite_1', 'ite_2', 'ite_3'],
            set_hash: 'd'.repeat(64),
          },
          jobId: '',
          revision: 0,
          operations: ['delete_items'],
          collections: ['Cartas'],
          files: 3,
          deleteImpact: {
            documents: 3,
            pages: 14,
            notes: 2,
            annotations: 5,
            documentsWithText: 2,
          },
          documentTitles: ['Carta de Sarmiento', 'Retrato de familia', 'Apuntes'],
        },
      ],
    ])
    render(AgentApprovalDialog)
    const dialog = await screen.findByRole('dialog')
    expect(dialog).toHaveTextContent('Claude Code pide borrar 3 documentos')
    expect(dialog).toHaveTextContent('Cartas')
    expect(dialog).toHaveTextContent('Carta de Sarmiento')
    expect(dialog).toHaveTextContent('Retrato de familia')
    expect(dialog).toHaveTextContent(
      'Se pierden: 3 documentos, 14 páginas, 2 notas, 5 anotaciones y su texto reconocido'
    )
    expect(dialog).toHaveTextContent('No se puede deshacer y se borra en todos tus equipos.')
    expect(dialog).not.toHaveTextContent('proveedores pagos')
    expect(within(dialog).getByRole('button', { name: 'Borrar' })).toBeInTheDocument()
    expect(within(dialog).queryByRole('button', { name: 'Aprobar' })).not.toBeInTheDocument()
    expect(within(dialog).getByRole('button', { name: 'Rechazar' })).toHaveFocus()
  })

  it('asks to delete a whole collection by name', async () => {
    respond([
      [
        {
          ...START,
          id: 'apr_del_col',
          action: {
            kind: 'delete_collection',
            collection_id: 'col_1',
            set_hash: 'e'.repeat(64),
          },
          jobId: '',
          revision: 0,
          operations: ['delete_collection'],
          collections: ['Cartas'],
          files: 2,
          deleteImpact: {
            documents: 2,
            pages: 0,
            notes: 1,
            annotations: 0,
            documentsWithText: 0,
          },
          documentTitles: ['Revista 1', 'Revista 2'],
        },
      ],
    ])
    render(AgentApprovalDialog)
    const dialog = await screen.findByRole('dialog')
    expect(dialog).toHaveTextContent('Claude Code pide borrar una colección')
    expect(dialog).toHaveTextContent('Cartas')
    expect(dialog).toHaveTextContent('Revista 1')
    expect(dialog).toHaveTextContent('Se pierden: 2 documentos, 1 nota.')
    expect(dialog).toHaveTextContent('No se puede deshacer y se borra en todos tus equipos.')
    expect(within(dialog).getByRole('button', { name: 'Borrar' })).toBeInTheDocument()
    expect(within(dialog).getByRole('button', { name: 'Rechazar' })).toHaveFocus()
  })

  it('caps the titles of a large delete and counts the rest', async () => {
    const titles = Array.from({ length: 20 }, (_, index) => `Documento ${index + 1}`)
    respond([
      [
        {
          ...START,
          id: 'apr_del_many',
          action: { kind: 'delete_items', item_ids: ['ite_1'], set_hash: 'f'.repeat(64) },
          jobId: '',
          revision: 0,
          operations: ['delete_items'],
          collections: ['Cartas'],
          files: 25,
          deleteImpact: {
            documents: 25,
            pages: 25,
            notes: 0,
            annotations: 0,
            documentsWithText: 0,
          },
          documentTitles: titles,
        },
      ],
    ])
    render(AgentApprovalDialog)
    const dialog = await screen.findByRole('dialog')
    expect(dialog).toHaveTextContent('Claude Code pide borrar 25 documentos')
    expect(dialog).toHaveTextContent('Documento 20')
    expect(dialog).toHaveTextContent('y 5 más')
    expect(dialog).toHaveTextContent('Se pierden: 25 documentos, 25 páginas.')
  })

  it('reads the queue again when the backend says it changed', async () => {
    respond([[], [START]])
    render(AgentApprovalDialog)
    await waitFor(() => expect(listeners.has('agent-approvals:changed')).toBe(true))
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()

    listeners.get('agent-approvals:changed')?.({ payload: null })
    expect(await screen.findByRole('dialog')).toHaveTextContent('Claude Code')
  })

  it('says so when an approved job had changed and nothing ran', async () => {
    respond([[START], []], {
      ...START,
      status: 'failed',
      decidedAt: 1,
      failure: 'El trabajo cambió después del pedido; no se ejecutó nada.',
    })
    render(AgentApprovalDialog)
    await fireEvent.click(
      within(await screen.findByRole('dialog')).getByRole('button', { name: 'Aprobar' })
    )
    expect(await screen.findByRole('alert')).toHaveTextContent('no se ejecutó nada')
  })

  it('shows an expired request as an error and re-reads the queue', async () => {
    respond([[START], []], new Error('approval_expired: request apr_1 expired'))
    render(AgentApprovalDialog)
    await fireEvent.click(
      within(await screen.findByRole('dialog')).getByRole('button', { name: 'Aprobar' })
    )
    expect(await screen.findByRole('alert')).toHaveTextContent('venció')
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
  })
})
