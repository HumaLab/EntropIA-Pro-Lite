import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import AssistantsTab from './AssistantsTab.svelte'
import type { AgentAccessStatus, AgentClient, AgentScope } from '$lib/agent-access'
import {
  connectorInvocation,
  connectorRecipes,
  NO_CONNECTOR,
  type ConnectorLocation,
} from '$lib/agent-access'

/**
 * Configuración → Asistentes (D11): the switch, the endpoint state, the paired
 * clients and the "Conectar un asistente" flow. The backend is the only thing
 * mocked — the tab talks to it through `$lib/agent-access`, so these tests also
 * pin the IPC payloads.
 */

const { invokeMock, writeTextMock, openDialogMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  writeTextMock: vi.fn<(_: string) => Promise<void>>(),
  openDialogMock: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: openDialogMock }))

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

/**
 * The connector bundled next to the app executable on Windows (D15): the
 * awkward case for every recipe — spaces and a non-ASCII user name.
 */
const WINDOWS_CONNECTOR: ConnectorLocation = {
  installed: true,
  path: 'C:\\Users\\José Pérez\\AppData\\Local\\EntropIA Lite\\entropia-agent-connector.exe',
  kind: 'bundled',
}

/** The same on Linux/macOS: the installer puts it beside the main binary. */
const POSIX_CONNECTOR: ConnectorLocation = {
  installed: true,
  path: '/opt/EntropIA Lite/entropia-agent-connector',
  kind: 'bundled',
}

/** Under MSIX the app can only point at the execution alias (D15). */
const MSIX_CONNECTOR: ConnectorLocation = {
  installed: true,
  path: 'entropia-agent-connector.exe',
  kind: 'msix_alias',
}

/** No installer carried it (a dev run, or a build made without the overlay). */
const MISSING_CONNECTOR: ConnectorLocation = NO_CONNECTOR

const ACTIVE_STATUS: AgentAccessStatus = {
  enabled: true,
  running: true,
  port: 41123,
  clients: [CLAUDE],
  importFolders: [],
}

const STOPPED_STATUS: AgentAccessStatus = {
  enabled: false,
  running: false,
  port: null,
  clients: [CLAUDE],
  importFolders: [],
}

/**
 * Answer each command the tab may call, and fail loudly on anything else. The
 * connector location defaults to "missing" (a build without the D15 overlay),
 * which is what the tests that predate bundling expect.
 */
