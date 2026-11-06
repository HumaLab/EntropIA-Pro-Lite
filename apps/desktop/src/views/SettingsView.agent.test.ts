import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import SettingsView from './SettingsView.svelte'
import { locale } from '$lib/i18n'

/**
 * La solapa «Agente» edita todos los parámetros del motor de investigación.
 *
 * Los defaults y la validación son del motor: acá se mockean los dos comandos
 * (`research_parameters_defaults` y `research_parameters_validate`) y se
 * comprueba que la solapa los use sin repetir sus valores ni sus reglas.
 */

const {
  invokeMock,
  settingsGetMock,
  settingsGetAllMock,
  settingsSetMock,
  settingsDeleteMock,
  researchParametersDefaultsMock,
  researchParametersValidateMock,
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
  settingsDeleteMock: vi.fn(),
  researchParametersDefaultsMock: vi.fn(),
  researchParametersValidateMock: vi.fn(),
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
    settingsDelete: settingsDeleteMock,
    testOpenrouterConnection: vi.fn(),
    testAssemblyaiConnection: vi.fn(),
    testGlmOcrConnection: vi.fn(),
    verifyZoteroKey: vi.fn(),
  }
})

vi.mock('$lib/research', async () => {
  const actual = await vi.importActual<typeof import('$lib/research')>('$lib/research')
  return {
    ...actual,
    researchParametersDefaults: researchParametersDefaultsMock,
    researchParametersValidate: researchParametersValidateMock,
  }
})

