import { invoke } from '@tauri-apps/api/core'
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
}

export interface HelpAnswer {
  answer: string
  sources: HelpSource[]
  /** True when the backend answered from the bundled FAQ, with no network call (S10). */
  local: boolean
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
 * diagnostic report). History is capped at the last 6 turns (D4); diagnostics
 * travel only when the caller passes the previewed text (S5).
 */
export function helpAsk(
  question: string,
  history: HelpTurn[] = [],
  diagnostics?: string | null
): Promise<HelpAnswer> {
  const kept = history.slice(-6)
  if (diagnostics) {
    return invoke<HelpAnswer>('help_ask', { question, history: kept, diagnostics })
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

/** True when the failure is the backend's missing-key rejection (S6). */
export function isNoKeyError(error: unknown): boolean {
  if (typeof error === 'string') return error.includes(HELP_NO_OPENROUTER_KEY)
  if (error instanceof Error) return error.message.includes(HELP_NO_OPENROUTER_KEY)
  return false
}

export function openHelpManual(): Promise<void> {
  return openExternalUrl(HELP_MANUAL_URL)
}
