/**
 * Settings frontend client for EntropIA Pro desktop app.
 * Wraps Tauri commands for the app_settings key-value store.
 */

import { invoke } from '@tauri-apps/api/core'

export interface SettingEntry {
  key: string
  value: string
}

export interface ModelInfo {
  id: string
  name: string
  context_length: number
}

// ---------------------------------------------------------------------------
// Settings CRUD
// ---------------------------------------------------------------------------

export function settingsGet(key: string): Promise<string | null> {
  return invoke<string | null>('settings_get', { key })
}

export function settingsSet(key: string, value: string): Promise<void> {
  return invoke<void>('settings_set', { key, value })
}

export function settingsGetAll(): Promise<SettingEntry[]> {
  return invoke<SettingEntry[]>('settings_get_all')
}

export function settingsDelete(key: string): Promise<void> {
  return invoke<void>('settings_delete', { key })
}

// ---------------------------------------------------------------------------
// OpenRouter-specific
// ---------------------------------------------------------------------------

export function testOpenrouterConnection(apiKey: string): Promise<ModelInfo[]> {
  return invoke<ModelInfo[]>('test_openrouter_connection', { apiKey })
}

export function testAssemblyaiConnection(apiKey: string): Promise<void> {
  return invoke<void>('test_assemblyai_connection', { apiKey })
}

export function testGlmOcrConnection(apiKey: string): Promise<void> {
  return invoke<void>('test_glm_ocr_connection', { apiKey })
}

// ---------------------------------------------------------------------------
// Well-known setting keys
// ---------------------------------------------------------------------------

