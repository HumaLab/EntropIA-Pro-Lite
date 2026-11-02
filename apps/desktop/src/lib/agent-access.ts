import { invoke } from '@tauri-apps/api/core'

/**
 * Frontend client of the agent endpoint's settings surface (D11).
 *
 * Every wrapper is a thin `invoke`: the backend owns pairing, tokens and the
 * endpoint itself. No value that crosses this boundary ever contains a token —
 * the client view has fields for id, name, scopes and timestamps only (S4).
 */

/**
 * What a paired client may do. Every pairing gets `read`; the owner can add
 * `local_write` and `remote_spend` (see {@link AgentGrants}).
 */
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
  /** Folders assistants may import from, as real paths. */
  importFolders: string[]
}

export interface CreatedAgentClient {
  client: AgentClient
  status: AgentAccessStatus
}

/**
 * The scopes the owner can grant from this panel. `read` comes with every
 * pairing and the other scopes are not offered.
 */
export interface AgentGrants {
  /** Prepare, pause, cancel and reprioritize processing batches. */
  localWrite: boolean
  /** Ask to start paid processing; each request is still approved in the app. */
  remoteSpend: boolean
}

/** The grants a client currently holds. */
export function grantsOf(client: AgentClient): AgentGrants {
  return {
    localWrite: client.scopes.includes('local_write'),
    remoteSpend: client.scopes.includes('remote_spend'),
  }
}

/** Let assistants import from a folder; answers with the fresh status. */
export function agentImportFolderAdd(path: string): Promise<AgentAccessStatus> {
  return invoke<AgentAccessStatus>('agent_import_folder_add', { path })
}

/** Stop assistant imports from a folder; answers with the fresh status. */
export function agentImportFolderRemove(path: string): Promise<AgentAccessStatus> {
  return invoke<AgentAccessStatus>('agent_import_folder_remove', { path })
}

/** Replace one client's grants; answers with the fresh status. */
export function agentClientSetGrants(
  clientId: string,
  grants: AgentGrants
): Promise<AgentAccessStatus> {
  return invoke<AgentAccessStatus>('agent_client_set_grants', { clientId, grants })
}

/** What an assistant asked to do with a processing job (D3). */
export type AgentApprovalAction =
  | { kind: 'start' }
  | { kind: 'resume' }
  | { kind: 'retry'; task_id: string | null; failed_only: boolean }
  | {
      kind: 'replace_text'
      item_id: string
      asset_id: string
      text_hash: string
      new_text_hash: string
    }

export type AgentApprovalStatus = 'pending' | 'approved' | 'failed' | 'rejected' | 'expired'

/**
 * A request for paid work, waiting for the owner. Bound to the client, the job
 * and the job revision the assistant saw; it carries names and counts only.
 */
export interface AgentApproval {
  id: string
  clientId: string
  clientName: string
  action: AgentApprovalAction
  jobId: string
  revision: number
  operations: string[]
  collections: string[]
  files: number
  status: AgentApprovalStatus
  /** Unix milliseconds. */
  createdAt: number
  expiresAt: number
  decidedAt: number | null
  /** Why an approved action did not run. */
  failure: string | null
  /** The start of a proposed text (`replace_text`). */
  preview: string | null
}

/** The event the backend emits whenever the approvals change. */
export const AGENT_APPROVALS_CHANGED_EVENT = 'agent-approvals:changed'

/** The requests waiting for the owner, oldest first. */
export function agentApprovalsPending(): Promise<AgentApproval[]> {
  return invoke<AgentApproval[]>('agent_approvals_pending')
}

/** The owner's answer; approving runs the action once. */
export function agentApprovalDecide(approvalId: string, approve: boolean): Promise<AgentApproval> {
  return invoke<AgentApproval>('agent_approval_decide', { approvalId, approve })
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

/** A Windows install path: a drive letter, then a separator. */
export function isWindowsProgram(program: string): boolean {
  return /^[A-Za-z]:[\\/]/.test(program)
}

/**
 * Quote a program for the shell the person will paste it into.
 *
 * The location carries the platform implicitly: a Windows path starts with a
 * drive letter, and the lines target PowerShell, the default Windows shell. A
 * double-quoted string there still expands `$name` and `$(...)`, so the path goes
 * in single quotes, which are literal; an embedded single quote (PowerShell also
 * counts the typographic ones) is doubled. Anything with spaces, accents or
 * quotes in a POSIX path goes in single quotes too, where only `'` itself needs
 * escaping. A bare command and a plain path stay untouched, so the common line
 * is exactly what it always was.
 */
export function shellQuote(program: string): string {
  if (isWindowsProgram(program))
    return `'${program.replace(/['\u2018\u2019\u201A\u201B]/g, '$&$&')}'`
  if (/^[\w./-]+$/.test(program)) return program
  return `'${program.replace(/'/g, `'\\''`)}'`
}

/**
 * The connector invocation: the client id is the only credential-ish value a
 * config carries, and it is useless without the keyring entry the app wrote.
 * PowerShell parses a quoted path followed by arguments as an expression, so a
 * Windows path takes the call operator; the `claude mcp add … --` lines pass the
 * quoted path as a plain argument and must not.
 */
export function connectorInvocation(clientId: string, location?: ConnectorLocation | null): string {
  const program = connectorProgram(location)
  const call = isWindowsProgram(program) ? '& ' : ''
  return `${call}${shellQuote(program)} --client ${clientId}`
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
  const command = `${shellQuote(program)} --client ${clientId}`
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
