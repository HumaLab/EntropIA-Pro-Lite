import { invoke } from '@tauri-apps/api/core'

export type ResearchJobStatus =
  | 'planned'
  | 'running'
  | 'paused'
  | 'awaiting_human'
  | 'done'
  | 'failed'
export type ResearchJobPhase =
  | 'coverage'
  | 'design'
  | 'plan'
  | 'clarification'
  | 'execution'
  | 'verification'
  | 'report'

/** Modalidad de informe: perfila las preguntas, el plan y la redacción. */
export interface ResearchModality {
  id: string
  name: string
}

/** Una pregunta de la ronda de clarificación. */
export interface ResearchQuestion {
  id: string
  axis: string
  text: string
  rationale: string
}

/** La respuesta del investigador a una pregunta de la ronda. */
export interface ResearchAnswer {
  id: string
  text: string
}

/**
 * Contenido del artefacto `clarification_round`. Sin `answers` la ronda sigue
 * abierta: el job no avanza hasta que se responde con `researchAnswer`.
 */
export interface ResearchClarificationRound {
  questions: ResearchQuestion[]
  answers?: ResearchAnswer[]
}

/** Una cita: el pasaje reproducido o su referencia, con la numeración del motor. */
export interface ResearchCitation {
  n: number
  evidence_id: string
  /** Item del corpus: lo que permite abrir el documento desde la cita. */
  item_id: string
  chunk_id: string
  collection?: string
  title: string
  text?: string
  start: number
  end: number
  truncated?: boolean
  date?: string
  date_precision?: string
}

export interface ResearchReportSection {
  title: string
  text: string
  claim_ids: string[]
  quotes: ResearchCitation[]
}

/**
 * Contenido del artefacto `report`.
 *
 * El motor decide **qué** se cita: la numeración, los pasajes y las
 * referencias vienen armados. `markdown` es el documento canónico —el mismo
 * que se escribe en disco—; la vista lo usa para exportar y pinta la versión
 * estructurada en pantalla.
 */
export interface ResearchReportContent {
  markdown?: string
  report?: { title?: string; sections?: ResearchReportSection[]; references?: ResearchCitation[] }
  coverage?: { collections?: ResearchCollectionSummary[] }
  coverage_warning?: { sufficient?: boolean; rationale?: string; gaps?: string[] }
  archive_limitations?: { text?: string; reason?: string }[]
  role_warnings?: { role?: string; error?: string; times?: number }[]
  profile?: { id?: string; name?: string; bias?: string }
  clarification?: ResearchClarificationRound
}

export interface ResearchCollectionSummary {
  id: string
  name: string
  items: number
  items_with_chunks: number
  chunks: number
}

export interface ResearchJobSummary {
  id: string
  /** Nombre que le puso el investigador; sin uno, la pregunta. */
  title: string
  question: string
  status: ResearchJobStatus
  phase: ResearchJobPhase
  llm_calls: number
  max_llm_calls: number | null
  cost: number | null
  max_cost: number | null
  close_reason: string | null
}

export interface ResearchEvent {
  id: string
  kind: string
  payload: unknown
  timestamp: number
}

export interface ResearchArtifact {
  id: string
  kind: string
  version: number
  obsolete: boolean
  content: unknown
}

export interface ResearchGate {
  id: string
  kind: string
  artifact_id: string
  status: 'pending' | 'approved' | 'rejected'
}

export interface ResearchSourceSummary {
  item_id: string
  title: string
}

export interface ResearchSourcePath {
  path: string
  page: number | null
}

export interface ResearchListResponse {
  jobs: ResearchJobSummary[]
  collections: ResearchCollectionSummary[]
  modalidades: ResearchModality[]
}

export interface ResearchDetailResponse {
  job: ResearchJobSummary
  events: ResearchEvent[]
  artifacts: ResearchArtifact[]
  gates: ResearchGate[]
  sources: ResearchSourceSummary[]
}

export interface ResearchContextSource {
  index: number
  assetId: string | null
  itemId: string
  itemTitle: string
  collectionId: string
  collectionName: string
  snippet: string
}

export interface ResearchContextMessage {
  role: 'user' | 'assistant'
  content: string
  sources?: ResearchContextSource[]
}

export interface ResearchHandoffDraft {
  question: string
  project: string
  context: ResearchContextMessage[] | null
}

export interface ResearchCreateRequest {
  /** Nombre de la investigación. Ausente o vacío, la pregunta lo cubre. */
  title?: string
  question: string
  project: string
  collection_ids: string[]
  max_llm_calls: number
  max_cost: number | null
  context: ResearchContextMessage[] | null
  /** Modalidad de informe; ausente equivale a `general`. */
  modalidad?: string
}

export interface ResearchAnswerRequest {
  job_id: string
  answers: ResearchAnswer[]
}

export interface ResearchGetRequest {
  job_id: string
}

export interface ResearchDecisionRequest {
  job_id: string
  gate_id: string
  approve: boolean
}

