import { describe, it, expect, vi, beforeEach } from 'vitest'

// Mock @tauri-apps/api/core
vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(),
}))

import { invoke } from '@tauri-apps/api/core'
import { LOCAL_ML } from './capabilities'
import {
  settingsGet,
  settingsSet,
  settingsGetAll,
  settingsDelete,
  testOpenrouterConnection,
  testAssemblyaiConnection,
  testGlmOcrConnection,
  SETTINGS_KEYS,
  DEFAULT_OPENROUTER_MODEL,
  DEFAULT_OPENROUTER_NER_MODEL,
  DEFAULT_LLM_MODE,
  DEFAULT_EMBEDDING_PROVIDER,
  DEFAULT_STT_MODE,
  DEFAULT_OCRH_MODE,
  DEFAULT_PROMPTS,
  DEFAULT_MODEL_PARAMS,
  DEFAULT_MODEL_PARAMS_BY_FLOW,
  DEFAULT_RAG_PARAMS,
} from './settings'

const mockInvoke = vi.mocked(invoke)

beforeEach(() => {
  vi.clearAllMocks()
})

describe('settings', () => {
  describe('settingsGet', () => {
    it('calls invoke with correct command and key', async () => {
      mockInvoke.mockResolvedValueOnce('test-value')
      const result = await settingsGet('my_key')
      expect(mockInvoke).toHaveBeenCalledWith('settings_get', { key: 'my_key' })
      expect(result).toBe('test-value')
    })

    it('returns null when setting does not exist', async () => {
      mockInvoke.mockResolvedValueOnce(null)
      const result = await settingsGet('nonexistent')
      expect(result).toBeNull()
    })
  })

  describe('settingsSet', () => {
    it('calls invoke with correct command, key and value', async () => {
      mockInvoke.mockResolvedValueOnce(undefined)
      await settingsSet('my_key', 'my_value')
      expect(mockInvoke).toHaveBeenCalledWith('settings_set', {
        key: 'my_key',
        value: 'my_value',
      })
    })
  })

  describe('settingsGetAll', () => {
    it('returns array of settings', async () => {
      const mockSettings = [
        { key: 'a', value: '1' },
        { key: 'b', value: '2' },
      ]
      mockInvoke.mockResolvedValueOnce(mockSettings)
      const result = await settingsGetAll()
      expect(mockInvoke).toHaveBeenCalledWith('settings_get_all')
      expect(result).toEqual(mockSettings)
    })
  })

  describe('settingsDelete', () => {
    it('calls invoke with correct command and key', async () => {
      mockInvoke.mockResolvedValueOnce(undefined)
      await settingsDelete('my_key')
      expect(mockInvoke).toHaveBeenCalledWith('settings_delete', { key: 'my_key' })
    })
  })

  describe('testOpenrouterConnection', () => {
    it('calls invoke with api key', async () => {
      const mockModels = [
        { id: 'google/gemma-4-26b-a4b-it', name: 'Gemma 4 26B', context_length: 8192 },
      ]
      mockInvoke.mockResolvedValueOnce(mockModels)
      const result = await testOpenrouterConnection('sk-or-test')
      expect(mockInvoke).toHaveBeenCalledWith('test_openrouter_connection', {
        apiKey: 'sk-or-test',
      })
      expect(result).toEqual(mockModels)
    })
  })

  describe('testAssemblyaiConnection', () => {
    it('calls invoke with api key', async () => {
      mockInvoke.mockResolvedValueOnce(undefined)
      await testAssemblyaiConnection('aai-test')
      expect(mockInvoke).toHaveBeenCalledWith('test_assemblyai_connection', {
        apiKey: 'aai-test',
      })
    })
  })

  describe('testGlmOcrConnection', () => {
    it('calls invoke with api key', async () => {
      mockInvoke.mockResolvedValueOnce(undefined)
      await testGlmOcrConnection('glm-test')
      expect(mockInvoke).toHaveBeenCalledWith('test_glm_ocr_connection', {
        apiKey: 'glm-test',
      })
    })
  })

  describe('constants', () => {
    it('exports well-known setting keys', () => {
      expect(SETTINGS_KEYS.OPENROUTER_API_KEY).toBe('openrouter_api_key')
      expect(SETTINGS_KEYS.OPENROUTER_MODEL).toBe('openrouter_model')
      expect(SETTINGS_KEYS.LLM_MODE).toBe('llm_mode')
      expect(SETTINGS_KEYS.ASSEMBLYAI_API_KEY).toBe('assemblyai_api_key')
      expect(SETTINGS_KEYS.ASSEMBLYAI_SPEAKER_LABELS).toBe('assemblyai_role_speaker_identification')
      expect(SETTINGS_KEYS.STT_MODE).toBe('stt_mode')
      expect(SETTINGS_KEYS.GLM_OCR_API_KEY).toBe('glm_ocr_api_key')
      expect(SETTINGS_KEYS.OCRH_MODE).toBe('ocrh_mode')
      expect(SETTINGS_KEYS.LANGUAGE).toBe('language')
    })

    it('exports local-model setting keys (Pro)', () => {
      expect(SETTINGS_KEYS.DEPS_VENV_PYTHON_PATH).toBe('deps_venv_python_path')
      expect(SETTINGS_KEYS.PYTHON_RUNTIME_SELECTION).toBe('python.runtime_selection')
      expect(SETTINGS_KEYS.LOCAL_MODEL_FILENAME).toBe('local_model_filename')
      expect(SETTINGS_KEYS.LOCAL_MODEL_SOURCE_URL).toBe('local_model_source_url')
      expect(SETTINGS_KEYS.LOCAL_EMBEDDING_MODEL_DIR).toBe('local_embedding_model_dir')
    })

    it('exports prompt setting keys', () => {
      expect(SETTINGS_KEYS.OCR_CORRECTION_PROMPT).toBe('prompt_ocr_correction')
      expect(SETTINGS_KEYS.SUMMARY_PROMPT).toBe('prompt_summary')
      expect(SETTINGS_KEYS.NER_PROMPT).toBe('prompt_ner')
      expect(SETTINGS_KEYS.TRIPLETS_PROMPT).toBe('prompt_triplets')
    })

    it('exports RAG setting keys', () => {
      expect(SETTINGS_KEYS.RAG_ACTIVE_CONVERSATION).toBe('rag_active_conversation')
      expect(SETTINGS_KEYS.RAG_TOP_K).toBe('rag_top_k')
      expect(SETTINGS_KEYS.RAG_MIN_SIMILARITY).toBe('rag_min_similarity')
      expect(SETTINGS_KEYS.RAG_CANDIDATES_PER_LEG).toBe('rag_candidates_per_leg')
      expect(SETTINGS_KEYS.RAG_FUSION_CANDIDATE_LIMIT).toBe('rag_fusion_candidate_limit')
      expect(SETTINGS_KEYS.RAG_RERANK_DEPTH).toBe('rag_rerank_depth')
      expect(SETTINGS_KEYS.RAG_RRF_K).toBe('rag_rrf_k')
      expect(SETTINGS_KEYS.RAG_SNIPPET_MAX_CHARS).toBe('rag_snippet_max_chars')
      expect(SETTINGS_KEYS.RAG_CONTEXT_MAX_CHARS).toBe('rag_context_max_chars')
      expect(SETTINGS_KEYS.RAG_HISTORY_TURNS).toBe('rag_history_turns')
      expect(SETTINGS_KEYS.RAG_HISTORY_TURN_MAX_CHARS).toBe('rag_history_turn_max_chars')
      expect(SETTINGS_KEYS.RAG_TEMPERATURE).toBe('rag_temperature')
      expect(SETTINGS_KEYS.RAG_MAX_TOKENS).toBe('rag_max_tokens')
    })

    it('exports per-flow LLM param keys', () => {
      expect(SETTINGS_KEYS.LLM_TEMPERATURE).toBe('llm_temperature')
      expect(SETTINGS_KEYS.LLM_OCR_CORRECTION_TEMPERATURE).toBe('llm_ocr_correction_temperature')
      expect(SETTINGS_KEYS.LLM_SUMMARY_MAX_TOKENS).toBe('llm_summary_max_tokens')
      expect(SETTINGS_KEYS.LLM_NER_TOP_P).toBe('llm_ner_top_p')
      expect(SETTINGS_KEYS.LLM_TRIPLETS_STOP_SEQUENCES).toBe('llm_triplets_stop_sequences')
      expect(SETTINGS_KEYS.LLM_OCR_CORRECTION_MODEL).toBe('llm_ocr_correction_model')
      expect(SETTINGS_KEYS.LLM_SUMMARY_MODEL).toBe('llm_summary_model')
      expect(SETTINGS_KEYS.LLM_NER_MODEL).toBe('openrouter_ner_model')
      expect(SETTINGS_KEYS.LLM_TRIPLETS_MODEL).toBe('llm_triplets_model')
    })

    it('has correct defaults', () => {
      expect(DEFAULT_OPENROUTER_MODEL).toBe('google/gemma-4-26b-a4b-it')
      expect(DEFAULT_LLM_MODE).toBe(LOCAL_ML ? 'local' : 'openrouter')
      expect(DEFAULT_EMBEDDING_PROVIDER).toBe(LOCAL_ML ? 'local' : 'api')
      expect(DEFAULT_STT_MODE).toBe(LOCAL_ML ? 'local' : 'assemblyai')
      expect(DEFAULT_OCRH_MODE).toBe(LOCAL_ML ? 'local' : 'glm_ocr')
    })

    it('exports default prompts for every flow', () => {
      expect(DEFAULT_PROMPTS.ocrCorrectionPrompt).toContain('{text}')
      expect(DEFAULT_PROMPTS.ocrCorrectionPrompt).toContain('REGLAS DE FORMATO OCR (OBLIGATORIAS):')
      expect(DEFAULT_PROMPTS.ocrCorrectionPrompt).toContain(
        'prioridad sobre cualquier instrucción contradictoria'
      )
      expect(DEFAULT_PROMPTS.ocrCorrectionPrompt).toContain('etiquetas HTML')
      expect(DEFAULT_PROMPTS.ocrCorrectionPrompt).toContain(
        'Mantené el contenido y el nivel de detalle, pero restablecé el orden natural de lectura'
      )
      expect(DEFAULT_PROMPTS.ocrCorrectionPrompt).not.toContain(
        'Mantené el contenido, el orden y el nivel de detalle'
      )
      expect(DEFAULT_PROMPTS.ocrCorrectionPrompt).toContain('Markdown')
      expect(DEFAULT_PROMPTS.ocrCorrectionPrompt).toContain(
        'Recorré la página completa en su orden natural de lectura'
      )
      expect(DEFAULT_PROMPTS.ocrCorrectionPrompt).toContain(
        'Reconstruí todas las palabras, oraciones y párrafos cortados por columnas'
      )
      expect(DEFAULT_PROMPTS.ocrCorrectionPrompt).toContain(
        'la imagen adjunta, cuando esté disponible, y la continuidad gramatical'
      )
      expect(DEFAULT_PROMPTS.ocrCorrectionPrompt).toContain(
        'el primer y el último texto legible, todos los encabezados y cada sección'
      )
      expect(DEFAULT_PROMPTS.ocrCorrectionPrompt).toContain(
        'Restituí negritas o cursivas en Markdown únicamente cuando una imagen adjunta muestre ese énfasis con claridad'
      )
      expect(DEFAULT_PROMPTS.ocrCorrectionPrompt).toContain(
        'La única excepción adicional es restituir delimitadores de énfasis Markdown'
      )
      expect(DEFAULT_PROMPTS.ocrCorrectionPrompt).toContain(
        'También tienen prioridad sobre cualquier instrucción contradictoria previa'
      )
      expect(DEFAULT_PROMPTS.ocrCorrectionPrompt).toContain(
        'La separación en párrafos del OCR es provisional'
      )
      expect(DEFAULT_PROMPTS.ocrCorrectionPrompt).toContain('aunque no sean líneas cortas')
      expect(DEFAULT_PROMPTS.summaryPrompt).toContain('{text}')
      expect(DEFAULT_PROMPTS.nerPrompt).toContain('{text}')
      expect(DEFAULT_PROMPTS.tripletsPrompt).toContain('{text}')
    })

    it('exports default model params and per-flow overrides', () => {
      expect(DEFAULT_MODEL_PARAMS.model).toBe(DEFAULT_OPENROUTER_MODEL)
      expect(DEFAULT_MODEL_PARAMS.temperature).toBe('0.3')
      expect(DEFAULT_MODEL_PARAMS.maxTokens).toBe('4096')
      expect(DEFAULT_MODEL_PARAMS.topP).toBe('1')
      expect(DEFAULT_MODEL_PARAMS.topK).toBe('0')
      expect(DEFAULT_MODEL_PARAMS.presencePenalty).toBe('0')
      expect(DEFAULT_MODEL_PARAMS_BY_FLOW.ocrCorrection).toMatchObject({
        model: DEFAULT_OPENROUTER_MODEL,
        temperature: '0.3',
        maxTokens: '8192',
      })
      expect(DEFAULT_MODEL_PARAMS_BY_FLOW.summary).toMatchObject({
        model: DEFAULT_OPENROUTER_MODEL,
        temperature: '0.3',
        maxTokens: '1024',
      })
      expect(DEFAULT_MODEL_PARAMS_BY_FLOW.ner).toMatchObject({
        model: DEFAULT_OPENROUTER_NER_MODEL,
        temperature: '0.3',
        maxTokens: '4096',
      })
      expect(DEFAULT_MODEL_PARAMS_BY_FLOW.triplets).toMatchObject({
        model: DEFAULT_OPENROUTER_MODEL,
        temperature: '0.3',
        maxTokens: '4096',
      })
    })

    it('exports default RAG params', () => {
      expect(DEFAULT_RAG_PARAMS.topK).toBe('6')
      expect(DEFAULT_RAG_PARAMS.rrfK).toBe('60')
      // Espejo de los defaults de Rust (`rag/params.rs`): la profundidad de
      // reordenamiento tiene que quedar entre topK y el tope de fusión, y por
      // debajo del tope — no reordenar la lista fusionada entera es el punto.
      expect(DEFAULT_RAG_PARAMS.fusionCandidateLimit).toBe('40')
      expect(DEFAULT_RAG_PARAMS.rerankDepth).toBe('16')
      expect(Number(DEFAULT_RAG_PARAMS.rerankDepth)).toBeGreaterThanOrEqual(
        Number(DEFAULT_RAG_PARAMS.topK)
      )
      expect(Number(DEFAULT_RAG_PARAMS.rerankDepth)).toBeLessThan(
        Number(DEFAULT_RAG_PARAMS.fusionCandidateLimit)
      )
      expect(DEFAULT_RAG_PARAMS.temperature).toBe('0.2')
      expect(DEFAULT_RAG_PARAMS.maxTokens).toBe('4096')
    })
  })
})