function respond(routes: Record<string, unknown>): void {
  invokeMock.mockImplementation((command: string) => {
    if (!(command in routes)) {
      if (command === 'agent_connector_location') {
        return Promise.resolve(MISSING_CONNECTOR)
      }
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
    agent_client_create: {
      client: created,
      status: { ...ACTIVE_STATUS, clients: [CLAUDE, created] },
    },
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
    expect(screen.getByTestId('assistants-client-created-cl_abc123')).toHaveTextContent(
      '2026-01-01'
    )
    expect(screen.getByTestId('assistants-client-last-used-cl_abc123')).toHaveTextContent(
      '2026-01-02'
    )
  })

  it('shows a stopped endpoint and no clients when access is disabled', async () => {
    respond({
      agent_access_status: {
        enabled: false,
        running: false,
        port: null,
        clients: [],
        importFolders: [],
      },
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

    await fireEvent.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: 'Cancelar' })
    )
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

  it('grants processing only after the confirmation', async () => {
    const granted: AgentClient = { ...CLAUDE, scopes: ['read', 'local_write'] }
    respond({
      agent_access_status: ACTIVE_STATUS,
      agent_client_set_grants: { ...ACTIVE_STATUS, clients: [granted] },
    })
    render(AssistantsTab)

    const prepare = await screen.findByRole('checkbox', {
      name: 'Preparar procesamiento y editar el corpus: Claude Code',
    })
    expect(prepare).not.toBeChecked()
    await fireEvent.click(prepare)
    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveTextContent('Nada de esto gasta')
    expect(dialog).toHaveTextContent('proponer importar archivos de tus carpetas de entrada')
    expect(invokeMock).not.toHaveBeenCalledWith('agent_client_set_grants', expect.anything())

    await fireEvent.click(within(dialog).getByRole('button', { name: 'Cancelar' }))
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalledWith('agent_client_set_grants', expect.anything())
    // The box shows what the backend holds again once the owner cancels.
    const again = screen.getByRole('checkbox', {
      name: 'Preparar procesamiento y editar el corpus: Claude Code',
    })
    expect(again).not.toBeChecked()

    await fireEvent.click(again)
    await fireEvent.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: 'Permitir' })
    )
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('agent_client_set_grants', {
        clientId: 'cl_abc123',
        grants: { localWrite: true, remoteSpend: false },
      })
    )
    await waitFor(() =>
      expect(
        screen.getByRole('checkbox', {
          name: 'Preparar procesamiento y editar el corpus: Claude Code',
        })
      ).toBeChecked()
    )
  })

  it('says every paid request is approved in the app before granting spend', async () => {
    respond({ agent_access_status: ACTIVE_STATUS })
    render(AssistantsTab)

    await fireEvent.click(
      await screen.findByRole('checkbox', { name: 'Pedir procesamiento pago: Claude Code' })
    )
    expect(screen.getByRole('dialog')).toHaveTextContent('no gasta nada hasta que lo apruebes')
  })

  it('withdraws a grant at once, without asking', async () => {
    const both: AgentClient = { ...CLAUDE, scopes: ['read', 'local_write', 'remote_spend'] }
    const prepareOnly: AgentClient = { ...CLAUDE, scopes: ['read', 'local_write'] }
    respond({
      agent_access_status: { ...ACTIVE_STATUS, clients: [both] },
      agent_client_set_grants: { ...ACTIVE_STATUS, clients: [prepareOnly] },
    })
    render(AssistantsTab)

    const spend = await screen.findByRole('checkbox', {
      name: 'Pedir procesamiento pago: Claude Code',
    })
    expect(spend).toBeChecked()
    await fireEvent.click(spend)
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('agent_client_set_grants', {
        clientId: 'cl_abc123',
        grants: { localWrite: true, remoteSpend: false },
      })
    )
    await waitFor(() =>
      expect(
        screen.getByRole('checkbox', { name: 'Pedir procesamiento pago: Claude Code' })
      ).not.toBeChecked()
    )
  })

  it('adds an import folder chosen with the system folder picker', async () => {
    openDialogMock.mockResolvedValue('/home/ana/Cartas')
    respond({
      agent_access_status: ACTIVE_STATUS,
      agent_import_folder_add: { ...ACTIVE_STATUS, importFolders: ['/home/ana/Cartas'] },
    })
    render(AssistantsTab)

    await fireEvent.click(await screen.findByRole('button', { name: 'Agregar carpeta' }))
    expect(openDialogMock).toHaveBeenCalledWith(
      expect.objectContaining({ directory: true, multiple: false })
    )
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('agent_import_folder_add', {
        path: '/home/ana/Cartas',
      })
    )
    expect(await screen.findByText('/home/ana/Cartas')).toBeInTheDocument()
  })

  it('does nothing when the folder picker is cancelled', async () => {
    openDialogMock.mockResolvedValue(null)
    respond({ agent_access_status: ACTIVE_STATUS })
    render(AssistantsTab)

    await fireEvent.click(await screen.findByRole('button', { name: 'Agregar carpeta' }))
    await waitFor(() => expect(openDialogMock).toHaveBeenCalled())
    expect(invokeMock).not.toHaveBeenCalledWith('agent_import_folder_add', expect.anything())
    expect(screen.getByText('Ninguna carpeta habilitada.')).toBeInTheDocument()
  })

  it('removes an import folder at once', async () => {
    respond({
      agent_access_status: { ...ACTIVE_STATUS, importFolders: ['/home/ana/Cartas'] },
      agent_import_folder_remove: ACTIVE_STATUS,
    })
    render(AssistantsTab)

    await fireEvent.click(
      await screen.findByRole('button', { name: 'Quitar carpeta /home/ana/Cartas' })
    )
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('agent_import_folder_remove', {
        path: '/home/ana/Cartas',
      })
    )
    await waitFor(() => expect(screen.queryByText('/home/ana/Cartas')).not.toBeInTheDocument())
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
    expect(screen.queryByTestId('assistants-client-storage-hint-cl_abc123')).not.toBeInTheDocument()
    expect(screen.getByTestId('assistants-client-storage-hint-cl_file123')).toHaveTextContent(
      'solo puede leer tu usuario'
    )
  })

  it('shows the installed connector path and builds the lines from it', async () => {
    respond({
      ...reconnectRoutes(),
      agent_connector_location: POSIX_CONNECTOR,
    })
    render(AssistantsTab)

    const input = await screen.findByLabelText('Nombre del asistente')
    await fireEvent.input(input, { target: { value: 'Claude Code' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Conectar' }))

    expect(await screen.findByTestId('assistants-connector-path')).toHaveTextContent(
      '/opt/EntropIA Lite/entropia-agent-connector'
    )
    expect(screen.getByTestId('assistants-connector-command')).toHaveTextContent(
      "'/opt/EntropIA Lite/entropia-agent-connector' --client cl_new123"
    )
    expect(screen.getByTestId('assistants-line-claude')).toHaveTextContent(
      "claude mcp add entropia -- '/opt/EntropIA Lite/entropia-agent-connector' --client cl_new123"
    )
    expect(screen.queryByTestId('assistants-connector-missing')).not.toBeInTheDocument()
  })

  it('tells a Windows install the lines are for PowerShell', async () => {
    respond({
      ...reconnectRoutes(),
      agent_connector_location: WINDOWS_CONNECTOR,
    })
    render(AssistantsTab)

    const input = await screen.findByLabelText('Nombre del asistente')
    await fireEvent.input(input, { target: { value: 'Claude Code' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Conectar' }))

    expect(await screen.findByTestId('assistants-connector-powershell')).toHaveTextContent(
      'PowerShell'
    )
    expect(screen.getByTestId('assistants-connector-command')).toHaveTextContent(
      `& '${WINDOWS_CONNECTOR.path}' --client cl_new123`
    )
  })

  it('does not mention PowerShell for a POSIX install', async () => {
    respond({
      ...reconnectRoutes(),
      agent_connector_location: POSIX_CONNECTOR,
    })
    render(AssistantsTab)

    const input = await screen.findByLabelText('Nombre del asistente')
    await fireEvent.input(input, { target: { value: 'Claude Code' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Conectar' }))

    await screen.findByTestId('assistants-connector-path')
    expect(screen.queryByTestId('assistants-connector-powershell')).not.toBeInTheDocument()
  })

  it('says the connector is not installed and keeps the bare command', async () => {
    respond(reconnectRoutes())
    render(AssistantsTab)

    const input = await screen.findByLabelText('Nombre del asistente')
    await fireEvent.input(input, { target: { value: 'Claude Code' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Conectar' }))

    expect(await screen.findByTestId('assistants-connector-missing')).toHaveTextContent(
      'no está instalado en esta compilación'
    )
    expect(screen.queryByTestId('assistants-connector-path')).not.toBeInTheDocument()
    expect(screen.getByTestId('assistants-connector-command')).toHaveTextContent(
      'entropia-agent-connector --client cl_new123'
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

/**
 * The recipes `$lib/agent-access` hands to the panel (D15). They live in that
 * module, but this is the only test file the change owns, so the exact strings
 * every supported client receives are pinned here: one quote or escape mistake
 * and the line pastes into the agent's config as a different program.
 */
describe('connector recipes', () => {
  const CLIENT = 'cl_new123'

  /** The program a config names, read back out of the JSON a client receives. */
  function opencodeProgram(location: ConnectorLocation): string[] {
    const line = connectorRecipes(CLIENT, location).find((r) => r.id === 'opencode')!.line
    return JSON.parse(line).mcp.entropia.command
  }

  function piConfig(location: ConnectorLocation): { command: string; args: string[] } {
    const line = connectorRecipes(CLIENT, location).find((r) => r.id === 'pi')!.line
    return JSON.parse(line).mcpServers.entropia
  }

  it('single-quotes a Windows path with spaces and non-ASCII for PowerShell', () => {
    const program = WINDOWS_CONNECTOR.path
    expect(connectorInvocation(CLIENT, WINDOWS_CONNECTOR)).toBe(`& '${program}' --client ${CLIENT}`)
    expect(connectorRecipes(CLIENT, WINDOWS_CONNECTOR)).toEqual([
      {
        id: 'claude',
        label: 'Claude Code',
        kind: 'command',
        line: `claude mcp add entropia -- '${program}' --client ${CLIENT}`,
      },
      {
        id: 'codex',
        label: 'Codex',
        kind: 'command',
        line: `codex mcp add entropia -- '${program}' --client ${CLIENT}`,
      },
      {
        id: 'opencode',
        label: 'OpenCode',
        kind: 'config',
        line: `{"mcp":{"entropia":{"type":"local","command":${JSON.stringify([program, '--client', CLIENT])}}}}`,
      },
      {
        id: 'pi',
        label: 'Pi',
        kind: 'config',
        line: `{"mcpServers":{"entropia":{"command":${JSON.stringify(program)},"args":${JSON.stringify(['--client', CLIENT])}}}}`,
      },
    ])
  })

  it('keeps $ and $( literal in a Windows path: single quotes never expand', () => {
    const path = 'C:\\Users\\$HOME\\$(x)\\entropia-agent-connector.exe'
    const location: ConnectorLocation = { installed: true, path, kind: 'bundled' }
    expect(connectorInvocation(CLIENT, location)).toBe(`& '${path}' --client ${CLIENT}`)
    const claude = connectorRecipes(CLIENT, location).find((r) => r.id === 'claude')!
    expect(claude.line).toBe(`claude mcp add entropia -- '${path}' --client ${CLIENT}`)
    expect(opencodeProgram(location)).toEqual([path, '--client', CLIENT])
    expect(piConfig(location)).toEqual({ command: path, args: ['--client', CLIENT] })
  })

  it('doubles a single quote inside a Windows path', () => {
    const path = "C:\\Users\\O'Brien\\entropia-agent-connector.exe"
    const location: ConnectorLocation = { installed: true, path, kind: 'bundled' }
    const quoted = "'C:\\Users\\O''Brien\\entropia-agent-connector.exe'"
    expect(connectorInvocation(CLIENT, location)).toBe(`& ${quoted} --client ${CLIENT}`)
    const codex = connectorRecipes(CLIENT, location).find((r) => r.id === 'codex')!
    expect(codex.line).toBe(`codex mcp add entropia -- ${quoted} --client ${CLIENT}`)
    expect(opencodeProgram(location)).toEqual([path, '--client', CLIENT])
  })

  it('doubles the typographic single quotes PowerShell also treats as quotes', () => {
    const path = 'C:\\Users\\O\u2019Brien\\entropia-agent-connector.exe'
    const location: ConnectorLocation = { installed: true, path, kind: 'bundled' }
    expect(connectorInvocation(CLIENT, location)).toBe(
      `& 'C:\\Users\\O\u2019\u2019Brien\\entropia-agent-connector.exe' --client ${CLIENT}`
    )
  })

  it('single-quotes a POSIX path with spaces and keeps the JSON escaped', () => {
    const program = POSIX_CONNECTOR.path
    expect(connectorInvocation(CLIENT, POSIX_CONNECTOR)).toBe(`'${program}' --client ${CLIENT}`)
    const lines = connectorRecipes(CLIENT, POSIX_CONNECTOR)
    expect(lines.find((r) => r.id === 'claude')!.line).toBe(
      `claude mcp add entropia -- '${program}' --client ${CLIENT}`
    )
    expect(lines.find((r) => r.id === 'codex')!.line).toBe(
      `codex mcp add entropia -- '${program}' --client ${CLIENT}`
    )
    expect(opencodeProgram(POSIX_CONNECTOR)).toEqual([program, '--client', CLIENT])
    expect(piConfig(POSIX_CONNECTOR)).toEqual({ command: program, args: ['--client', CLIENT] })
  })

  it('uses the MSIX alias name unchanged', () => {
    const program = MSIX_CONNECTOR.path
    expect(connectorInvocation(CLIENT, MSIX_CONNECTOR)).toBe(`${program} --client ${CLIENT}`)
    expect(connectorRecipes(CLIENT, MSIX_CONNECTOR).find((r) => r.id === 'claude')!.line).toBe(
      `claude mcp add entropia -- ${program} --client ${CLIENT}`
    )
    expect(opencodeProgram(MSIX_CONNECTOR)).toEqual([program, '--client', CLIENT])
    expect(piConfig(MSIX_CONNECTOR)).toEqual({ command: program, args: ['--client', CLIENT] })
  })

  it('keeps the bare command when no connector is installed', () => {
    expect(connectorInvocation(CLIENT, MISSING_CONNECTOR)).toBe(
      `entropia-agent-connector --client ${CLIENT}`
    )
    expect(connectorInvocation(CLIENT)).toBe(`entropia-agent-connector --client ${CLIENT}`)
    expect(connectorRecipes(CLIENT, MISSING_CONNECTOR)).toEqual([
      {
        id: 'claude',
        label: 'Claude Code',
        kind: 'command',
        line: `claude mcp add entropia -- entropia-agent-connector --client ${CLIENT}`,
      },
      {
        id: 'codex',
        label: 'Codex',
        kind: 'command',
        line: `codex mcp add entropia -- entropia-agent-connector --client ${CLIENT}`,
      },
      {
        id: 'opencode',
        label: 'OpenCode',
        kind: 'config',
        line: `{"mcp":{"entropia":{"type":"local","command":["entropia-agent-connector","--client","${CLIENT}"]}}}`,
      },
      {
        id: 'pi',
        label: 'Pi',
        kind: 'config',
        line: `{"mcpServers":{"entropia":{"command":"entropia-agent-connector","args":["--client","${CLIENT}"]}}}`,
      },
    ])
  })
})
