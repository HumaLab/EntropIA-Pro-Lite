import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { openExternalUrl } from './external-links'

/** The exact string the backend sends when there is no OpenRouter key (S6). */
export const HELP_NO_OPENROUTER_KEY = 'help_no_openrouter_key'

/** The published user manual the no-key page links to. */
export const HELP_MANUAL_URL =
  'https://humalab.github.io/EntropIA-Pro-Lite/manual-usuario/manual-usuario.html'

export interface HelpTurn {
  role: 'user' | 'assistant'
  content: string
}

export interface HelpSource {
  chapter: string
  title: string
  /** Where the source comes from; absent means manual (older payloads). */
  kind?: 'manual' | 'screenMap' | 'faq'
}

/** One title of the user's own documents the agent read with the CONTENT tools (T22). */
export interface HelpDocument {
  title: string
}

/** One tool step the help agent ran, in order (T22): the tool, the compact
 * JSON args and the outcome. Tool names are shown as-is for now (T24 polishes). */
export interface HelpStep {
  tool: string
  args: string
  status: 'ok' | 'denied' | 'failed'
}

export interface HelpAnswer {
  answer: string
  sources: HelpSource[]
  /** True when the backend answered from the bundled FAQ, with no network call (S10). */
  local: boolean
  /** Titles of the user's documents the agent read (T22); empty unless the box was ticked. */
  documents: HelpDocument[]
  /** Every tool call the agent made, in order (T22); empty for local and plain answers. */
  steps: HelpStep[]
  /** True when the round cap forced the final answer (T22). */
  roundLimitReached: boolean
  /** True when the provider rejected tool use and the answer is plain, without actions (T22). */
  toolsUnavailable: boolean
}

export interface HelpStatus {
  hasOpenrouterKey: boolean
}

/** Whether the assistant can run on the user's own OpenRouter key (S3, S6). */
export function helpStatus(): Promise<HelpStatus> {
  return invoke<HelpStatus>('help_status')
}

/** The exact diagnostic text `help_ask` would send (D3, S5). */
export function helpDiagnosticsPreview(): Promise<string> {
  return invoke<string>('help_diagnostics_preview')
}

/**
 * Answers a help question from the manual excerpts (and the opted-in
 * diagnostic report). History is capped at the last HELP_HISTORY_MAX_TURNS
 * turns, the most the settings allow; the backend applies the configured cap
 * (`help_history_turns`, D10); diagnostics
 * travel only when the caller passes the previewed text (S5); document
 * excerpts travel only with `{ documents: true }` for this question (S12);
 * `requestId` (T24) is the frontend-generated id of this question: the
 * backend emits live `help://step` events under it and `helpStop` ends the
 * run through it.
 */
/** Upper bound of `help_history_turns` (D10); Rust trims to the setting. */
export const HELP_HISTORY_MAX_TURNS = 12

export function helpAsk(
  question: string,
  history: HelpTurn[] = [],
  diagnostics?: string | null,
  options?: { documents?: boolean; requestId?: string }
): Promise<HelpAnswer> {
  const kept = history.slice(-HELP_HISTORY_MAX_TURNS)
  const extra: { diagnostics?: string; documents?: boolean; requestId?: string } = {}
  if (diagnostics) extra.diagnostics = diagnostics
  if (options?.documents) extra.documents = true
  if (options?.requestId) extra.requestId = options.requestId
  return invoke<HelpAnswer>('help_ask', { question, history: kept, ...extra })
}

/**
 * Every bundled FAQ entry as a starter button (S10): answered locally, so
 * the list needs no key and no network.
 */
export interface HelpFaqItem {
  group: string
  question: string
}

export function helpFaqList(): Promise<HelpFaqItem[]> {
  return invoke<HelpFaqItem[]>('help_faq_list')
}

/** One live agent step the backend emits while the run is in flight (T24).
 * Same shape as `HelpStep` plus the run id; the final `HelpAnswer.steps`
 * stays the source of truth at the end. */
export interface HelpStepEvent {
  requestId: string
  tool: string
  args: string
  status: 'ok' | 'denied' | 'failed'
}

/** The backend event carrying live agent steps (T24). */
export const HELP_STEP_EVENT = 'help://step'