export const SETTINGS_KEYS = {
  OPENROUTER_API_KEY: 'openrouter_api_key',
  OPENROUTER_MODEL: 'openrouter_model',
  OPENROUTER_EMBEDDING_MODEL: 'openrouter_embedding_model',
  LLM_MODE: 'llm_mode',
  EMBEDDING_PROVIDER: 'embedding_provider',
  LOCAL_EMBEDDING_MODEL_DIR: 'local_embedding_model_dir',
  ASSEMBLYAI_API_KEY: 'assemblyai_api_key',
  ASSEMBLYAI_SPEAKER_LABELS: 'assemblyai_role_speaker_identification',
  STT_MODE: 'stt_mode',
  GLM_OCR_API_KEY: 'glm_ocr_api_key',
  OCRH_MODE: 'ocrh_mode',
  LANGUAGE: 'language',
  // --- Pro local-model wiring (preserved) ---
  DEPS_VENV_PYTHON_PATH: 'deps_venv_python_path',
  PYTHON_RUNTIME_SELECTION: 'python.runtime_selection',
  LOCAL_MODEL_FILENAME: 'local_model_filename',
  LOCAL_MODEL_SOURCE_URL: 'local_model_source_url',
  // --- Shared structure from Lite ---
  RAG_ACTIVE_CONVERSATION: 'rag_active_conversation',
  OCR_CORRECTION_PROMPT: 'prompt_ocr_correction',
  SUMMARY_PROMPT: 'prompt_summary',
  NER_PROMPT: 'prompt_ner',
  TRIPLETS_PROMPT: 'prompt_triplets',
  LLM_TEMPERATURE: 'llm_temperature',
  LLM_MAX_TOKENS: 'llm_max_tokens',
  LLM_TOP_P: 'llm_top_p',
  LLM_TOP_K: 'llm_top_k',
  LLM_PRESENCE_PENALTY: 'llm_presence_penalty',
  LLM_FREQUENCY_PENALTY: 'llm_frequency_penalty',
  LLM_STOP_SEQUENCES: 'llm_stop_sequences',
  LLM_OCR_CORRECTION_MODEL: 'llm_ocr_correction_model',
  LLM_OCR_CORRECTION_TEMPERATURE: 'llm_ocr_correction_temperature',
  LLM_OCR_CORRECTION_MAX_TOKENS: 'llm_ocr_correction_max_tokens',
  LLM_OCR_CORRECTION_TOP_P: 'llm_ocr_correction_top_p',
  LLM_OCR_CORRECTION_TOP_K: 'llm_ocr_correction_top_k',
  LLM_OCR_CORRECTION_PRESENCE_PENALTY: 'llm_ocr_correction_presence_penalty',
  LLM_OCR_CORRECTION_FREQUENCY_PENALTY: 'llm_ocr_correction_frequency_penalty',
  LLM_OCR_CORRECTION_STOP_SEQUENCES: 'llm_ocr_correction_stop_sequences',
  LLM_SUMMARY_MODEL: 'llm_summary_model',
  LLM_SUMMARY_TEMPERATURE: 'llm_summary_temperature',
  LLM_SUMMARY_MAX_TOKENS: 'llm_summary_max_tokens',
  LLM_SUMMARY_TOP_P: 'llm_summary_top_p',
  LLM_SUMMARY_TOP_K: 'llm_summary_top_k',
  LLM_SUMMARY_PRESENCE_PENALTY: 'llm_summary_presence_penalty',
  LLM_SUMMARY_FREQUENCY_PENALTY: 'llm_summary_frequency_penalty',
  LLM_SUMMARY_STOP_SEQUENCES: 'llm_summary_stop_sequences',
  LLM_NER_MODEL: 'openrouter_ner_model',
  LLM_NER_TEMPERATURE: 'llm_ner_temperature',
  LLM_NER_MAX_TOKENS: 'llm_ner_max_tokens',
  LLM_NER_TOP_P: 'llm_ner_top_p',
  LLM_NER_TOP_K: 'llm_ner_top_k',
  LLM_NER_PRESENCE_PENALTY: 'llm_ner_presence_penalty',
  LLM_NER_FREQUENCY_PENALTY: 'llm_ner_frequency_penalty',
  LLM_NER_STOP_SEQUENCES: 'llm_ner_stop_sequences',
  LLM_TRIPLETS_MODEL: 'llm_triplets_model',
  LLM_TRIPLETS_TEMPERATURE: 'llm_triplets_temperature',
  LLM_TRIPLETS_MAX_TOKENS: 'llm_triplets_max_tokens',
  LLM_TRIPLETS_TOP_P: 'llm_triplets_top_p',
  LLM_TRIPLETS_TOP_K: 'llm_triplets_top_k',
  LLM_TRIPLETS_PRESENCE_PENALTY: 'llm_triplets_presence_penalty',
  LLM_TRIPLETS_FREQUENCY_PENALTY: 'llm_triplets_frequency_penalty',
  LLM_TRIPLETS_STOP_SEQUENCES: 'llm_triplets_stop_sequences',
  RAG_TOP_K: 'rag_top_k',
  RAG_MIN_SIMILARITY: 'rag_min_similarity',
  RAG_CANDIDATES_PER_LEG: 'rag_candidates_per_leg',
  RAG_RRF_K: 'rag_rrf_k',
  RAG_SNIPPET_MAX_CHARS: 'rag_snippet_max_chars',
  RAG_CONTEXT_MAX_CHARS: 'rag_context_max_chars',
  RAG_HISTORY_TURNS: 'rag_history_turns',
  RAG_HISTORY_TURN_MAX_CHARS: 'rag_history_turn_max_chars',
  RAG_TEMPERATURE: 'rag_temperature',
  RAG_MAX_TOKENS: 'rag_max_tokens',
} as const

export type LlmMode = 'local' | 'openrouter' | 'auto'
export type EmbeddingProvider = 'api' | 'local'
export type SttMode = 'local' | 'assemblyai' | 'auto'
export type OcrhMode = 'local' | 'glm_ocr' | 'auto'

export const DEFAULT_OPENROUTER_MODEL = 'google/gemma-4-26b-a4b-it'
export const DEFAULT_OPENROUTER_NER_MODEL = DEFAULT_OPENROUTER_MODEL
export const DEFAULT_OPENROUTER_EMBEDDING_MODEL = 'baai/bge-m3'

// Default operating modes flip with the build variant. The Pro (local-ML) build
// lands on the local engines; the API-only build lands on the remote providers
// (the exact values a fresh lite install ships). Inline compare so the
// define()'d VITE_LOCAL_ML literal tree-shakes the dead arm per build.
const _pro = import.meta.env.VITE_LOCAL_ML === '1'
export const DEFAULT_LLM_MODE: LlmMode = _pro ? 'local' : 'openrouter'
export const DEFAULT_EMBEDDING_PROVIDER: EmbeddingProvider = _pro ? 'local' : 'api'
export const DEFAULT_STT_MODE: SttMode = _pro ? 'local' : 'assemblyai'
export const DEFAULT_OCRH_MODE: OcrhMode = _pro ? 'local' : 'glm_ocr'

