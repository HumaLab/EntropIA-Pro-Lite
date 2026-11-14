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

/** One tool step the help agent ran, in order (T22, T26): the tool,
 * the compact JSON args and the outcome. A `pending` step (T26) waits for
 * the owner in the MCP approval dialog; `target` names its object when
 * known (never a UUID) and `approvalId` follows the outcome. Both are
 * absent on older payloads. */
export type HelpStepStatus = 'ok' | 'pending' | 'denied' | 'failed' | 'expired'

export interface HelpStep {
  tool: string
  args: string
  status: HelpStepStatus
  /** The human name of the step's object, when known (T26). */
  target?: string
  /** The MCP approval id, for steps that wait for the owner (T26). */
  approvalId?: string
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

/** One live agent step the backend emits while the run is in flight
 * (T24, T26). Same shape as `HelpStep` plus the run id; the final
 * `HelpAnswer.steps` stays the source of truth at the end. A `pending`
 * step carries the human `target` and the `approvalId` it waits for. */
export interface HelpStepEvent {
  requestId: string
  tool: string
  args: string
  status: HelpStepStatus
  target?: string
  approvalId?: string
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

/** One decided MCP approval the backend followed after the answer
 * (T26): which request, how it ended and a clear Spanish message naming
 * the target. Rejected and expired ran nothing; failed names the reason. */
export type HelpApprovalOutcomeStatus = 'approved' | 'rejected' | 'expired' | 'failed'

export interface HelpApprovalOutcome {
  approvalId: string
  status: HelpApprovalOutcomeStatus
  message: string
}

/** The backend event carrying decided MCP approval outcomes (T26). */
export const HELP_APPROVAL_OUTCOME_EVENT = 'help://approval-outcome'

/** Listens to the backend's MCP approval outcomes (T26). */
export function listenHelpApprovalOutcome(
  onOutcome: (outcome: HelpApprovalOutcome) => void
): Promise<UnlistenFn> {
  return listen<HelpApprovalOutcome>(HELP_APPROVAL_OUTCOME_EVENT, (event) =>
    onOutcome(event.payload)
  )
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
 * Human-readable label of one agent step (T24, T26), in the UI language:
 * what the assistant did, with the query, name or path the call carried,
 * and a mark when the call was denied or failed. Steps for tools that
 * only REQUEST something (the MCP_APPROVAL class: deletes, text updates,
 * imports, paid processing) say so while pending («Pidió borrar …») and
 * as a result once the owner decides («Borró …» / «Borrado rechazado»).
 * `target` (T26) names the object when known; a raw UUID in the args is
 * never shown when a name exists — the generic («una colección») wins.
 * Unknown tools fall back to their own name (never blank, never a crash).
 * Pure: no stores, no I/O.
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
  const target = (step.target ?? '').trim()
  // An MCP approval step never shows the raw id: the name the backend
  // resolved, or the generic fallback.
  const approvalLabel = approvalStepLabel(step.tool, parsed, target, step.status, en, quote)
  if (approvalLabel !== null) return approvalLabel
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
    case 'documents_index':
      label = en ? 'Reindexed a document' : 'Reindexó un documento'
      break
    case 'processing_control': {
      // Pause/cancel apply at once (LocalWrite, never pending): resume/
      // retry are approval steps handled above. The job id is the only
      // handle a job has, so it stays in the label.
      const action = stepField(parsed, ['action'])
      const jobId = stepField(parsed, ['jobId'])
      const job = jobId.length > 0 ? ` ${quote(jobId)}` : ''
      if (action === 'pause') label = en ? `Paused the${job} job` : `Pausó el procesamiento${job}`
      else if (action === 'cancel')
        label = en ? `Cancelled the${job} job` : `Canceló el procesamiento${job}`
      else label = en ? 'Controlled a processing job' : 'Controló un procesamiento'
      break
    }
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
  if (step.status === 'pending') return en ? `${label} (pending)` : `${label} (pendiente)`
  if (step.status === 'expired') return en ? `${label} (expired)` : `${label} (vencido)`
  if (step.status === 'denied') return en ? `${label} (denied)` : `${label} (denegado)`
  if (step.status === 'failed') return en ? `${label} (failed)` : `${label} (falló)`
  return label
}

/** Whether a step waits for (or waited for) the MCP approval dialog (T26):
 * the tools that only REQUEST something. `processing_control` counts only
 * for resume/retry (pause/cancel apply at once through the inline card). */
function isApprovalStep(tool: string, args: Record<string, unknown>): boolean {
  if (
    tool === 'collections_delete' ||
    tool === 'documents_delete' ||
    tool === 'documents_text_update' ||
    tool === 'documents_import' ||
    tool === 'processing_start'
  ) {
    return true
  }
  if (tool === 'processing_control') {
    const action = typeof args['action'] === 'string' ? (args['action'] as string) : ''
    return action === 'resume' || action === 'retry'
  }
  return false
}

/** A label for an MCP approval step (T26), or `null` for any other tool.
 * Pending says what was REQUESTED («Pidió borrar …»); decided states say
 * what HAPPENED («Borró …» / «Borrado rechazado»). The `target` name wins;
 * without it the generic wins — a raw UUID is never shown. */
function approvalStepLabel(
  tool: string,
  args: Record<string, unknown>,
  target: string,
  status: HelpStep['status'],
  en: boolean,
  quote: (text: string) => string
): string | null {
  if (!isApprovalStep(tool, args)) return null
  const hasTarget = target.length > 0
  const isCount = /^\d+\s+documentos/.test(target)
  const quoted = hasTarget && !isCount ? quote(target) : ''
  switch (tool) {
    case 'collections_delete': {
      const what = hasTarget
        ? en
          ? `the ${quoted} collection`
          : `la colección ${quoted}`
        : en
          ? 'a collection'
          : 'una colección'
      if (status === 'pending') return en ? `Asked to delete ${what}` : `Pidió borrar ${what}`
      if (status === 'ok') return en ? `Deleted ${what}` : `Borró ${what}`
      if (status === 'denied')
        return en ? `Deletion rejected: ${what}` : `Borrado rechazado: ${what}`
      if (status === 'expired')
        return en ? `The request to delete ${what} expired` : `El pedido para borrar ${what} venció`
      return en ? `Could not delete ${what}` : `No se pudo borrar ${what}`
    }
    case 'documents_delete': {
      if (isCount) {
        if (status === 'pending') return en ? `Asked to delete ${target}` : `Pidió borrar ${target}`
        if (status === 'ok') return en ? `Deleted ${target}` : `Borró ${target}`
        if (status === 'denied')
          return en ? `Deletion rejected: ${target}` : `Borrado rechazado: ${target}`
        if (status === 'expired')
          return en
            ? `The deletion request expired: ${target}`
            : `El pedido para borrar venció: ${target}`
        return en ? `Could not delete ${target}` : `No se pudo borrar ${target}`
      }
      const what = hasTarget
        ? en
          ? `the ${quoted} document`
          : `el documento ${quoted}`
        : en
          ? 'documents'
          : 'documentos'
      if (status === 'pending') return en ? `Asked to delete ${what}` : `Pidió borrar ${what}`
      if (status === 'ok') return en ? `Deleted ${what}` : `Borró ${what}`
      if (status === 'denied')
        return en ? `Deletion rejected: ${what}` : `Borrado rechazado: ${what}`
      if (status === 'expired')
        return en
          ? `The deletion request expired: ${what}`
          : `El pedido para borrar venció: ${what}`
      return en ? `Could not delete ${what}` : `No se pudo borrar ${what}`
    }
    case 'documents_text_update': {
      const what = hasTarget ? quoted : en ? 'a document' : 'un documento'
      if (status === 'pending')
        return en ? `Asked to update the text of ${what}` : `Pidió actualizar el texto de ${what}`
      if (status === 'ok')
        return en ? `Updated the text of ${what}` : `Actualizó el texto de ${what}`
      if (status === 'denied')
        return en ? `Text update rejected: ${what}` : `Actualización rechazada: ${what}`
      if (status === 'expired')
        return en
          ? `The text update request expired: ${what}`
          : `El pedido para actualizar venció: ${what}`
      return en
        ? `Could not update the text of ${what}`
        : `No se pudo actualizar el texto de ${what}`
    }
    case 'documents_import': {
      const what = hasTarget ? quoted : en ? 'documents' : 'documentos'
      const into = hasTarget ? (en ? ` into ${quoted}` : ` a ${quoted}`) : ''
      if (status === 'pending')
        return en ? `Asked to import documents${into}` : `Pidió importar documentos${into}`
      if (status === 'ok') return en ? `Imported ${what}${into}` : `Importó ${what}${into}`
      if (status === 'denied') return en ? `Import rejected${into}` : `Importación rechazada${into}`
      if (status === 'expired')
        return en ? `The import request expired${into}` : `El pedido para importar venció${into}`
      return en ? `Could not import ${what}${into}` : `No se pudo importar ${what}${into}`
    }
    case 'processing_start': {
      if (status === 'pending')
        return en ? 'Asked to start processing' : 'Pidió iniciar un procesamiento'
      if (status === 'ok') return en ? 'Started a processing job' : 'Inició un procesamiento'
      if (status === 'denied') return en ? 'Start rejected' : 'Inicio rechazado'
      if (status === 'expired')
        return en ? 'The start request expired' : 'El pedido para iniciar venció'
      return en ? 'Could not start processing' : 'No se pudo iniciar el procesamiento'
    }
    case 'processing_control': {
      const action = typeof args['action'] === 'string' ? (args['action'] as string) : ''
      const resume = action === 'resume'
      if (status === 'pending')
        return resume
          ? en
            ? 'Asked to resume processing'
            : 'Pidió reanudar el procesamiento'
          : en
            ? 'Asked to retry processing'
            : 'Pidió reintentar el procesamiento'
      if (status === 'ok')
        return resume
          ? en
            ? 'Resumed processing'
            : 'Reanudó el procesamiento'
          : en
            ? 'Retried processing'
            : 'Reintentó el procesamiento'
      if (status === 'denied')
        return resume
          ? en
            ? 'Resume rejected'
            : 'Reanudación rechazada'
          : en
            ? 'Retry rejected'
            : 'Reintento rechazado'
      if (status === 'expired')
        return resume
          ? en
            ? 'The resume request expired'
            : 'El pedido para reanudar venció'
          : en
            ? 'The retry request expired'
            : 'El pedido para reintentar venció'
      return resume
        ? en
          ? 'Could not resume processing'
          : 'No se pudo reanudar el procesamiento'
        : en
          ? 'Could not retry processing'
          : 'No se pudo reintentar el procesamiento'
    }
    default:
      return null
  }
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