/** Stops a running help agent turn (T24 «Detener»): the run answers quickly
 * with a «Detenido» message and the steps done so far. */
export function helpStop(requestId: string): Promise<void> {
  return invoke<void>('help_stop', { requestId })
}

/** Listens to the backend's live agent steps (T24). */
export function listenHelpSteps(onStep: (step: HelpStepEvent) => void): Promise<UnlistenFn> {
  return listen<HelpStepEvent>(HELP_STEP_EVENT, (event) => onStep(event.payload))
}

/** A fresh id for one help question (T24): the run the live steps and
 * `helpStop` belong to. */
export function newHelpRequestId(): string {
  try {
    const uuid = crypto.randomUUID()
    if (uuid) return uuid
  } catch {
    return `help-${Date.now().toString(36)}-${Math.floor(Math.random() * 0xffffff).toString(36)}`
  }
  return `help-${Date.now().toString(36)}-${Math.floor(Math.random() * 0xffffff).toString(36)}`
}

/** The UI language a step label is rendered in. */
export type HelpStepLocale = 'es' | 'en'

/** First non-blank string field of parsed tool args, for the label detail. */
function stepField(args: Record<string, unknown>, keys: string[]): string {
  for (const key of keys) {
    const value = args[key]
    if (typeof value === 'string' && value.trim().length > 0) return value.trim()
  }
  return ''
}

/**
 * Human-readable label of one agent step (T24), in the UI language: what
 * the assistant did, with the query, name or path the call carried, and a
 * mark when the call was denied or failed. Unknown tools fall back to
 * their own name (never blank, never a crash). Pure: no stores, no I/O.
 */