export const DEFAULT_PROMPTS = {
  ocrCorrectionPrompt: `Sos un especialista en transcripción de documentos históricos. El siguiente texto fue extraído por OCR de un documento impreso y contiene errores.

Tu tarea:
1. Corregí errores de OCR: sustituciones de caracteres, espacios faltantes, palabras garabateadas, letras mal leídas.
2. Unificá líneas rotas: mergeá líneas que fueron divididas por el layout en columnas o guiones en oraciones y párrafos completos. NO conserves saltos de línea que provienen del layout en columnas — reconstruí el flujo de lectura natural.
3. Ignorá los cortes de columnas de impresión: el texto viene de layouts multi-columna. Mergeá el texto de diferentes columnas en un orden de lectura coherente.
4. Preservá el idioma, estilo y terminología histórica originales. No modernices ni interpretes.
5. Si una palabra o fragmento es dudoso, conservá la versión más probable según el contexto, pero NO inventes contenido ausente.
6. No resumas ni reescribas: corregí el OCR, pero mantené el contenido, el orden de lectura y el nivel de detalle del original.
7. Si una palabra quedó cortada por guion de fin de línea, reconstruila; si el guion pertenece realmente al contenido, conserválo.

Devolvé SOLO el texto corregido y unificado con saltos de párrafo apropiados.
NO agregues explicaciones, títulos, comillas, markdown, bloques de código ni JSON.
NO repitas la consigna.

Texto OCR:
{text}`,
  summaryPrompt: `Resumí este texto de documento histórico en un ÚNICO párrafo conciso. El resumen debe:
- Tener entre 10 y 15 líneas
- Preservar nombres propios, fechas, lugares y eventos clave
- Estar escrito en el mismo idioma que el texto original (por defecto, español)
- SIEMPRE terminar con una oración completa que termine en punto

NO superes las 15 líneas. NO cortes a mitad de frase.

Texto:
{text}`,
  nerPrompt: `Extraé entidades nombradas del texto histórico. Devolvé SOLO JSON válido, sin markdown. Usá exclusivamente estas categorías: PER, LOC, ORG, DATE, MISC. Formato: [{"value":"...","type":"PER|LOC|ORG|DATE|MISC","start_offset":0,"end_offset":0,"confidence":0.95}]. Si no hay entidades, devolvé []. No inventes entidades ni uses categorías fuera del contrato.

Texto:
{text}`,
  tripletsPrompt: `Extraé triples semánticos (sujeto-predicado-objeto) de este texto de documento histórico.

Reglas obligatorias:
- Devolvé SOLO un array JSON válido.
- Cada elemento DEBE ser un objeto con EXACTAMENTE estas claves: "subject", "predicate", "object".
- Todos los valores DEBEN ser strings JSON válidos.
- No agregues claves extra.
- No agregues texto antes ni después del array.
- Si no encontrás relaciones confiables, devolvé [].
- Preferí sujetos y objetos completos (sintagmas nominales completos), no fragmentos sueltos, pronombres ni títulos aislados si el referente explícito aparece en el texto.
- Evitá duplicados o variantes mínimas de la misma relación.

Enfocate en relaciones fácticas: quién hizo qué, quién está relacionado con quién, qué pasó dónde y cuándo. Usá los términos exactos del texto. Respondé en el mismo idioma que el texto original (por defecto, español).

Ejemplo válido:
[
  {"subject":"Juan Pérez","predicate":"firmó","object":"el acta"}
]

Texto:
{text}`,
} as const

export const DEFAULT_MODEL_PARAMS = {
  model: DEFAULT_OPENROUTER_MODEL,
  temperature: '0.3',
  maxTokens: '4096',
  topP: '1',
  topK: '0',
  presencePenalty: '0',
  frequencyPenalty: '0',
  stopSequences: '',
} as const

export const DEFAULT_MODEL_PARAMS_BY_FLOW = {
  ocrCorrection: { ...DEFAULT_MODEL_PARAMS, maxTokens: '8192' },
  summary: { ...DEFAULT_MODEL_PARAMS, maxTokens: '1024' },
  ner: { ...DEFAULT_MODEL_PARAMS, model: DEFAULT_OPENROUTER_NER_MODEL },
  triplets: { ...DEFAULT_MODEL_PARAMS },
} as const

export const DEFAULT_RAG_PARAMS = {
  topK: '6',
  minSimilarity: '0',
  candidatesPerLeg: '24',
  rrfK: '60',
  snippetMaxChars: '1600',
  contextMaxChars: '10000',
  historyTurns: '6',
  historyTurnMaxChars: '500',
  temperature: '0.2',
  maxTokens: '4096',
} as const
