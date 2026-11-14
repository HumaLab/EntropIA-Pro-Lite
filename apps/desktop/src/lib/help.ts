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
 * excerpts travel only with `{ documents: true }` for this question (S12).
 */
/** Upper bound of `help_history_turns` (D10); Rust trims to the setting. */
export const HELP_HISTORY_MAX_TURNS = 12

export function helpAsk(
  question: string,
  history: HelpTurn[] = [],
  diagnostics?: string | null,
  options?: { documents?: boolean }
): Promise<HelpAnswer> {
  const kept = history.slice(-HELP_HISTORY_MAX_TURNS)
  if (diagnostics) {
    if (options?.documents) {
      return invoke<HelpAnswer>('help_ask', {
        question,
        history: kept,
        diagnostics,
        documents: true,
      })
    }
    return invoke<HelpAnswer>('help_ask', { question, history: kept, diagnostics })
  }
  if (options?.documents) {
    return invoke<HelpAnswer>('help_ask', { question, history: kept, documents: true })
  }
  return invoke<HelpAnswer>('help_ask', { question, history: kept })
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
