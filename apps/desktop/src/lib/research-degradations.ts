import { t, type I18nKey } from './i18n'

/**
 * Turns the engine's role warnings (`role_warnings` of a report: one entry per
 * role and error, with how many times it happened) into lines a researcher can
 * read: the step is named in plain words instead of its internal id, an
 * unreadable or cut-off model answer says what it means for the report and
 * what to try, and the raw engine error is kept apart as technical detail.
 */

export interface RoleWarning {
  role?: string
  error?: string
  times?: number
}

export interface ReadableDegradation {
  message: string
  /** Raw engine error, shown folded away; null when the message already says it all. */
  detail: string | null
}

const ROLE_LABELS: Record<string, I18nKey> = {
  investigador_principal: 'investigation.role.investigador_principal',
  asistente_archivo: 'investigation.role.asistente_archivo',
  asistente_bibliografia: 'investigation.role.asistente_bibliografia',
  asistente_redaccion: 'investigation.role.asistente_redaccion',
  asistente_validador: 'investigation.role.asistente_validador',
  redactor: 'investigation.role.redactor',
  prospeccion: 'investigation.role.prospeccion',
  recuperacion: 'investigation.role.recuperacion',
  fuente_bibliografica: 'investigation.role.fuente_bibliografica',
}

// Engine wording (EntropIA-Agent, src/investigacion.rs) this module recognizes.
const INVALID_OUTPUT_PREFIX = 'Salida inválida de '
const CUT_OFF_MARKER = 'se cortó al llegar al límite de salida'
// The cut-off message names the output cap it hit: «… límite de salida (16384 tokens)».
const CUT_OFF_CAP = /límite de salida \((\d+) tokens\)/
const ARCHIVE_RETRY_ERROR = 'el lote de archivo necesitó reintentos con lotes más chicos'

function roleLabel(role: string | undefined): string {
  const key = role ? ROLE_LABELS[role] : undefined
  return t(key ?? 'investigation.role.unknown')
}

function timesSuffix(times: number | undefined): string {
  const count = times ?? 1
  return count > 1 ? t('investigation.degradation.times', { count }) : ''
}

function lowerFirst(text: string): string {
  return text.charAt(0).toLocaleLowerCase() + text.slice(1)
}

function describe(warning: RoleWarning, error: string): ReadableDegradation {
  const step = roleLabel(warning.role)
  const times = timesSuffix(warning.times)

  if (error.startsWith(INVALID_OUTPUT_PREFIX)) {
    if (!error.includes(CUT_OFF_MARKER)) {
      return { message: t('investigation.degradation.unreadable', { step, times }), detail: error }
    }
    // A cut-off answer is fixed by a larger output cap, which the person sets
    // in the agent settings tab; say where, and what the cap was.
    const tokens = CUT_OFF_CAP.exec(error)?.[1]
    const cap = tokens ? t('investigation.degradation.cap', { tokens }) : ''
    const settingsTab = t('settings.agentParamsTab')
    return {
      message: t('investigation.degradation.cutOff', { step, times, settingsTab, cap }),
      detail: error,
    }
  }

  if (warning.role === 'asistente_archivo' && error === ARCHIVE_RETRY_ERROR) {
    return { message: t('investigation.degradation.archiveRetry', { step, times }), detail: null }
  }

  // Other engine messages are already prose; only the role id they may quote
  // is replaced by the step's name.
  const text = warning.role ? error.replaceAll(warning.role, lowerFirst(step)) : error
  return {
    message: t('investigation.degradation.other', { step, text, times }),
    detail: null,
  }
}

export function describeRoleWarnings(warnings: RoleWarning[]): ReadableDegradation[] {
  const seen = new Set<string>()
  const lines: ReadableDegradation[] = []

  for (const warning of warnings) {
    const error = warning.error?.trim()
    if (!error) continue
    const line = describe(warning, error)
    if (seen.has(line.message)) continue
    seen.add(line.message)
    lines.push(line)
  }

  return lines
}
