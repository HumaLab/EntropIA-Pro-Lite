import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import SettingsView, {
  buildSettingsSnapshot,
  hasUnsavedSettingsChanges,
  type SettingsSnapshotInput,
} from './SettingsView.svelte'
import { locale } from '$lib/i18n'
import { navigation } from '$lib/navigation'
import { setupKeyboardShortcuts } from '$lib/keyboard'
import { DEFAULT_PROMPTS } from '$lib/settings'

const {
  invokeMock,
  settingsGetMock,
  settingsGetAllMock,
  settingsSetMock,
  testOpenrouterConnectionMock,
  testAssemblyaiConnectionMock,
  testGlmOcrConnectionMock,
  llmIsAvailableMock,
  llmLocalModelInfoMock,
  llmDownloadModelMock,
  embeddingLocalModelInfoMock,
  embeddingOpenModelsDirMock,
  embeddingDownloadModelMock,
  rerankerLocalModelInfoMock,
  rerankerOpenModelsDirMock,
  rerankerDownloadModelMock,
} = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  settingsGetMock: vi.fn(),
  settingsGetAllMock: vi.fn(),
  settingsSetMock: vi.fn(),
  testOpenrouterConnectionMock: vi.fn(),
  testAssemblyaiConnectionMock: vi.fn(),
  testGlmOcrConnectionMock: vi.fn(),
  llmIsAvailableMock: vi.fn(),
  llmLocalModelInfoMock: vi.fn(),
  llmDownloadModelMock: vi.fn(),
  embeddingLocalModelInfoMock: vi.fn(),
  embeddingOpenModelsDirMock: vi.fn(),
  embeddingDownloadModelMock: vi.fn(),
  rerankerLocalModelInfoMock: vi.fn(),
  rerankerOpenModelsDirMock: vi.fn(),
  rerankerDownloadModelMock: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

vi.mock('$lib/settings', async () => {
  const actual = await vi.importActual<typeof import('$lib/settings')>('$lib/settings')
  return {
    ...actual,
    settingsGet: settingsGetMock,
    settingsGetAll: settingsGetAllMock,
    settingsSet: settingsSetMock,
    testOpenrouterConnection: testOpenrouterConnectionMock,
    testAssemblyaiConnection: testAssemblyaiConnectionMock,
    testGlmOcrConnection: testGlmOcrConnectionMock,
  }
})

vi.mock('$lib/llm', () => ({
  llmIsAvailable: llmIsAvailableMock,
  llmLocalModelInfo: llmLocalModelInfoMock,
  llmOpenModelsDir: vi.fn(),
  llmDownloadModel: llmDownloadModelMock,
}))

vi.mock('$lib/embeddings', () => ({
  embeddingLocalModelInfo: embeddingLocalModelInfoMock,
  embeddingOpenModelsDir: embeddingOpenModelsDirMock,
  embeddingDownloadModel: embeddingDownloadModelMock,
}))

vi.mock('$lib/reranker', () => ({
  rerankerLocalModelInfo: rerankerLocalModelInfoMock,
  rerankerOpenModelsDir: rerankerOpenModelsDirMock,
  rerankerDownloadModel: rerankerDownloadModelMock,
}))

function applyDefaultSettingsBackend() {
  settingsGetMock.mockImplementation(async (key: string) => {
    if (key === 'openrouter_api_key') return 'sk-or-v1-test-key'
    if (key === 'openrouter_model') return 'anthropic/claude-3.7-sonnet'
    if (key === 'embedding_provider') return 'api'
    if (key === 'openrouter_embedding_model') return 'baai/bge-m3'
    if (key === 'llm_mode') return 'openrouter'
    if (key === 'stt_mode') return 'assemblyai'
    if (key === 'ocrh_mode') return 'glm_ocr'
    if (key === 'assemblyai_api_key') return 'aai-orig-test-1234'
    if (key === 'assemblyai_role_speaker_identification') return null
    if (key === 'language') return 'es'
    return null
  })
}

