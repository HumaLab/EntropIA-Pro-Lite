import { invoke } from '@tauri-apps/api/core'

/**
 * Frontend client of the agent endpoint's settings surface (D11).
 *
 * Every wrapper is a thin `invoke`: the backend owns pairing, tokens and the
 * endpoint itself. No value that crosses this boundary ever contains a token —
 * the client view has fields for id, name, scopes and timestamps only (S4).
 */

/** What a paired client may do. Only `read` is granted in this release. */
export type AgentScope =
  'read' | 'local_write' | 'remote_spend' | 'external_write' | 'admin' | 'web'

/**
 * Where the client's token is stored (D14): the OS credential store, or the
 * owner-only file the app falls back to on a machine without one. Non-secret, so
 * the panel can say which it is.
 */
export type AgentTokenStore = 'keyring' | 'file'

/** One paired assistant, as the panel lists it. */
export interface AgentClient {
  id: string
  name: string
  scopes: AgentScope[]
  /** The keyring, or the protected file when this machine has no keyring. */
  tokenStore: AgentTokenStore
  /** Unix seconds. */
  createdAt: number
  /** Unix seconds; `null` while the client never connected. */
  lastUsedAt: number | null
}

export interface AgentAccessStatus {
  /** The persisted switch. */
  enabled: boolean
  /** Whether the loopback endpoint is listening right now. */
  running: boolean
  /** The ephemeral port while it listens. */
  port: number | null
  clients: AgentClient[]
}

export interface CreatedAgentClient {
  client: AgentClient
  status: AgentAccessStatus
}

/** The connector binary every assistant config names. */
export const CONNECTOR_COMMAND = 'entropia-agent-connector'

/**
 * Where the app found the connector this build installed (D15).
 *
 * `bundled` is a file next to the app executable (NSIS/MSI, .deb, macOS
 * `.app`); `msix_alias` is the Store package's execution alias, because the
 * packaged path inside WindowsApps cannot be launched from a normal command
 * line; `missing` means this build carried none.
 */
export interface ConnectorLocation {
  installed: boolean
  /** An absolute path, the alias name, or `null` when nothing was found. */
  path: string | null
  kind: 'bundled' | 'msix_alias' | 'missing'
}

/** The answer for a build without the connector: recipes keep the bare command. */
export const NO_CONNECTOR: ConnectorLocation = {
  installed: false,
  path: null,
  kind: 'missing',
}

/** What the panel shows for `agent_connector_location`. */
export function agentConnectorLocation(): Promise<ConnectorLocation> {
  return invoke<ConnectorLocation>('agent_connector_location')
}

/** What the panel shows for `agent_access_status`. */
export function agentAccessStatus(): Promise<AgentAccessStatus> {
  return invoke<AgentAccessStatus>('agent_access_status')
}

/** Flip the switch; the backend persists it and starts or stops the endpoint. */
export function agentAccessSetEnabled(enabled: boolean): Promise<AgentAccessStatus> {
  return invoke<AgentAccessStatus>('agent_access_set_enabled', { enabled })
}

/** Pair an assistant. The name is normalized here, once, before it leaves. */
export function agentClientCreate(name: string): Promise<CreatedAgentClient> {
  return invoke<CreatedAgentClient>('agent_client_create', { name: name.trim() })
}

/** Revoke a paired assistant; its token stops working on the next request. */
export function agentClientRevoke(clientId: string): Promise<AgentAccessStatus> {
  return invoke<AgentAccessStatus>('agent_client_revoke', { clientId })
}

/**
 * The program a config names, before any shell quoting: the installed path when
 * there is one, the bare command otherwise.
 */
export function connectorProgram(location?: ConnectorLocation | null): string {
  return location?.installed && location.path ? location.path : CONNECTOR_COMMAND
}

/**
 * Quote a program for the shell the person will paste it into.
 *
 * The location carries the platform implicitly: a Windows path starts with a
 * drive letter, and Windows file names cannot contain `"`, so wrapping is the
 * whole job there. Anything with spaces, accents or quotes in a POSIX path goes
 * in single quotes, where only `'` itself needs escaping. A bare command and a
 * plain path stay untouched, so the common line is exactly what it always was.
 */
export function shellQuote(program: string): string {
  if (/^[A-Za-z]:[\\/]/.test(program)) return `"${program}"`
  if (/^[\w./-]+$/.test(program)) return program
  return `'${program.replace(/'/g, `'\\''`)}'`
}

/**
 * The connector invocation: the client id is the only credential-ish value a
 * config carries, and it is useless without the keyring entry the app wrote.
 */
export function connectorInvocation(clientId: string, location?: ConnectorLocation | null): string {
  return `${shellQuote(connectorProgram(location))} --client ${clientId}`
}

/**
 * How each supported client names the connector. `command` entries are the
 * one-line CLI form; `config` entries are a minimal JSON block — command and
 * args only, no headers, no environment, no token — to paste into the client's
 * own configuration file.
 */
export type ConnectorRecipeId = 'claude' | 'codex' | 'opencode' | 'pi'

export interface ConnectorRecipe {
  id: ConnectorRecipeId
  label: string
  kind: 'command' | 'config'
  line: string
}

/** One ready-to-copy line per supported client, in a stable order. */
export function connectorRecipes(
  clientId: string,
  location?: ConnectorLocation | null
): ConnectorRecipe[] {
  const program = connectorProgram(location)
  const command = connectorInvocation(clientId, location)
  // JSON strings are escaped by the serializer, never by hand: a Windows path
  // arrives at the client with its backslashes and non-ASCII intact.
  const args = JSON.stringify([program, '--client', clientId])
  return [
    {
      id: 'claude',
      label: 'Claude Code',
      kind: 'command',
      line: `claude mcp add entropia -- ${command}`,
    },
    {
      id: 'codex',
      label: 'Codex',
      kind: 'command',
      line: `codex mcp add entropia -- ${command}`,
    },
    {
      id: 'opencode',
      label: 'OpenCode',
      kind: 'config',
      // OpenCode names the whole invocation in `command` (program plus args).
      line: `{"mcp":{"entropia":{"type":"local","command":${args}}}}`,
    },
    {
      id: 'pi',
      label: 'Pi',
      kind: 'config',
      // Pi follows the common MCP shape: program in `command`, arguments apart.
      line: `{"mcpServers":{"entropia":{"command":${JSON.stringify(program)},"args":${JSON.stringify(['--client', clientId])}}}}`,
    },
  ]
}