export interface ResearchReviseRequest {
  job_id: string
  artifact_id: string
  content: unknown
}

export type ResearchMutationRequest =
  | ({ op: 'create' } & ResearchCreateRequest)
  | ({ op: 'decision' } & ResearchDecisionRequest)
  | ({ op: 'revise' } & ResearchReviseRequest)
  | ({ op: 'answer' } & ResearchAnswerRequest)
  | { op: 'update_budget'; job_id: string; max_llm_calls: number; max_cost: number | null }
  | { op: 'list' }
  | { op: 'get' | 'pause' | 'resume' | 'cancel' | 'advance' | 'continue_coverage'; job_id: string }
  | { op: 'source'; job_id: string; item_id: string }
  | { op: 'delete'; job_id: string }

let researchHandoffDraft: ResearchHandoffDraft | null = null

export function setResearchHandoff(draft: ResearchHandoffDraft): void {
  researchHandoffDraft = draft
}

export function takeResearchHandoff(): ResearchHandoffDraft | null {
  const draft = researchHandoffDraft
  researchHandoffDraft = null
  return draft
}

export function peekResearchHandoff(): ResearchHandoffDraft | null {
  return researchHandoffDraft
}

export function researchRequest(request: ResearchMutationRequest): Promise<unknown> {
  return invoke<unknown>('research_request', { request })
}

export function researchList(): Promise<ResearchListResponse> {
  return researchRequest({ op: 'list' }) as Promise<ResearchListResponse>
}

export async function researchCreate(request: ResearchCreateRequest): Promise<ResearchJobSummary> {
  const result = (await researchRequest({ op: 'create', ...request })) as ResearchDetailResponse
  return result.job
}

export function researchGet(jobId: string): Promise<ResearchDetailResponse> {
  return researchRequest({ op: 'get', job_id: jobId }) as Promise<ResearchDetailResponse>
}

export function researchDecision(request: ResearchDecisionRequest): Promise<unknown> {
  return researchRequest({ op: 'decision', ...request })
}

export function researchRevise(request: ResearchReviseRequest): Promise<unknown> {
  return researchRequest({ op: 'revise', ...request })
}

export function researchPause(jobId: string): Promise<unknown> {
  return researchRequest({ op: 'pause', job_id: jobId })
}

export function researchResume(jobId: string): Promise<unknown> {
  return researchRequest({ op: 'resume', job_id: jobId })
}

export function researchCancel(jobId: string): Promise<unknown> {
  return researchRequest({ op: 'cancel', job_id: jobId })
}

/**
 * Borra la investigación y todo lo que colgaba de ella: informe, evidencia,
 * juicios y archivos. Es destructivo y sin vuelta; un job corriendo se cancela
 * primero.
 */
export function researchDelete(jobId: string): Promise<unknown> {
  return researchRequest({ op: 'delete', job_id: jobId })
}

export function researchSource(
  jobId: string,
  itemId: string
): Promise<{ sources: ResearchSourcePath[] }> {
  return researchRequest({ op: 'source', job_id: jobId, item_id: itemId }) as Promise<{
    sources: ResearchSourcePath[]
  }>
}

/**
 * Responde la ronda de clarificación y desbloquea el job.
 *
 * La ronda no es un gate de aprobar o rechazar: el motor rechaza
 * `researchDecision` sobre ella justamente porque aprobar preguntas sin
 * contestarlas dejaba el job corriendo sobre una etapa que no puede avanzar.
 * Una pregunta suelta puede ir vacía y queda declarada en el informe; la ronda
 * entera en blanco, no.
 */
export function researchAnswer(request: ResearchAnswerRequest): Promise<ResearchDetailResponse> {
  return researchRequest({ op: 'answer', ...request }) as Promise<ResearchDetailResponse>
}

/** Ronda de clarificación vigente, si la hay. */
export function currentClarificationRound(
  artifacts: ResearchArtifact[]
): { artifact: ResearchArtifact; round: ResearchClarificationRound } | null {
  const vigentes = artifacts.filter((a) => a.kind === 'clarification_round' && !a.obsolete)
  const artifact = vigentes[vigentes.length - 1]
  if (!artifact) return null
  const round = artifact.content as ResearchClarificationRound
  if (!Array.isArray(round?.questions)) return null
  return { artifact, round }
}

/** La ronda vigente está abierta: hay preguntas y todavía no hay respuestas. */
export function hasOpenClarification(artifacts: ResearchArtifact[]): boolean {
  const actual = currentClarificationRound(artifacts)
  return actual !== null && !Array.isArray(actual.round.answers)
}

export function researchAdvance(jobId: string): Promise<unknown> {
  return researchRequest({ op: 'advance', job_id: jobId })
}

/**
 * Tauri `invoke` rechaza con el string del backend, no con `Error`;
 * ese mensaje es el que hay que mostrar.
 */
export function describeBackendError(error: unknown, fallback: () => string): string {
  if (error instanceof Error && error.message.trim()) return error.message
  if (typeof error === 'string' && error.trim()) return error
  return fallback()
}
