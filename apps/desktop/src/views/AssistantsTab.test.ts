import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import AssistantsTab from './AssistantsTab.svelte'
import type { AgentAccessStatus, AgentClient, AgentScope } from '$lib/agent-access'

/**
 * Configuración → Asistentes (D11): the switch, the endpoint state, the paired
 * clients and the "Conectar un asistente" flow. The backend is the only thing
 * mocked — the tab talks to it through `$lib/agent-access`, so these tests also
 * pin the IPC payloads.
 */

const { invokeMock, writeTextMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  writeTextMock: vi.fn<(_: string) => Promise<void>>(),
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

const SCOPES: AgentScope[] = ['read']

const CLAUDE: AgentClient = {
  id: 'cl_abc123',
  name: 'Claude Code',
  scopes: SCOPES,
  tokenStore: 'keyring',
  createdAt: 1767225600,
  lastUsedAt: 1767312000,
}

/** A client paired on a machine with no OS keyring: its token is in a file. */
const FILE_STORED: AgentClient = {
  id: 'cl_file123',
  name: 'Codex',
  scopes: SCOPES,
  tokenStore: 'file',
  createdAt: 1767225600,
  lastUsedAt: null,
}

const ACTIVE_STATUS: AgentAccessStatus = {
  enabled: true,
  running: true,
  port: 41123,
  clients: [CLAUDE],
}

const STOPPED_STATUS: AgentAccessStatus = {
  enabled: false,
  running: false,
  port: null,
  clients: [CLAUDE],
}

/** Answer each command the tab may call, and fail loudly on anything else. */
function respond(routes: Record<string, unknown>): void {
  invokeMock.mockImplementation((command: string) => {
    if (!(command in routes)) {
      return Promise.reject(new Error(`unexpected command ${command}`))
    }
    return Promise.resolve(routes[command])
  })
}

function reconnectRoutes(): Record<string, unknown> {
  const created: AgentClient = {
    id: 'cl_new123',
    name: 'Claude Code',
    scopes: SCOPES,
    tokenStore: 'keyring',
    createdAt: 1767398400,
    lastUsedAt: null,
  }
  return {
    agent_access_status: { ...ACTIVE_STATUS, clients: [CLAUDE] },
    agent_client_create: { client: created, status: { ...ACTIVE_STATUS, clients: [CLAUDE, created] } },
  }
}

const endpointState = () => screen.getByTestId('assistants-endpoint-state')
const enabledSwitch = () => screen.getByRole('switch', { name: 'Permitir asistentes externos' })

describe('AssistantsTab', () => {
  beforeEach(() => {
    invokeMock.mockReset()
    writeTextMock.mockReset().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText: writeTextMock },
      configurable: true,
    })
  })

  it('shows the endpoint state and the paired clients on open', async () => {
    respond({ agent_access_status: ACTIVE_STATUS })
    render(AssistantsTab)

    await waitFor(() => expect(endpointState()).toHaveTextContent('Activo en 127.0.0.1:41123'))
    expect(invokeMock).toHaveBeenCalledWith('agent_access_status')
    expect(enabledSwitch()).toHaveAttribute('aria-checked', 'true')
    expect(screen.getByText('Claude Code')).toBeInTheDocument()
    expect(screen.getByTestId('assistants-client-created-cl_abc123')).toHaveTextContent('2026-01-01')
    expect(screen.getByTestId('assistants-client-last-used-cl_abc123')).toHaveTextContent(
      '2026-01-02'
    )
  })

  it('shows a stopped endpoint and no clients when access is disabled', async () => {
    respond({
      agent_access_status: { enabled: false, running: false, port: null, clients: [] },
    })
    render(AssistantsTab)

    await waitFor(() => expect(endpointState()).toHaveTextContent('Deshabilitado'))
    expect(enabledSwitch()).toHaveAttribute('aria-checked', 'false')
    expect(screen.getByText('Todavía no conectaste ningún asistente.')).toBeInTheDocument()
  })

  it('turns access off and re-renders the stopped endpoint', async () => {
    respond({ agent_access_status: ACTIVE_STATUS, agent_access_set_enabled: STOPPED_STATUS })
    render(AssistantsTab)

    await waitFor(() => expect(enabledSwitch()).toHaveAttribute('aria-checked', 'true'))
    await fireEvent.click(enabledSwitch())

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('agent_access_set_enabled', { enabled: false })
    )
    await waitFor(() => expect(endpointState()).toHaveTextContent('Deshabilitado'))
    expect(enabledSwitch()).toHaveAttribute('aria-checked', 'false')
  })

  it('pairs an assistant and shows the id with one line per client', async () => {
    respond(reconnectRoutes())
    render(AssistantsTab)

    const input = await screen.findByLabelText('Nombre del asistente')
    await fireEvent.input(input, { target: { value: '  Claude Code  ' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Conectar' }))

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('agent_client_create', { name: 'Claude Code' })
    )
    expect(await screen.findByTestId('assistants-client-id')).toHaveTextContent('cl_new123')
    expect(screen.getByTestId('assistants-connector-command')).toHaveTextContent(
      'entropia-agent-connector --client cl_new123'
    )
    expect(screen.getByTestId('assistants-line-claude')).toHaveTextContent(
      'claude mcp add entropia -- entropia-agent-connector --client cl_new123'
    )
    expect(screen.getByTestId('assistants-line-codex')).toHaveTextContent(
      'codex mcp add entropia -- entropia-agent-connector --client cl_new123'
    )
    expect(screen.getByTestId('assistants-line-opencode')).toHaveTextContent(
      '{"mcp":{"entropia":{"type":"local","command":["entropia-agent-connector","--client","cl_new123"]}}}'
    )
    expect(screen.getByTestId('assistants-line-pi')).toHaveTextContent(
      '{"mcpServers":{"entropia":{"command":"entropia-agent-connector","args":["--client","cl_new123"]}}}'
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Copiar comando para Claude Code' }))
    await waitFor(() =>
      expect(writeTextMock).toHaveBeenCalledWith(
        'claude mcp add entropia -- entropia-agent-connector --client cl_new123'
      )
    )
  })

  it('only revokes after the confirmation', async () => {
    respond({
      agent_access_status: ACTIVE_STATUS,
      agent_client_revoke: { ...ACTIVE_STATUS, clients: [] },
    })
    render(AssistantsTab)

    const revoke = await screen.findByRole('button', { name: 'Revocar Claude Code' })
    await fireEvent.click(revoke)
    expect(screen.getByRole('dialog')).toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalledWith('agent_client_revoke', expect.anything())

    await fireEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Cancelar' }))
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalledWith('agent_client_revoke', expect.anything())

    await fireEvent.click(screen.getByRole('button', { name: 'Revocar Claude Code' }))
    await fireEvent.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: 'Revocar asistente' })
    )

    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('agent_client_revoke', { clientId: 'cl_abc123' })
    )
    await waitFor(() => expect(screen.queryByText('Claude Code')).not.toBeInTheDocument())
  })

  it('shows where each client token is stored', async () => {
    respond({ agent_access_status: { ...ACTIVE_STATUS, clients: [CLAUDE, FILE_STORED] } })
    render(AssistantsTab)

    await waitFor(() =>
      expect(screen.getByTestId('assistants-client-storage-cl_abc123')).toHaveTextContent(
        'Llavero del sistema'
      )
    )
    expect(screen.getByTestId('assistants-client-storage-cl_file123')).toHaveTextContent(
      'Archivo protegido'
    )
    // Only the file case needs the hint: it is the one the person may wonder about.
    expect(
      screen.queryByTestId('assistants-client-storage-hint-cl_abc123')
    ).not.toBeInTheDocument()
    expect(screen.getByTestId('assistants-client-storage-hint-cl_file123')).toHaveTextContent(
      'solo puede leer tu usuario'
    )
  })

  it('shows a backend error as a message', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'agent_access_status') return Promise.resolve(ACTIVE_STATUS)
      return Promise.reject('credential_store_unavailable: no hay llavero del sistema')
    })
    render(AssistantsTab)

    await waitFor(() => expect(enabledSwitch()).toHaveAttribute('aria-checked', 'true'))
    await fireEvent.click(enabledSwitch())

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'credential_store_unavailable: no hay llavero del sistema'
    )
  })

  it('never renders a token, even if the backend sent one', async () => {
    // The backend never returns a token; this pins that the panel only uses
    // the fields it names, so an extra field can never reach the screen or a
    // copied line (S4).
    const leaky = { ...CLAUDE, token: 'eat_deadbeef' }
    respond({
      agent_access_status: { ...ACTIVE_STATUS, clients: [leaky] },
      agent_client_create: { client: leaky, status: { ...ACTIVE_STATUS, clients: [leaky] } },
    })
    render(AssistantsTab)

    const input = await screen.findByLabelText('Nombre del asistente')
    await fireEvent.input(input, { target: { value: 'Claude Code' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Conectar' }))
    await screen.findByTestId('assistants-client-id')

    expect(document.body.textContent ?? '').not.toContain('eat_')
    for (const button of screen.getAllByRole('button')) {
      expect(button.getAttribute('aria-label') ?? '').not.toContain('eat_')
    }
  })
})