describe('SettingsView', () => {
  beforeEach(() => {
    locale.set('es')
    invokeMock.mockReset().mockResolvedValue(undefined)
    settingsGetMock.mockReset()
    settingsGetAllMock.mockReset().mockResolvedValue([])
    settingsSetMock.mockReset().mockResolvedValue(undefined)
    testOpenrouterConnectionMock.mockReset()
    testAssemblyaiConnectionMock.mockReset().mockResolvedValue(undefined)
    testGlmOcrConnectionMock.mockReset().mockResolvedValue(undefined)
    llmIsAvailableMock.mockReset().mockResolvedValue(true)
    llmDownloadModelMock.mockReset().mockResolvedValue(undefined)
    embeddingOpenModelsDirMock.mockReset().mockResolvedValue(undefined)
    embeddingDownloadModelMock.mockReset().mockResolvedValue('started')
    rerankerOpenModelsDirMock.mockReset().mockResolvedValue(undefined)
    rerankerDownloadModelMock.mockReset().mockResolvedValue('started')
    rerankerLocalModelInfoMock.mockReset().mockResolvedValue({
      available: false,
      canAutoDownload: true,
      directory:
        'C:/Users/test/AppData/Roaming/com.entropia.pro.desktop/models/rerankers/bge-reranker-v2-m3',
      path: 'C:/Users/test/AppData/Roaming/com.entropia.pro.desktop/models/rerankers/bge-reranker-v2-m3/model_int8.onnx',
      requiredFiles: [
        {
          filename: 'model_int8.onnx',
          sourcePath: 'onnx/model_int8.onnx',
          destination: 'model_int8.onnx',
          expectedSizeBytes: 570727094,
          actualSizeBytes: null,
          valid: false,
        },
      ],
      sourceRepo: 'onnx-community/bge-reranker-v2-m3-ONNX',
    })
    embeddingLocalModelInfoMock.mockReset().mockResolvedValue({
      exists: false,
      available: false,
      can_auto_download: true,
      directory: 'C:/Users/test/AppData/Roaming/com.entropia.pro.desktop/models/embeddings/bge-m3',
      path: 'C:/Users/test/AppData/Roaming/com.entropia.pro.desktop/models/embeddings/bge-m3/model.onnx',
      size_bytes: null,
      required_files: [
        {
          filename: 'model.onnx',
          source_path: 'onnx/model.onnx',
          destination: 'model.onnx',
          size_bytes: 724923,
          exists: false,
        },
        {
          filename: 'model.onnx_data',
          source_path: 'onnx/model.onnx_data',
          destination: 'model.onnx_data',
          size_bytes: 2266820608,
          exists: false,
        },
        {
          filename: 'tokenizer.json',
          source_path: 'onnx/tokenizer.json',
          destination: 'tokenizer.json',
          size_bytes: 17082821,
          exists: false,
        },
      ],
      missing_files: [
        {
          filename: 'model.onnx',
          source_path: 'onnx/model.onnx',
          destination: 'model.onnx',
          size_bytes: 724923,
          exists: false,
        },
        {
          filename: 'model.onnx_data',
          source_path: 'onnx/model.onnx_data',
          destination: 'model.onnx_data',
          size_bytes: 2266820608,
          exists: false,
        },
        {
          filename: 'tokenizer.json',
          source_path: 'onnx/tokenizer.json',
          destination: 'tokenizer.json',
          size_bytes: 17082821,
          exists: false,
        },
      ],
      source_repo: 'BAAI/bge-m3',
    })
    llmLocalModelInfoMock.mockReset().mockResolvedValue({
      exists: true,
      available: true,
      can_auto_download: false,
      disabled_reason: null,
      path: '/home/test/.local/share/com.entropia.pro.desktop/models/gemma-4-E2B-it-Q4_K_M.gguf',
      size_bytes: 2_500_000_000,
      filename: 'gemma-4-E2B-it-Q4_K_M.gguf',
      source_url:
        'https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF/resolve/main/gemma-4-E2B-it-Q4_K_M.gguf?download=true',
    })

    applyDefaultSettingsBackend()
  })

  it('renders the provider-focused header hierarchy with the active LLM provider summary', async () => {
    render(SettingsView)

    expect(await screen.findByText('Preferencias')).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'Configuración' })).toBeInTheDocument()
    expect(
      screen.getByText(
        'Ajustá proveedores, credenciales y parámetros de inteligencia artificial para EntropIA Pro.'
      )
    ).toBeInTheDocument()

    await waitFor(() => {
      expect(screen.getByText('Proveedor LLM: OpenRouter')).toBeInTheDocument()
    })
  })

  it('renders the full tab structure including the Pro-only Dependencias tab', async () => {
    render(SettingsView)

    expect(await screen.findByRole('tab', { name: 'APIs remotas' })).toBeInTheDocument()
    expect(screen.getByRole('tab', { name: 'Prompts' })).toBeInTheDocument()
    expect(screen.getByRole('tab', { name: 'Model Params' })).toBeInTheDocument()
    expect(screen.getByRole('tab', { name: 'RAG Params' })).toBeInTheDocument()
    expect(screen.getByRole('tab', { name: /Dependencias de IA/ })).toBeInTheDocument()
    expect(screen.getByRole('tab', { name: 'Logs' })).toBeInTheDocument()
  })

  it('exposes the LLM mode, embedding provider, STT and OCR-H local selectors', async () => {
    render(SettingsView)

    // LLM mode (3) + embedding provider (2) + STT (3) + OCR-H (3) = 11 radios.
    await waitFor(() => {
      expect(screen.getAllByRole('radio')).toHaveLength(11)
    })
    // The embedding provider exposes the unambiguous local ONNX option.
    expect(screen.getByRole('radio', { name: /Local ONNX/i })).toBeInTheDocument()
    // Each modal flow (LLM, STT, OCR-H) plus embeddings exposes a "Local" option.
    expect(screen.getAllByRole('radio', { name: /^Local/ }).length).toBeGreaterThanOrEqual(4)
  })

  it('shows and starts installation of the Pro local RAG reranker', async () => {
    render(SettingsView)

    expect(await screen.findByRole('heading', { name: 'Reranker RAG local' })).toBeInTheDocument()
    expect(await screen.findByText('Modelo sin instalar')).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Instalar reranker local' }))

    expect(rerankerDownloadModelMock).toHaveBeenCalledOnce()
  })

  it('edits and saves prompt and model parameter settings', async () => {
    render(SettingsView)

    await fireEvent.click(await screen.findByRole('tab', { name: 'Prompts' }))
    const ocrPrompt = screen.getByLabelText('OCR correction prompt')
    await fireEvent.input(ocrPrompt, { target: { value: 'Custom OCR {text}' } })

    await fireEvent.click(screen.getByRole('tab', { name: 'Model Params' }))
    const modelInput = screen.getAllByLabelText('model')[0]
    const temperatureInput = screen.getAllByLabelText('temperature (0-2)')[0]
    const maxTokensInput = screen.getAllByLabelText('maxTokens (1-16000)')[0]
    expect(modelInput).toBeDefined()
    expect(temperatureInput).toBeDefined()
    expect(maxTokensInput).toBeDefined()
    expect(
      screen.getAllByText('Temperatura: gradúa la creatividad de la respuesta generada (0-2)')
    ).not.toHaveLength(0)
    expect(
      screen.getAllByText('Tokens máximos: limita la longitud de la respuesta generada (1-16000)')
    ).not.toHaveLength(0)
    await fireEvent.input(modelInput!, { target: { value: 'openai/gpt-test' } })
    await fireEvent.input(temperatureInput!, { target: { value: '0.6' } })
    await fireEvent.input(maxTokensInput!, { target: { value: '1234' } })

    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    expect(settingsSetMock).toHaveBeenCalledWith('prompt_ocr_correction', 'Custom OCR {text}')
    expect(settingsSetMock).toHaveBeenCalledWith('llm_ocr_correction_model', 'openai/gpt-test')
    expect(settingsSetMock).toHaveBeenCalledWith('llm_summary_model', 'anthropic/claude-3.7-sonnet')
    expect(settingsSetMock).toHaveBeenCalledWith(
      'openrouter_ner_model',
      'anthropic/claude-3.7-sonnet'
    )
    expect(settingsSetMock).toHaveBeenCalledWith(
      'llm_triplets_model',
      'anthropic/claude-3.7-sonnet'
    )
    expect(settingsSetMock).toHaveBeenCalledWith('llm_ocr_correction_temperature', '0.6')
    expect(settingsSetMock).toHaveBeenCalledWith('llm_ocr_correction_max_tokens', '1234')
  })

  it('shows the effective model and numeric defaults for all four flows', async () => {
    settingsGetMock.mockImplementation(async (key: string) => {
      if (key === 'openrouter_api_key') return 'sk-or-v1-test-key'
      if (key === 'language') return 'es'
      return null
    })
    render(SettingsView)

    await fireEvent.click(await screen.findByRole('tab', { name: 'Model Params' }))

    expect(
      screen.getAllByLabelText('model').map((input) => (input as HTMLInputElement).value)
    ).toEqual([
      'google/gemma-4-26b-a4b-it',
      'google/gemma-4-26b-a4b-it',
      'google/gemma-4-26b-a4b-it',
      'google/gemma-4-26b-a4b-it',
    ])
    expect(
      screen
        .getAllByLabelText('maxTokens (1-16000)')
        .map((input) => (input as HTMLInputElement).value)
    ).toEqual(['8192', '1024', '4096', '4096'])
    expect(
      screen.getAllByLabelText('topP (0-1)').map((input) => (input as HTMLInputElement).value)
    ).toEqual(['1', '1', '1', '1'])
    expect(
      screen.getAllByLabelText('topK (0-1000)').map((input) => (input as HTMLInputElement).value)
    ).toEqual(['0', '0', '0', '0'])
  })

  it('loads persisted per-flow models and token limits independently', async () => {
    settingsGetAllMock.mockResolvedValue([
      { key: 'llm_ocr_correction_model', value: 'vendor/ocr' },
      { key: 'llm_ocr_correction_max_tokens', value: '3000' },
      { key: 'llm_summary_model', value: 'vendor/summary' },
      { key: 'llm_summary_max_tokens', value: '700' },
      { key: 'openrouter_ner_model', value: 'vendor/ner' },
      { key: 'llm_ner_max_tokens', value: '4096' },
      { key: 'llm_triplets_model', value: 'vendor/triplets' },
      { key: 'llm_triplets_max_tokens', value: '1800' },
    ])
    render(SettingsView)

    await fireEvent.click(await screen.findByRole('tab', { name: 'Model Params' }))

    expect(
      screen.getAllByLabelText('model').map((input) => (input as HTMLInputElement).value)
    ).toEqual(['vendor/ocr', 'vendor/summary', 'vendor/ner', 'vendor/triplets'])
    expect(
      screen
        .getAllByLabelText('maxTokens (1-16000)')
        .map((input) => (input as HTMLInputElement).value)
    ).toEqual(['3000', '700', '4096', '1800'])
  })

  it('rejects model param formats that the Rust parser cannot parse', async () => {
    render(SettingsView)

    await screen.findByText(/sk-o\*\*\*\*\.\.\.\*\*\*\*-key/)
    await fireEvent.click(screen.getByRole('tab', { name: 'Model Params' }))

    const maxTokensInput = screen.getAllByLabelText('maxTokens (1-16000)')[0]
    const temperatureInput = screen.getAllByLabelText('temperature (0-2)')[0]

    // Number('12.0') es 12 para JS, pero "12.0".parse::<i32>() falla en Rust.
    await fireEvent.input(maxTokensInput!, { target: { value: '12.0' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    expect(
      await screen.findAllByText('Parámetro inválido en OCR correction: maxTokens')
    ).not.toHaveLength(0)
    expect(settingsSetMock).not.toHaveBeenCalled()

    await fireEvent.input(maxTokensInput!, { target: { value: '16001' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    expect(
      await screen.findAllByText('Parámetro inválido en OCR correction: maxTokens')
    ).not.toHaveLength(0)
    expect(settingsSetMock).not.toHaveBeenCalled()

    await fireEvent.input(maxTokensInput!, { target: { value: '1e3' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    expect(settingsSetMock).not.toHaveBeenCalled()

    // '0x1' vale 1 para Number() (en rango 0-2), pero parse::<f32> lo rechaza.
    await fireEvent.input(maxTokensInput!, { target: { value: '8192' } })
    await fireEvent.input(temperatureInput!, { target: { value: '0x1' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    expect(
      await screen.findAllByText('Parámetro inválido en OCR correction: temperature')
    ).not.toHaveLength(0)
    expect(settingsSetMock).not.toHaveBeenCalled()
  })

  it('normalizes model param text to plain numbers when saving', async () => {
    render(SettingsView)

    await screen.findByText(/sk-o\*\*\*\*\.\.\.\*\*\*\*-key/)
    await fireEvent.click(screen.getByRole('tab', { name: 'Model Params' }))

    const temperatureInput = screen.getAllByLabelText('temperature (0-2)')[0]
    const maxTokensInput = screen.getAllByLabelText('maxTokens (1-16000)')[0]
    await fireEvent.input(temperatureInput!, { target: { value: '.5' } })
    await fireEvent.input(maxTokensInput!, { target: { value: '007' } })

    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    await waitFor(() =>
      expect(settingsSetMock).toHaveBeenCalledWith('llm_ocr_correction_temperature', '0.5')
    )
    expect(settingsSetMock).toHaveBeenCalledWith('llm_ocr_correction_max_tokens', '7')
  })

  it('switches to the Model Params tab when validation fails from another tab', async () => {
    invokeMock.mockImplementation(async (cmd: string) => (cmd === 'logs_get' ? [] : undefined))
    render(SettingsView)

    await screen.findByText(/sk-o\*\*\*\*\.\.\.\*\*\*\*-key/)
    await fireEvent.click(screen.getByRole('tab', { name: 'Model Params' }))

    const maxTokensInput = screen.getAllByLabelText('maxTokens (1-16000)')[0]
    await fireEvent.input(maxTokensInput!, { target: { value: '12.0' } })

    // El error debe ser visible aunque el guardado se dispare desde otra tab.
    await fireEvent.click(screen.getByRole('tab', { name: 'Logs' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    expect(
      await screen.findAllByText('Parámetro inválido en OCR correction: maxTokens')
    ).not.toHaveLength(0)
    expect(screen.getByRole('tab', { name: 'Model Params' })).toHaveAttribute(
      'aria-selected',
      'true'
    )
    expect(settingsSetMock).not.toHaveBeenCalled()
  })

  it('switches to the RAG params tab and shows defaults when no settings are stored', async () => {
    render(SettingsView)

    await screen.findByText(/sk-o\*\*\*\*\.\.\.\*\*\*\*-key/)
    await fireEvent.click(screen.getByRole('tab', { name: 'RAG Params' }))

    expect(screen.getByRole('heading', { name: 'RAG Params' })).toBeInTheDocument()
    expect(screen.getByLabelText('topK (1-20)')).toHaveValue('6')
    expect(screen.getByLabelText('minSimilarity (0-1, 0 = off)')).toHaveValue('0')
    expect(screen.getByLabelText('candidatesPerLeg (4-200)')).toHaveValue('24')
    expect(screen.getByLabelText('rrfK (1-500)')).toHaveValue('60')
    expect(screen.getByLabelText('snippetMaxChars (200-8000)')).toHaveValue('1600')
    expect(screen.getByLabelText('contextMaxChars (1000-350000)')).toHaveValue('10000')
    expect(screen.getByLabelText('historyTurns (0-20)')).toHaveValue('6')
    expect(screen.getByLabelText('historyTurnMaxChars (100-4000)')).toHaveValue('500')
    expect(screen.getByLabelText('temperature (0-2)')).toHaveValue('0.2')
    expect(screen.getByLabelText('maxTokens (64-16000)')).toHaveValue('4096')
    expect(
      screen.getByText('Temperatura: gradúa la creatividad del modelo (0-2)')
    ).toBeInTheDocument()
    expect(
      screen.getByText('Respuesta: limita tokens generados por el modelo (64-16000)')
    ).toBeInTheDocument()
  })

  it('shows stored RAG params overrides instead of defaults', async () => {
    settingsGetAllMock.mockResolvedValue([
      { key: 'rag_top_k', value: '12' },
      { key: 'rag_temperature', value: '0.7' },
    ])

    render(SettingsView)

    await screen.findByText(/sk-o\*\*\*\*\.\.\.\*\*\*\*-key/)
    await fireEvent.click(screen.getByRole('tab', { name: 'RAG Params' }))

    await waitFor(() => expect(screen.getByLabelText('topK (1-20)')).toHaveValue('12'))
    expect(screen.getByLabelText('temperature (0-2)')).toHaveValue('0.7')
    expect(screen.getByLabelText('rrfK (1-500)')).toHaveValue('60')
  })

  it('edits and saves RAG params, persisting defaults for untouched fields', async () => {
    render(SettingsView)

    await screen.findByText(/sk-o\*\*\*\*\.\.\.\*\*\*\*-key/)
    await fireEvent.click(screen.getByRole('tab', { name: 'RAG Params' }))

    await fireEvent.input(screen.getByLabelText('topK (1-20)'), { target: { value: '9' } })
    await fireEvent.input(screen.getByLabelText('contextMaxChars (1000-350000)'), {
      target: { value: '20000' },
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    await waitFor(() => expect(settingsSetMock).toHaveBeenCalledWith('rag_top_k', '9'))
    expect(settingsSetMock).toHaveBeenCalledWith('rag_context_max_chars', '20000')
    expect(settingsSetMock).toHaveBeenCalledWith('rag_min_similarity', '0')
    expect(settingsSetMock).toHaveBeenCalledWith('rag_candidates_per_leg', '24')
    expect(settingsSetMock).toHaveBeenCalledWith('rag_rrf_k', '60')
    expect(settingsSetMock).toHaveBeenCalledWith('rag_snippet_max_chars', '1600')
    expect(settingsSetMock).toHaveBeenCalledWith('rag_history_turns', '6')
    expect(settingsSetMock).toHaveBeenCalledWith('rag_history_turn_max_chars', '500')
    expect(settingsSetMock).toHaveBeenCalledWith('rag_temperature', '0.2')
    expect(settingsSetMock).toHaveBeenCalledWith('rag_max_tokens', '4096')
  })

  it('blocks saving out-of-range RAG params and shows the validation error', async () => {
    render(SettingsView)

    await screen.findByText(/sk-o\*\*\*\*\.\.\.\*\*\*\*-key/)
    await fireEvent.click(screen.getByRole('tab', { name: 'RAG Params' }))

    await fireEvent.input(screen.getByLabelText('maxTokens (64-16000)'), {
      target: { value: '16001' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    expect(await screen.findAllByText('Parámetro RAG inválido: maxTokens')).not.toHaveLength(0)
    expect(settingsSetMock).not.toHaveBeenCalled()
  })

  it('blocks saving when snippetMaxChars exceeds contextMaxChars', async () => {
    render(SettingsView)

    await screen.findByText(/sk-o\*\*\*\*\.\.\.\*\*\*\*-key/)
    await fireEvent.click(screen.getByRole('tab', { name: 'RAG Params' }))

    await fireEvent.input(screen.getByLabelText('snippetMaxChars (200-8000)'), {
      target: { value: '5000' },
    })
    await fireEvent.input(screen.getByLabelText('contextMaxChars (1000-350000)'), {
      target: { value: '2000' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    expect(
      await screen.findAllByText('snippetMaxChars no puede superar contextMaxChars.')
    ).not.toHaveLength(0)
    expect(settingsSetMock).not.toHaveBeenCalled()
  })

  it('normalizes RAG numeric text to its canonical form on save', async () => {
    render(SettingsView)

    await screen.findByText(/sk-o\*\*\*\*\.\.\.\*\*\*\*-key/)
    await fireEvent.click(screen.getByRole('tab', { name: 'RAG Params' }))

    await fireEvent.input(screen.getByLabelText('temperature (0-2)'), {
      target: { value: '0.20' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    await waitFor(() => expect(settingsSetMock).toHaveBeenCalledWith('rag_temperature', '0.2'))
  })

  it('restores RAG params defaults from the tab button', async () => {
    render(SettingsView)

    await screen.findByText(/sk-o\*\*\*\*\.\.\.\*\*\*\*-key/)
    await fireEvent.click(screen.getByRole('tab', { name: 'RAG Params' }))

    await fireEvent.input(screen.getByLabelText('topK (1-20)'), { target: { value: '15' } })
    expect(screen.getByLabelText('topK (1-20)')).toHaveValue('15')

    await fireEvent.click(screen.getByRole('button', { name: 'Restaurar defaults' }))

    expect(screen.getByLabelText('topK (1-20)')).toHaveValue('6')
  })

  it('blocks saving OCR and Summary prompts without the text placeholder', async () => {
    render(SettingsView)

    await fireEvent.click(await screen.findByRole('tab', { name: 'Prompts' }))
    await fireEvent.input(screen.getByLabelText('OCR correction prompt'), {
      target: { value: 'Custom OCR without placeholder' },
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    expect(
      await screen.findByText('OCR correction prompt: Debe incluir el placeholder {text}.')
    ).toBeInTheDocument()
    expect(settingsSetMock).not.toHaveBeenCalled()

    await fireEvent.input(screen.getByLabelText('OCR correction prompt'), {
      target: { value: 'Custom OCR {text}' },
    })
    await fireEvent.input(screen.getByLabelText('Summary prompt'), {
      target: { value: 'Summarize this document' },
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    expect(
      await screen.findByText('Summary prompt: Debe incluir el placeholder {text}.')
    ).toBeInTheDocument()
    expect(settingsSetMock).not.toHaveBeenCalled()
  })

  it('blocks saving NER prompts missing required labels', async () => {
    render(SettingsView)

    await fireEvent.click(await screen.findByRole('tab', { name: 'Prompts' }))
    await fireEvent.input(screen.getByLabelText('NER prompt'), {
      target: { value: 'Extract PER, LOC, ORG and DATE from {text}' },
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    expect(
      await screen.findByText('NER prompt: NER debe conservar estas etiquetas: MISC.')
    ).toBeInTheDocument()
    expect(settingsSetMock).not.toHaveBeenCalled()
  })

  it('blocks saving Triplets prompts missing required JSON keys', async () => {
    render(SettingsView)

    await fireEvent.click(await screen.findByRole('tab', { name: 'Prompts' }))
    await fireEvent.input(screen.getByLabelText('Triplets prompt'), {
      target: { value: 'Return subject and predicate for {text}' },
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    expect(
      await screen.findByText('Triplets prompt: Triplets debe conservar estas claves: object.')
    ).toBeInTheDocument()
    expect(settingsSetMock).not.toHaveBeenCalled()
  })

  it('validates prompt edits without saving settings', async () => {
    render(SettingsView)

    await fireEvent.click(await screen.findByRole('tab', { name: 'Prompts' }))
    const ocrPrompt = screen.getByLabelText('OCR correction prompt')
    const ocrPromptCard = ocrPrompt.closest('.settings__prompt-card')
    expect(ocrPromptCard).not.toBeNull()

    await fireEvent.input(ocrPrompt, { target: { value: 'Missing placeholder' } })
    await fireEvent.click(
      within(ocrPromptCard as HTMLElement).getByRole('button', { name: 'Validar cambios' })
    )

    expect(
      await within(ocrPromptCard as HTMLElement).findByText('Debe incluir el placeholder {text}.')
    ).toBeInTheDocument()
    expect(settingsSetMock).not.toHaveBeenCalled()

    await fireEvent.input(ocrPrompt, { target: { value: 'Correct {text}' } })
    await fireEvent.click(
      within(ocrPromptCard as HTMLElement).getByRole('button', { name: 'Validar cambios' })
    )

    expect(
      await within(ocrPromptCard as HTMLElement).findByText('Prompt válido.')
    ).toBeInTheDocument()
    expect(settingsSetMock).not.toHaveBeenCalled()
  })

  it('restores and saves the default prompt for an edited flow', async () => {
    render(SettingsView)

    await fireEvent.click(await screen.findByRole('tab', { name: 'Prompts' }))
    const ocrPrompt = screen.getByLabelText('OCR correction prompt')
    const ocrPromptCard = ocrPrompt.closest('.settings__prompt-card')
    expect(ocrPromptCard).not.toBeNull()

    await fireEvent.input(ocrPrompt, { target: { value: 'Custom OCR {text}' } })
    await fireEvent.click(
      within(ocrPromptCard as HTMLElement).getByRole('button', {
        name: 'Restaurar default',
      })
    )

    expect(ocrPrompt).toHaveValue(DEFAULT_PROMPTS.ocrCorrectionPrompt)

    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))
    expect(settingsSetMock).toHaveBeenCalledWith(
      'prompt_ocr_correction',
      DEFAULT_PROMPTS.ocrCorrectionPrompt
    )
  })

  it('opens provider API key links through the desktop bridge', async () => {
    render(SettingsView)

    await fireEvent.click(await screen.findByRole('link', { name: /OpenRouter/ }))
    await fireEvent.click(screen.getByRole('link', { name: /AssemblyAI/ }))
    await fireEvent.click(screen.getByRole('link', { name: /Z\.ai/ }))

    expect(invokeMock).toHaveBeenCalledWith('open_external_url', {
      url: 'https://openrouter.ai/settings/keys',
    })
    expect(invokeMock).toHaveBeenCalledWith('open_external_url', {
      url: 'https://www.assemblyai.com/app/account',
    })
    expect(invokeMock).toHaveBeenCalledWith('open_external_url', {
      url: 'https://z.ai/manage-apikey/apikey-list',
    })
  })

  it('shows refined success feedback for connection checks and saves', async () => {
    testOpenrouterConnectionMock.mockResolvedValue([
      { id: 'google/gemma-3-4b-it', name: 'Gemma 3 4B', context_length: 8192 },
      { id: 'anthropic/claude-3.7-sonnet', name: 'Claude 3.7 Sonnet', context_length: 200000 },
    ])

    render(SettingsView)

    const testButtons = await screen.findAllByRole('button', { name: 'Probar conexión' })
    expect(testButtons).toHaveLength(3)

    const openrouterTestButton = testButtons[0]
    const assemblyaiTestButton = testButtons[1]
    const glmOcrTestButton = testButtons[2]
    expect(openrouterTestButton).toBeDefined()
    expect(assemblyaiTestButton).toBeDefined()
    expect(glmOcrTestButton).toBeDefined()

    await waitFor(() => expect(openrouterTestButton!).toBeEnabled())
    await fireEvent.click(openrouterTestButton!)

    expect(await screen.findByText('Conexión lista · 2 modelos disponibles.')).toBeInTheDocument()
    expect(testOpenrouterConnectionMock).toHaveBeenCalledWith('sk-or-v1-test-key')
    expect(screen.getByText('Modelos sugeridos desde OpenRouter')).toBeInTheDocument()

    await fireEvent.click(assemblyaiTestButton!)

    expect(
      await screen.findByText('Conexión lista · AssemblyAI validó tu cuenta.')
    ).toBeInTheDocument()
    expect(testAssemblyaiConnectionMock).toHaveBeenCalledWith('aai-orig-test-1234')
    expect(screen.getByText(/aai-o\*\*\*\*\.\.\.\*\*\*\*1234/)).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    expect(
      await screen.findByText(
        'Configuración guardada. Ya podés usar esta preferencia en toda la app.'
      )
    ).toBeInTheDocument()
    // Pro persists the user-selected modes, not hardcoded cloud ones.
    expect(settingsSetMock).toHaveBeenCalledWith('embedding_provider', 'api')
    expect(settingsSetMock).toHaveBeenCalledWith('openrouter_embedding_model', 'baai/bge-m3')
    expect(settingsSetMock).toHaveBeenCalledWith('llm_mode', 'openrouter')
    expect(settingsSetMock).toHaveBeenCalledWith('stt_mode', 'assemblyai')
    expect(settingsSetMock).toHaveBeenCalledWith('ocrh_mode', 'glm_ocr')
    expect(settingsSetMock).toHaveBeenCalledWith('local_embedding_model_dir', '')
    expect(settingsSetMock).toHaveBeenCalledWith('assemblyai_role_speaker_identification', 'true')
  })

  it('loads collection audio AssemblyAI speaker labels enabled by default and saves it', async () => {
    render(SettingsView)

    const speakerSelect = await screen.findByLabelText(
      'Identificación de hablantes en audio de colección'
    )
    expect(speakerSelect).toHaveValue('true')

    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    expect(settingsSetMock).toHaveBeenCalledWith('assemblyai_role_speaker_identification', 'true')
  })

  it('respects a saved false value for collection audio AssemblyAI speaker labels', async () => {
    settingsGetMock.mockImplementation(async (key: string) => {
      if (key === 'openrouter_api_key') return 'sk-or-v1-test-key'
      if (key === 'openrouter_model') return 'anthropic/claude-3.7-sonnet'
      if (key === 'openrouter_embedding_model') return 'baai/bge-m3'
      if (key === 'assemblyai_api_key') return 'aai-orig-test-1234'
      if (key === 'assemblyai_role_speaker_identification') return 'false'
      return null
    })

    render(SettingsView)

    const speakerSelect = await screen.findByLabelText(
      'Identificación de hablantes en audio de colección'
    )
    await waitFor(() => expect(speakerSelect).toHaveValue('false'))

    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    expect(settingsSetMock).toHaveBeenCalledWith('assemblyai_role_speaker_identification', 'false')
  })

  it('enables connection tests for saved keyring credentials without retyping secrets', async () => {
    settingsGetMock.mockImplementation(async (key: string) => {
      if (key === 'openrouter_api_key') return 'secret_ref:openrouter_api_key'
      if (key === 'assemblyai_api_key') return 'secret_ref:assemblyai_api_key'
      if (key === 'glm_ocr_api_key') return 'secret_ref:glm_ocr_api_key'
      if (key === 'openrouter_model') return 'anthropic/claude-3.7-sonnet'
      if (key === 'openrouter_embedding_model') return 'baai/bge-m3'
      return null
    })
    testOpenrouterConnectionMock.mockResolvedValue([
      { id: 'google/gemma-4-26b-a4b-it', name: 'Gemma 4 26B', context_length: 8192 },
    ])

    render(SettingsView)

    const testButtons = await screen.findAllByRole('button', { name: 'Probar conexión' })
    expect(testButtons).toHaveLength(3)
    await waitFor(() => {
      expect(testButtons[0]).toBeEnabled()
      expect(testButtons[1]).toBeEnabled()
      expect(testButtons[2]).toBeEnabled()
    })

    await fireEvent.click(testButtons[0]!)
    await fireEvent.click(testButtons[1]!)
    await fireEvent.click(testButtons[2]!)

    expect(testOpenrouterConnectionMock).toHaveBeenCalledWith('')
    expect(testAssemblyaiConnectionMock).toHaveBeenCalledWith('')
    expect(testGlmOcrConnectionMock).toHaveBeenCalledWith('')
  })

  it('clears plaintext API keys from component state after saving them', async () => {
    render(SettingsView)

    const openRouterInput = await screen.findByLabelText('API Key', { selector: '#api-key' })
    await waitFor(() => expect(openRouterInput).toHaveValue('sk-or-v1-test-key'))

    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    await waitFor(() => expect(openRouterInput).toHaveValue(''))
    expect(
      screen.getAllByText(/Clave guardada en el almacén de credenciales del sistema/).length
    ).toBeGreaterThanOrEqual(1)
  })

  it('shows a retryable error when initial settings fail to load', async () => {
    settingsGetMock.mockRejectedValueOnce(new Error('credential store unavailable'))

    render(SettingsView)

    expect(
      await screen.findByText(
        'No se pudo cargar la configuración guardada: credential store unavailable'
      )
    ).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Reintentar carga' }))

    expect(await screen.findByText(/sk-o\*\*\*\*\.\.\.\*\*\*\*-key/)).toBeInTheDocument()
  })

  // --- Pro-only local model sections (preserved from Pro's original suite) ---

  it('saves the local BGE-M3 embedding provider and model directory', async () => {
    render(SettingsView)

    // Wait for the async settings load to settle before toggling: a
    // late-resolving load() would otherwise clobber the radio selection.
    await screen.findByText(/sk-o\*\*\*\*\.\.\.\*\*\*\*-key/)
    const localEmbeddingOption = await screen.findByRole('radio', { name: /Local ONNX/i })
    await fireEvent.click(localEmbeddingOption)

    const localPathInput = await screen.findByLabelText('Carpeta del modelo local BGE-M3')
    await fireEvent.input(localPathInput, { target: { value: 'C:/models/bge-m3' } })

    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    await waitFor(() => {
      expect(settingsSetMock).toHaveBeenCalledWith('embedding_provider', 'local')
      expect(settingsSetMock).toHaveBeenCalledWith('openrouter_embedding_model', 'baai/bge-m3')
      expect(settingsSetMock).toHaveBeenCalledWith('local_embedding_model_dir', 'C:/models/bge-m3')
    })
  })

  it('shows local BGE-M3 install status and can start the embedding asset download', async () => {
    render(SettingsView)

    await screen.findByText(/sk-o\*\*\*\*\.\.\.\*\*\*\*-key/)
    const localEmbeddingOption = await screen.findByRole('radio', { name: /Local ONNX/i })
    await fireEvent.click(localEmbeddingOption)

    expect(await screen.findByText('Modelo BGE-M3 local incompleto')).toBeInTheDocument()
    expect(screen.getAllByText(/model\.onnx_data/).length).toBeGreaterThan(0)
    expect(screen.getByText(/BAAI\/bge-m3/)).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Instalar BGE-M3 local' }))

    await waitFor(() => {
      expect(settingsSetMock).toHaveBeenCalledWith('embedding_provider', 'local')
      expect(settingsSetMock).toHaveBeenCalledWith('openrouter_embedding_model', 'baai/bge-m3')
      expect(settingsSetMock).toHaveBeenCalledWith('local_embedding_model_dir', '')
      expect(embeddingDownloadModelMock).toHaveBeenCalled()
    })
  })

  it('keeps the local BGE-M3 directory setting empty when using the AppData default', async () => {
    settingsGetMock.mockImplementation(async (key: string) => {
      if (key === 'embedding_provider') return 'local'
      if (key === 'local_embedding_model_dir') return 'resources/models/embeddings/bge-m3'
      if (key === 'openrouter_embedding_model') return 'baai/bge-m3'
      if (key === 'llm_mode') return 'openrouter'
      if (key === 'stt_mode') return 'assemblyai'
      if (key === 'language') return 'es'
      return null
    })

    render(SettingsView)

    const localPathInput = await screen.findByLabelText('Carpeta del modelo local BGE-M3')
    expect(localPathInput).toHaveValue('')
    expect(
      await screen.findByText(
        'C:/Users/test/AppData/Roaming/com.entropia.pro.desktop/models/embeddings/bge-m3'
      )
    ).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    await waitFor(() => {
      expect(settingsSetMock).toHaveBeenCalledWith('local_embedding_model_dir', '')
    })
  })

  it('opens the local BGE-M3 embeddings folder from Settings', async () => {
    render(SettingsView)

    await screen.findByText(/sk-o\*\*\*\*\.\.\.\*\*\*\*-key/)
    const localEmbeddingOption = await screen.findByRole('radio', { name: /Local ONNX/i })
    await fireEvent.click(localEmbeddingOption)
    await fireEvent.click(screen.getByRole('button', { name: 'Abrir carpeta BGE-M3' }))

    await waitFor(() => {
      expect(embeddingOpenModelsDirMock).toHaveBeenCalled()
    })
  })

  it('prefills the local Gemma download source from backend defaults', async () => {
    llmIsAvailableMock.mockResolvedValue(false)
    llmLocalModelInfoMock.mockResolvedValue({
      exists: false,
      available: true,
      can_auto_download: true,
      disabled_reason: null,
      path: '/home/test/.local/share/com.entropia.pro.desktop/models/gemma-4-E2B-it-Q4_K_M.gguf',
      size_bytes: null,
      filename: 'gemma-4-E2B-it-Q4_K_M.gguf',
      source_url:
        'https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF/resolve/main/gemma-4-E2B-it-Q4_K_M.gguf?download=true',
    })

    render(SettingsView)

    const sourceInput = await screen.findByLabelText('Fuente de descarga')
    expect(await screen.findByText('Listo para descargar desde la app')).toBeInTheDocument()
    expect(sourceInput).toHaveValue(
      'https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF/resolve/main/gemma-4-E2B-it-Q4_K_M.gguf?download=true'
    )
    expect(screen.getByLabelText('Nombre de archivo esperado')).toHaveValue(
      'gemma-4-E2B-it-Q4_K_M.gguf'
    )

    await fireEvent.click(screen.getByRole('button', { name: 'Descargar modelo' }))

    await waitFor(() => {
      expect(settingsSetMock).toHaveBeenCalledWith(
        'local_model_source_url',
        'https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF/resolve/main/gemma-4-E2B-it-Q4_K_M.gguf?download=true'
      )
      expect(settingsSetMock).toHaveBeenCalledWith(
        'local_model_filename',
        'gemma-4-E2B-it-Q4_K_M.gguf'
      )
      expect(llmDownloadModelMock).toHaveBeenCalled()
    })
  })
})

describe('settings dirty detection helpers', () => {
  const baseInput: SettingsSnapshotInput = {
    apiKey: '',
    model: 'anthropic/claude-3.7-sonnet',
    embeddingModel: 'baai/bge-m3',
    embeddingProvider: 'api',
    localEmbeddingModelDir: '',
    llmMode: 'local',
    sttMode: 'local',
    ocrhMode: 'local',
    localModelSourceUrl: '',
    localModelFilename: '',
    assemblyAiApiKey: '',
    assemblyAiCollectionSpeakerLabels: true,
    glmOcrApiKey: '',
    ocrCorrectionPrompt: 'Correct {text}',
    summaryPrompt: 'Summarize {text}',
    nerPrompt: 'NER {text}',
    tripletsPrompt: 'Triples {text}',
    modelParamsByFlow: {
      summary: { temperature: '0.2', maxTokens: '' },
    },
    ragParams: { topK: '6', temperature: '0.2' },
  }

  it('is clean when the current snapshot matches the saved baseline', () => {
    const saved = buildSettingsSnapshot(baseInput)
    expect(hasUnsavedSettingsChanges(saved, buildSettingsSnapshot({ ...baseInput }))).toBe(false)
  })

  it('flags top-level and nested model param changes as dirty', () => {
    const saved = buildSettingsSnapshot(baseInput)

    expect(
      hasUnsavedSettingsChanges(
        saved,
        buildSettingsSnapshot({ ...baseInput, model: 'openai/gpt-test' })
      )
    ).toBe(true)
    expect(
      hasUnsavedSettingsChanges(
        saved,
        buildSettingsSnapshot({ ...baseInput, llmMode: 'openrouter' })
      )
    ).toBe(true)
    expect(
      hasUnsavedSettingsChanges(
        saved,
        buildSettingsSnapshot({ ...baseInput, summaryPrompt: 'Edited {text}' })
      )
    ).toBe(true)
    expect(
      hasUnsavedSettingsChanges(
        saved,
        buildSettingsSnapshot({
          ...baseInput,
          modelParamsByFlow: { summary: { temperature: '0.9', maxTokens: '' } },
        })
      )
    ).toBe(true)
    expect(
      hasUnsavedSettingsChanges(
        saved,
        buildSettingsSnapshot({ ...baseInput, ragParams: { topK: '12', temperature: '0.2' } })
      )
    ).toBe(true)
  })

  it('never reports dirty before a baseline snapshot exists', () => {
    expect(hasUnsavedSettingsChanges(null, buildSettingsSnapshot(baseInput))).toBe(false)
  })
})

describe('SettingsView Escape behavior', () => {
  beforeEach(() => {
    locale.set('es')
    invokeMock.mockReset().mockResolvedValue(undefined)
    settingsGetMock.mockReset()
    settingsGetAllMock.mockReset().mockResolvedValue([])
    settingsSetMock.mockReset().mockResolvedValue(undefined)
    llmLocalModelInfoMock.mockReset().mockResolvedValue(null)
    embeddingLocalModelInfoMock.mockReset().mockResolvedValue(null)
    settingsGetMock.mockImplementation(async (key: string) => {
      if (key === 'openrouter_api_key') return 'sk-or-v1-test-key'
      if (key === 'openrouter_model') return 'anthropic/claude-3.7-sonnet'
      if (key === 'openrouter_embedding_model') return 'baai/bge-m3'
      if (key === 'assemblyai_api_key') return 'aai-orig-test-1234'
      return null
    })
  })

  async function renderLoadedSettings() {
    render(SettingsView)
    await screen.findByText(/sk-o\*\*\*\*\.\.\.\*\*\*\*-key/)
  }

  it('lets Escape navigate back when settings have no unsaved changes', async () => {
    const backSpy = vi.spyOn(navigation, 'back').mockImplementation(() => {})
    const cleanupKeyboard = setupKeyboardShortcuts()

    try {
      await renderLoadedSettings()

      await fireEvent.keyDown(window, { key: 'Escape' })

      expect(backSpy).toHaveBeenCalledTimes(1)
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    } finally {
      cleanupKeyboard()
      backSpy.mockRestore()
    }
  })

  it('asks before discarding unsaved changes on Escape and navigates only after confirming', async () => {
    const backSpy = vi.spyOn(navigation, 'back').mockImplementation(() => {})
    const cleanupKeyboard = setupKeyboardShortcuts()

    try {
      await renderLoadedSettings()

      await fireEvent.input(screen.getByLabelText('Modelo generativo'), {
        target: { value: 'openai/gpt-test' },
      })

      await fireEvent.keyDown(window, { key: 'Escape' })

      expect(backSpy).not.toHaveBeenCalled()
      expect(await screen.findByRole('dialog')).toBeInTheDocument()
      expect(screen.getByText('Descartar cambios')).toBeInTheDocument()

      // Keep editing → dialog closes, still on settings.
      await fireEvent.click(screen.getByRole('button', { name: 'Seguir editando' }))
      await waitFor(() => {
        expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
      })
      expect(backSpy).not.toHaveBeenCalled()

      // Escape again and confirm the discard → now it navigates back.
      await fireEvent.keyDown(window, { key: 'Escape' })
      await fireEvent.click(await screen.findByRole('button', { name: 'Descartar' }))

      expect(backSpy).toHaveBeenCalledTimes(1)
      await waitFor(() => {
        expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
      })
    } finally {
      cleanupKeyboard()
      backSpy.mockRestore()
    }
  })

  it('does not prompt on Escape after saving the edited settings', async () => {
    const backSpy = vi.spyOn(navigation, 'back').mockImplementation(() => {})
    const cleanupKeyboard = setupKeyboardShortcuts()

    try {
      await renderLoadedSettings()

      await fireEvent.input(screen.getByLabelText('Modelo generativo'), {
        target: { value: 'openai/gpt-test' },
      })
      await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))
      await screen.findByText(
        'Configuración guardada. Ya podés usar esta preferencia en toda la app.'
      )

      await fireEvent.keyDown(window, { key: 'Escape' })

      expect(backSpy).toHaveBeenCalledTimes(1)
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    } finally {
      cleanupKeyboard()
      backSpy.mockRestore()
    }
  })
})