export function describeHelpStep(step: HelpStep, locale: HelpStepLocale): string {
  const en = locale === 'en'
  const quote = (text: string) => (en ? `“${text}”` : `«${text}»`)
  let parsed: Record<string, unknown> = {}
  try {
    const value: unknown = JSON.parse(step.args)
    if (value && typeof value === 'object' && !Array.isArray(value)) {
      parsed = value as Record<string, unknown>
    }
  } catch {
    parsed = {}
  }
  const query = stepField(parsed, ['query'])
  const name = stepField(parsed, ['name', 'title', 'collectionId', 'itemId', 'jobId', 'path'])
  const path = stepField(parsed, ['path', 'path_prefix', 'pathPrefix'])
  let label: string
  switch (step.tool) {
    case 'collections_list':
      label = en ? 'Listed collections' : 'Listó colecciones'
      break
    case 'collections_create':
      label =
        name.length > 0
          ? en
            ? `Created the ${quote(name)} collection`
            : `Creó la colección ${quote(name)}`
          : en
            ? 'Created a collection'
            : 'Creó una colección'
      break
    case 'collections_update':
      label =
        name.length > 0
          ? en
            ? `Updated the ${quote(name)} collection`
            : `Actualizó la colección ${quote(name)}`
          : en
            ? 'Updated a collection'
            : 'Actualizó una colección'
      break
    case 'collections_delete':
      label =
        name.length > 0
          ? en
            ? `Deleted the ${quote(name)} collection`
            : `Eliminó la colección ${quote(name)}`
          : en
            ? 'Deleted a collection'
            : 'Eliminó una colección'
      break
    case 'import_sources_list':
      label = en ? 'Listed import sources' : 'Listó orígenes de importación'
      break
    case 'items_list':
      label = en ? 'Listed documents' : 'Listó documentos'
      break
    case 'items_get':
      label = en ? 'Read a document' : 'Leyó un documento'
      break
    case 'items_update':
      label =
        name.length > 0
          ? en
            ? `Updated the ${quote(name)} document`
            : `Actualizó el documento ${quote(name)}`
          : en
            ? 'Updated a document'
            : 'Actualizó un documento'
      break
    case 'jobs_list':
      label = en ? 'Listed jobs' : 'Listó trabajos'
      break
    case 'jobs_get':
      label = en ? 'Checked a job' : 'Consultó un trabajo'
      break
    case 'search_items':
    case 'search_passages':
      label =
        query.length > 0
          ? en
            ? `Searched documents: ${quote(query)}`
            : `Buscó documentos: ${quote(query)}`
          : en
            ? 'Searched documents'
            : 'Buscó documentos'
      break
    case 'documents_get_text':
      label = en ? 'Read text from a document' : 'Leyó texto de un documento'
      break
    case 'documents_text_update':
      label = en ? 'Updated a document’s text' : 'Actualizó el texto de un documento'
      break
    case 'documents_import':
      label = en ? 'Imported documents' : 'Importó documentos'
      break
    case 'documents_delete':
      label = en ? 'Deleted documents' : 'Eliminó documentos'
      break
    case 'documents_index':
      label = en ? 'Reindexed a document' : 'Reindexó un documento'
      break
    case 'processing_prepare': {
      const operations = Array.isArray(parsed['operations'])
        ? (parsed['operations'] as unknown[])
            .filter((item): item is string => typeof item === 'string')
            .join(', ')
        : ''
      label =
        operations.length > 0
          ? en
            ? `Prepared a processing job (${operations})`
            : `Preparó un procesamiento (${operations})`
          : en
            ? 'Prepared a processing job'
            : 'Preparó un procesamiento'
      break
    }
    case 'processing_start':
      label = en ? 'Started a processing job' : 'Inició un procesamiento'
      break
    case 'processing_control': {
      const action = stepField(parsed, ['action'])
      const job = name
      const target = job.length > 0 ? ` ${quote(job)}` : ''
      if (action === 'pause')
        label = en ? `Paused the${target} job` : `Pausó el procesamiento${target}`
      else if (action === 'cancel')
        label = en ? `Cancelled the${target} job` : `Canceló el procesamiento${target}`
      else if (action === 'resume')
        label = en ? `Resumed the${target} job` : `Reanudó el procesamiento${target}`
      else if (action === 'retry')
        label = en ? `Retried the${target} job` : `Reintentó el procesamiento${target}`
      else label = en ? 'Controlled a processing job' : 'Controló un procesamiento'
      break
    }
    case 'approvals_status':
      label = en ? 'Checked pending approvals' : 'Consultó aprobaciones pendientes'
      break
    case 'entropia_status':
      label = en ? 'Checked the status' : 'Consultó el estado'
      break
    case 'repo_tree':
      label =
        path.length > 0
          ? en
            ? `Listed repository files: ${path}`
            : `Listó archivos del repositorio: ${path}`
          : en
            ? 'Listed repository files'
            : 'Listó archivos del repositorio'
      break
    case 'repo_read':
      label =
        path.length > 0
          ? en
            ? `Read from the repository: ${path}`
            : `Leyó del repositorio: ${path}`
          : en
            ? 'Read from the repository'
            : 'Leyó del repositorio'
      break
    case 'hlab_entropia_page':
      label = en ? 'Checked the HLab page' : 'Consultó la página de HLab'
      break
    default:
      label = step.tool
      break
  }
  if (step.status === 'denied') return en ? `${label} (denied)` : `${label} (denegado)`
  if (step.status === 'failed') return en ? `${label} (failed)` : `${label} (falló)`
  return label
}

/** One inline approval the help agent asks the owner for (T22, D12). */
export interface HelpApprovalRequest {
  id: string
  tool: string
  summary: string
}

/** The backend event carrying inline approval requests (T22, D12). */
export const HELP_APPROVAL_EVENT = 'help://approval'

/** Answers one inline approval card: `id` is the `help://approval` payload's. */
export function helpApprovalAnswer(id: string, approved: boolean): Promise<void> {
  return invoke<void>('help_approval_answer', { id, approved })
}

/** Listens to the backend's inline approval requests (T22, D12). */
export function listenHelpApproval(
  onRequest: (request: HelpApprovalRequest) => void
): Promise<UnlistenFn> {
  return listen<HelpApprovalRequest>(HELP_APPROVAL_EVENT, (event) => onRequest(event.payload))
}

/** True when the failure is the backend's missing-key rejection (S6). */
export function isNoKeyError(error: unknown): boolean {
  if (typeof error === 'string') return error.includes(HELP_NO_OPENROUTER_KEY)
  if (error instanceof Error) return error.message.includes(HELP_NO_OPENROUTER_KEY)
  return false
}

export function openHelpManual(): Promise<void> {
  return openExternalUrl(HELP_MANUAL_URL)
}