vi.mock('$lib/llm', () => ({
  llmIsAvailable: vi.fn(),
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

/** `ParametrosAgente::default()` del motor, con sus 13 campos. */
const ENGINE_DEFAULTS = {
  max_tokens_salida: 16384,
  temperatura: 0.3,
  timeout_s: 180,
  esperas_reintento_s: [2, 6],
  lotes_archivo_bytes: [48000, 24000, 12000],
  fragmento_archivo_bytes: 8000,
  ventana_cita: 600,
  preguntas_minimas: 4,
  rerank_depth: 16,
  leg_k: 24,
  rrf_k: 60,
  snippet_max: 1600,
  max_consultas: { general: 20, trayectorias: 30, cronologia: 20 },
}

const ENGINE_MESSAGE = 'max_tokens_salida debe estar entre 1024 y 131072 (recibido: 10)'

beforeEach(() => {
  locale.set('es')
  invokeMock.mockReset().mockResolvedValue(undefined)
  settingsGetMock.mockReset().mockImplementation(async (key: string) => {
    if (key === 'openrouter_api_key') return 'sk-or-v1-test-key'
    if (key === 'openrouter_model') return 'anthropic/claude-3.7-sonnet'
    return null
  })
  settingsGetAllMock.mockReset().mockResolvedValue([])
  settingsSetMock.mockReset().mockResolvedValue(undefined)
  settingsDeleteMock.mockReset().mockResolvedValue(undefined)
  researchParametersDefaultsMock.mockReset().mockResolvedValue(ENGINE_DEFAULTS)
  researchParametersValidateMock.mockReset().mockResolvedValue(undefined)
  llmLocalModelInfoMock.mockReset().mockResolvedValue(null)
  llmDownloadModelMock.mockReset().mockResolvedValue(undefined)
  embeddingLocalModelInfoMock.mockReset().mockResolvedValue(null)
  embeddingOpenModelsDirMock.mockReset().mockResolvedValue(undefined)
  embeddingDownloadModelMock.mockReset().mockResolvedValue('started')
  rerankerLocalModelInfoMock.mockReset().mockResolvedValue(null)
  rerankerOpenModelsDirMock.mockReset().mockResolvedValue(undefined)
  rerankerDownloadModelMock.mockReset().mockResolvedValue('started')
})

/** Abre la solapa Agente después de que la vista terminó de cargar. */
async function openAgentTab() {
  await waitFor(() => expect(researchParametersDefaultsMock).toHaveBeenCalled())
  await fireEvent.click(screen.getByRole('tab', { name: 'Agente' }))
}

describe('SettingsView — solapa Agente', () => {
  it('aparece justo después de RAG Params y muestra los 13 campos con los defaults del motor', async () => {
    render(SettingsView)

    await waitFor(() => expect(researchParametersDefaultsMock).toHaveBeenCalled())
    const tabs = screen.getAllByRole('tab').map((tab) => tab.textContent?.trim())
    expect(tabs.indexOf('Agente')).toBe(tabs.indexOf('RAG Params') + 1)

    await openAgentTab()

    expect(
      screen.getByRole('heading', { name: 'Parámetros del Agente de Investigación' })
    ).toBeInTheDocument()
    expect(screen.getByLabelText(/max_tokens_salida/)).toHaveValue('16384')
    expect(screen.getByLabelText(/temperatura/)).toHaveValue('0.3')
    expect(screen.getByLabelText(/timeout_s/)).toHaveValue('180')
    expect(screen.getByLabelText(/esperas_reintento_s/)).toHaveValue('2, 6')
    expect(screen.getByLabelText(/lotes_archivo_bytes/)).toHaveValue('48000, 24000, 12000')
    expect(screen.getByLabelText(/fragmento_archivo_bytes/)).toHaveValue('8000')
    expect(screen.getByLabelText(/ventana_cita/)).toHaveValue('600')
    expect(screen.getByLabelText(/preguntas_minimas/)).toHaveValue('4')
    expect(screen.getByLabelText(/rerank_depth/)).toHaveValue('16')
    expect(screen.getByLabelText(/leg_k/)).toHaveValue('24')
    expect(screen.getByLabelText(/rrf_k/)).toHaveValue('60')
    expect(screen.getByLabelText(/snippet_max/)).toHaveValue('1600')
    expect(screen.getByLabelText('general')).toHaveValue('20')
    expect(screen.getByLabelText('trayectorias')).toHaveValue('30')
    expect(screen.getByLabelText('cronologia')).toHaveValue('20')
    // Cada campo muestra su ayuda y su valor por defecto.
    expect(screen.getByText('Por defecto: 16384')).toBeInTheDocument()
    expect(
      screen.getByText(
        'Bytes máximos del insumo de un lote de archivo, de mayor a menor: cada reintento usa el tope siguiente.'
      )
    ).toBeInTheDocument()
  })

  it('muestra el mensaje exacto del motor y no persiste nada cuando los parámetros no validan', async () => {
    researchParametersValidateMock.mockRejectedValue(ENGINE_MESSAGE)
    render(SettingsView)

    await openAgentTab()
    await fireEvent.input(screen.getByLabelText(/max_tokens_salida/), { target: { value: '10' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    expect(await screen.findAllByText(ENGINE_MESSAGE)).not.toHaveLength(0)
    // El mensaje queda al lado del formulario, no solo en el aviso de la página.
    expect(screen.getByRole('alert')).toHaveTextContent(ENGINE_MESSAGE)
    expect(researchParametersValidateMock).toHaveBeenCalledWith(
      expect.objectContaining({ max_tokens_salida: 10, lotes_archivo_bytes: [48000, 24000, 12000] })
    )
    expect(settingsSetMock).not.toHaveBeenCalled()
  })

  it('guarda el JSON completo una sola vez cuando el motor los acepta', async () => {
    render(SettingsView)

    await openAgentTab()
    await fireEvent.input(screen.getByLabelText(/temperatura/), { target: { value: '1.1' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Guardar cambios' }))

    await waitFor(() =>
      expect(settingsSetMock).toHaveBeenCalledWith(
        'research_agent_parameters',
        JSON.stringify({ ...ENGINE_DEFAULTS, temperatura: 1.1 })
      )
    )
    const writes = settingsSetMock.mock.calls.filter(([key]) => key === 'research_agent_parameters')
    expect(writes).toHaveLength(1)
  })

  it('lee los parámetros guardados y completa lo que falta con los defaults', async () => {
    settingsGetAllMock.mockResolvedValue([
      {
        key: 'research_agent_parameters',
        value: JSON.stringify({ max_tokens_salida: 2048, max_consultas: { general: 25 } }),
      },
    ])
    render(SettingsView)

    await openAgentTab()

    expect(screen.getByLabelText(/max_tokens_salida/)).toHaveValue('2048')
    expect(screen.getByLabelText(/temperatura/)).toHaveValue('0.3')
    expect(screen.getByLabelText('general')).toHaveValue('25')
    expect(screen.getByLabelText('trayectorias')).toHaveValue('30')
  })

  it('restaura los defaults del motor y borra la clave guardada', async () => {
    settingsGetAllMock.mockResolvedValue([
      {
        key: 'research_agent_parameters',
        value: JSON.stringify({ ...ENGINE_DEFAULTS, max_tokens_salida: 2048, temperatura: 1.1 }),
      },
    ])
    render(SettingsView)

    await openAgentTab()
    expect(screen.getByLabelText(/max_tokens_salida/)).toHaveValue('2048')

    await fireEvent.click(screen.getByRole('button', { name: 'Restaurar valores por defecto' }))

    expect(screen.getByLabelText(/max_tokens_salida/)).toHaveValue('16384')
    expect(screen.getByLabelText(/temperatura/)).toHaveValue('0.3')
    await waitFor(() =>
      expect(settingsDeleteMock).toHaveBeenCalledWith('research_agent_parameters')
    )
  })
})
