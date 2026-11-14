import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { locale } from '$lib/i18n'
import HelpApprovalDialog from './HelpApprovalDialog.svelte'

const mockInvoke = vi.mocked(invoke)

const REQUEST = {
  id: 'help-apr-1',
  tool: 'collections_create',
  summary: 'Crear la colección «SIMAPE»',
}

describe('HelpApprovalDialog (T27)', () => {
  beforeEach(() => {
    locale.set('es')
    mockInvoke.mockReset()
  })

  it('shows nothing until the backend asks for an approval', () => {
    render(HelpApprovalDialog, { props: { request: null, onanswered: vi.fn() } })

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('shows the request as a modal dialog with its summary', () => {
    render(HelpApprovalDialog, { props: { request: REQUEST, onanswered: vi.fn() } })

    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveTextContent('La ayuda pide aprobación')
    expect(dialog).toHaveTextContent('Crear la colección «SIMAPE»')
    expect(screen.getByRole('button', { name: 'Aprobar' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Rechazar' })).toBeInTheDocument()
  })

  it('focuses Rechazar, like the MCP approval dialog does', async () => {
    render(HelpApprovalDialog, { props: { request: REQUEST, onanswered: vi.fn() } })

    await waitFor(() => expect(screen.getByRole('button', { name: 'Rechazar' })).toHaveFocus())
  })

  it('Aprobar answers with the id and reports the decision', async () => {
    const onanswered = vi.fn()
    render(HelpApprovalDialog, { props: { request: REQUEST, onanswered } })

    await fireEvent.click(screen.getByRole('button', { name: 'Aprobar' }))

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('help_approval_answer', {
        id: 'help-apr-1',
        approved: true,
      })
    )
    expect(onanswered).toHaveBeenCalledWith({
      id: 'help-apr-1',
      approved: true,
      summary: 'Crear la colección «SIMAPE»',
    })
  })

  it('Rechazar answers negatively and reports the decision', async () => {
    const onanswered = vi.fn()
    render(HelpApprovalDialog, { props: { request: REQUEST, onanswered } })

    await fireEvent.click(screen.getByRole('button', { name: 'Rechazar' }))

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('help_approval_answer', {
        id: 'help-apr-1',
        approved: false,
      })
    )
    expect(onanswered).toHaveBeenCalledWith({
      id: 'help-apr-1',
      approved: false,
      summary: 'Crear la colección «SIMAPE»',
    })
  })

  it('Escape rejects, like the MCP approval dialog does', async () => {
    const onanswered = vi.fn()
    render(HelpApprovalDialog, { props: { request: REQUEST, onanswered } })

    await fireEvent.keyDown(window, { key: 'Escape' })

    await waitFor(() =>
      expect(mockInvoke).toHaveBeenCalledWith('help_approval_answer', {
        id: 'help-apr-1',
        approved: false,
      })
    )
    expect(onanswered).toHaveBeenCalledWith({
      id: 'help-apr-1',
      approved: false,
      summary: 'Crear la colección «SIMAPE»',
    })
  })

  it('a click outside does nothing: the dialog stays and nobody is answered', async () => {
    const onanswered = vi.fn()
    render(HelpApprovalDialog, { props: { request: REQUEST, onanswered } })

    await fireEvent.click(document.querySelector('.confirm-dialog__overlay') as Element)

    expect(mockInvoke).not.toHaveBeenCalled()
    expect(onanswered).not.toHaveBeenCalled()
    expect(screen.getByRole('dialog')).toBeInTheDocument()
  })

  it('shows an error when the answer fails instead of closing silently', async () => {
    mockInvoke.mockRejectedValue(new Error('denied'))
    render(HelpApprovalDialog, { props: { request: REQUEST, onanswered: vi.fn() } })

    await fireEvent.click(screen.getByRole('button', { name: 'Aprobar' }))

    expect(await screen.findByRole('alert')).toHaveTextContent('No se pudo responder el pedido')
    // The request is still open: the owner can retry or reject.
    expect(screen.getByRole('dialog')).toBeInTheDocument()
  })
})
