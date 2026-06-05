import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import DependenciasTab from './DependenciasTab.svelte'

const depsMocks = vi.hoisted(() => ({
  checkAllDeps: vi.fn(),
  installAllDeps: vi.fn(),
  installOneDep: vi.fn(),
  getUvStatus: vi.fn(),
  resetDeps: vi.fn(),
  onDepsProgress: vi.fn(),
  onDepsComplete: vi.fn(),
  onDepsError: vi.fn(),
  getRuntimeStatus: vi.fn(),
  repairRuntime: vi.fn(),
  onRuntimeStatus: vi.fn(),
  onRuntimeProgress: vi.fn(),
  runtimeCanBootstrapAutomatically: vi.fn(),
  llmLocalModelInfo: vi.fn(),
  llmDownloadModel: vi.fn(),
  embeddingLocalModelInfo: vi.fn(),
  embeddingDownloadModel: vi.fn(),
  listen: vi.fn(),
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: depsMocks.listen,
}))

vi.mock('$lib/deps', () => ({
  checkAllDeps: depsMocks.checkAllDeps,
  installAllDeps: depsMocks.installAllDeps,
  installOneDep: depsMocks.installOneDep,
  getUvStatus: depsMocks.getUvStatus,
  resetDeps: depsMocks.resetDeps,
  onDepsProgress: depsMocks.onDepsProgress,
  onDepsComplete: depsMocks.onDepsComplete,
  onDepsError: depsMocks.onDepsError,
  DEP_DISPLAY_NAMES: {
    Python: 'Python 3.11',
    Fastembed: 'Fastembed (legacy embeddings)',
    PaddlePaddle: 'PaddlePaddle (runtime OCR)',
    PaddleOcr: 'PaddleOCR (OCR principal)',
    FasterWhisper: 'Faster Whisper (transcripción)',
    Spacy: 'spaCy (NER)',
    SpacyModelEs: 'Modelo spaCy español',
  },
  DEP_DESCRIPTIONS: {
    Python: 'Intérprete Python requerido para todas las funciones de IA',
    Fastembed: 'Motor legacy de embeddings; el perfil liviano usa OpenRouter BGE-M3',
    PaddlePaddle: 'Base de ejecución requerida por PaddleOCR-VL',
    PaddleOcr: 'Motor principal de reconocimiento óptico de caracteres',
    FasterWhisper: 'Transcripción de audio a texto',
    Spacy: 'Reconocimiento de entidades nombradas',
    SpacyModelEs: 'Modelo de lenguaje español para spaCy',
  },
  CRITICAL_DEPS: ['Python', 'PaddlePaddle', 'PaddleOcr'],
}))

vi.mock('$lib/runtime', () => ({
  getRuntimeStatus: depsMocks.getRuntimeStatus,
  repairRuntime: depsMocks.repairRuntime,
  onRuntimeStatus: depsMocks.onRuntimeStatus,
  onRuntimeProgress: depsMocks.onRuntimeProgress,
  runtimeNeedsAttention: (status: { state?: string } | null | undefined) =>
    status != null && ['repairing', 'damaged', 'fixture', 'incompatible', 'blocked_source_unavailable', 'blocked_offline', 'checking', 'hydrating', 'verifying', 'downloading'].includes(status.state ?? ''),
  runtimeBlocksCurrentUse: (status: { state?: string } | null | undefined, localDepsReady: boolean) =>
    !(status?.state === 'fixture' && localDepsReady) &&
    status != null && ['repairing', 'damaged', 'fixture', 'incompatible', 'blocked_source_unavailable', 'blocked_offline', 'checking', 'hydrating', 'verifying', 'downloading'].includes(status.state ?? ''),
  shouldShowRuntimeRepairAction: (status: { state?: string; repairAvailable?: boolean } | null | undefined) =>
    status?.repairAvailable === true && !['repairing', 'fixture', 'incompatible', 'blocked_source_unavailable', 'blocked_offline'].includes(status?.state ?? ''),
  runtimeCanBootstrapAutomatically: depsMocks.runtimeCanBootstrapAutomatically,
}))

vi.mock('$lib/llm', () => ({
  llmLocalModelInfo: depsMocks.llmLocalModelInfo,
  llmDownloadModel: depsMocks.llmDownloadModel,
}))

vi.mock('$lib/embeddings', () => ({
  embeddingLocalModelInfo: depsMocks.embeddingLocalModelInfo,
  embeddingDownloadModel: depsMocks.embeddingDownloadModel,
}))

describe('DependenciasTab', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    depsMocks.checkAllDeps.mockResolvedValue([
      { id: 'Python', status: { type: 'installed', version: '3.11.9' }, version: '3.11.9' },
    ])
    depsMocks.getUvStatus.mockResolvedValue({
      uv_ready: true,
      uv_path: '/runtime/uv',
      uv_version: '0.6.0',
      uv_source: 'managed-runtime',
      uv_compatible_for_dev: true,
      venv_exists: true,
      venv_path: '/runtime/venv',
      uv_warning: null,
      release_runtime_ready: true,
      release_runtime_state: 'healthy',
      dev_fallback_available: false,
      dev_fallback_reason: null,
    })
    depsMocks.onDepsProgress.mockResolvedValue(vi.fn())
    depsMocks.onDepsComplete.mockResolvedValue(vi.fn())
    depsMocks.onDepsError.mockResolvedValue(vi.fn())
    depsMocks.onRuntimeStatus.mockResolvedValue(vi.fn())
    depsMocks.onRuntimeProgress.mockResolvedValue(vi.fn())
    depsMocks.listen.mockResolvedValue(vi.fn())
    depsMocks.runtimeCanBootstrapAutomatically.mockReturnValue(false)
    depsMocks.llmDownloadModel.mockResolvedValue('started')
    depsMocks.embeddingDownloadModel.mockResolvedValue('started')
    depsMocks.llmLocalModelInfo.mockResolvedValue({
      exists: true,
      available: true,
      can_auto_download: false,
      disabled_reason: null,
      path: '/runtime/models/gemma-4-E2B-it-Q4_K_M.gguf',
      size_bytes: 2_500_000_000,
      filename: 'gemma-4-E2B-it-Q4_K_M.gguf',
      source_url: 'https://example.invalid/gemma.gguf',
    })
    depsMocks.embeddingLocalModelInfo.mockResolvedValue({
      exists: true,
      available: true,
      can_auto_download: false,
      directory: '/runtime/models/embeddings/bge-m3',
      path: '/runtime/models/embeddings/bge-m3/model.onnx',
      size_bytes: 2_200_000_000,
      required_files: [],
      missing_files: [],
      source_repo: 'BAAI/bge-m3',
    })
    depsMocks.resetDeps.mockResolvedValue(undefined)
    depsMocks.getRuntimeStatus.mockResolvedValue({
      state: 'healthy',
      packVersion: '2026.05.0',
      repairNeeded: false,
      repairAvailable: false,
      summary: 'Runtime listo',
      blockedCapabilities: [],
      details: [],
      guidance: [],
      bootstrapEligible: false,
      bootstrapRequired: false,
      activeOperation: null,
    })
    depsMocks.repairRuntime.mockResolvedValue({
      state: 'healthy',
      packVersion: '2026.05.0',
      repairNeeded: false,
      repairAvailable: false,
      summary: 'Runtime listo',
      blockedCapabilities: [],
      details: [],
      guidance: [],
      bootstrapEligible: false,
      bootstrapRequired: false,
      activeOperation: null,
    })
  })

  it('shows runtime status details and repair CTA for damaged runtime', async () => {
    depsMocks.getRuntimeStatus.mockResolvedValueOnce({
      state: 'damaged',
      packVersion: '2026.05.0',
      repairNeeded: true,
      repairAvailable: true,
      summary: 'Runtime dañado',
      blockedCapabilities: ['ocr', 'nlp'],
      details: ['Checksum inválido'],
      guidance: ['Ejecutá la reparación del runtime desde Ajustes > Dependencias.'],
      bootstrapEligible: true,
      bootstrapRequired: true,
      activeOperation: null,
    })

    render(DependenciasTab)

    expect(await screen.findByText('Runtime dañado')).toBeInTheDocument()
    expect(screen.getByText(/ocr, nlp/i)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Reparar runtime' })).toBeInTheDocument()
  })

  it('invokes runtime repair from the dedicated runtime panel', async () => {
    depsMocks.getRuntimeStatus.mockResolvedValueOnce({
      state: 'damaged',
      packVersion: '2026.05.0',
      repairNeeded: true,
      repairAvailable: true,
      summary: 'Runtime dañado',
      blockedCapabilities: ['transcription'],
      details: ['Falta transcribe.py'],
      guidance: ['Podés intentar reparar el runtime.'],
      bootstrapEligible: true,
      bootstrapRequired: true,
      activeOperation: null,
    })

    render(DependenciasTab)

    const repairButton = await screen.findByRole('button', { name: 'Reparar runtime' })
    await fireEvent.click(repairButton)

    await waitFor(() => {
      expect(depsMocks.repairRuntime).toHaveBeenCalledTimes(1)
    })
  })

  it('shows fixture runtime messaging without exposing repair CTA', async () => {
    depsMocks.checkAllDeps.mockResolvedValueOnce([
      { id: 'Python', status: { type: 'installed', version: '3.11.9' }, version: '3.11.9' },
      { id: 'PaddleOcr', status: { type: 'missing' }, version: null },
    ])
    depsMocks.getRuntimeStatus.mockResolvedValueOnce({
      state: 'fixture',
      packVersion: '2026.05.0',
      repairNeeded: false,
      repairAvailable: false,
      summary: 'Runtime de desarrollo detectado para linux-x86_64: faltan payloads externos de release',
      blockedCapabilities: ['ocr', 'transcription', 'nlp'],
      details: ['La app 0.0.10 arrancó correctamente, pero este runtime-pack todavía está en modo fixture/dev (app_version declarada: 0.0.10).'],
      guidance: ['Próximo paso manual inevitable: inyectar los artefactos externos requeridos al runtime-pack de release para esta plataforma.'],
      bootstrapEligible: false,
      bootstrapRequired: true,
      activeOperation: null,
    })

    render(DependenciasTab)

    expect(await screen.findByText(/Runtime de desarrollo detectado/i)).toBeInTheDocument()
    expect(screen.getByText(/Próximo paso manual inevitable/i)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Reparar runtime' })).not.toBeInTheDocument()
  })

  it('renders PaddlePaddle metadata and avoids ready success when runtime is blocked', async () => {
    depsMocks.checkAllDeps.mockResolvedValueOnce([
      { id: 'Python', status: { type: 'installed', version: '3.11.9' }, version: '3.11.9' },
      { id: 'Fastembed', status: { type: 'installed', version: '0.7.0' }, version: '0.7.0' },
      {
        id: 'PaddlePaddle',
        status: { type: 'installed', version: '3.2.2' },
        version: '3.2.2',
      },
      { id: 'PaddleOcr', status: { type: 'installed', version: '3.3.0' }, version: '3.3.0' },
    ])
    depsMocks.getRuntimeStatus.mockResolvedValueOnce({
      state: 'incompatible',
      packVersion: '2026.05.0',
      repairNeeded: false,
      repairAvailable: false,
      summary: 'Runtime incompatible con EntropIA Pro 0.0.13',
      blockedCapabilities: ['ocr', 'transcription', 'nlp'],
      details: ['El runtime-pack declara app_version 0.0.10 pero la app usa 0.0.13'],
      guidance: ['Regenerá o seleccioná un runtime-pack compatible.'],
      bootstrapEligible: false,
      bootstrapRequired: true,
      activeOperation: null,
    })
    depsMocks.getUvStatus.mockResolvedValueOnce({
      uv_ready: true,
      uv_path: '/runtime/uv',
      uv_version: '0.6.14 (a4cec56dc 2025-04-09)',
      uv_source: 'strict-compatible',
      uv_compatible_for_dev: true,
      venv_exists: true,
      venv_path: '/runtime/venv',
      uv_warning: null,
      release_runtime_ready: false,
      release_runtime_state: 'incompatible',
      dev_fallback_available: false,
      dev_fallback_reason: null,
    })

    render(DependenciasTab)

    expect(await screen.findByText('PaddlePaddle (runtime OCR)')).toBeInTheDocument()
    expect(screen.getByText('Base de ejecución requerida por PaddleOCR-VL')).toBeInTheDocument()
    expect(
      screen.queryByText('Todas las dependencias están instaladas y listas para usar.')
    ).not.toBeInTheDocument()
    expect(screen.getByText(/runtime de EntropIA Pro necesita atención/i)).toBeInTheDocument()
    expect(screen.getByText(/Gestión automática pausada/i)).toBeInTheDocument()
  })

  it('does not surface fixture runtime messaging when local deps are installed', async () => {
    depsMocks.checkAllDeps.mockResolvedValueOnce([
      { id: 'Python', status: { type: 'installed', version: '3.11.9' }, version: '3.11.9' },
      { id: 'Fastembed', status: { type: 'installed', version: '0.7.0' }, version: '0.7.0' },
      {
        id: 'PaddlePaddle',
        status: { type: 'installed', version: '3.2.2' },
        version: '3.2.2',
      },
      { id: 'PaddleOcr', status: { type: 'installed', version: '3.5.0' }, version: '3.5.0' },
      { id: 'FasterWhisper', status: { type: 'installed', version: '1.2.1' }, version: '1.2.1' },
      { id: 'Spacy', status: { type: 'installed', version: '3.8.7' }, version: '3.8.7' },
      { id: 'SpacyModelEs', status: { type: 'installed', version: '3.8.0' }, version: '3.8.0' },
    ])
    depsMocks.getRuntimeStatus.mockResolvedValueOnce({
      state: 'fixture',
      packVersion: '2026.05.0',
      repairNeeded: false,
      repairAvailable: false,
      summary: 'Runtime de desarrollo detectado para linux-x86_64: faltan payloads externos de release',
      blockedCapabilities: ['ocr', 'transcription', 'nlp'],
      details: ['La app funciona localmente, pero el runtime-pack offline sigue pendiente.'],
      guidance: ['Inyectar payloads externos antes de distribuir offline.'],
      bootstrapEligible: false,
      bootstrapRequired: true,
      activeOperation: null,
    })
    depsMocks.getUvStatus.mockResolvedValueOnce({
      uv_ready: false,
      uv_path: null,
      uv_version: null,
      uv_source: null,
      uv_compatible_for_dev: false,
      venv_exists: true,
      venv_path: '/runtime/venv',
      uv_warning: null,
      release_runtime_ready: false,
      release_runtime_state: 'fixture',
      dev_fallback_available: false,
      dev_fallback_reason: null,
    })

    render(DependenciasTab)

    expect(await screen.findByText('Todas las dependencias están instaladas y listas para usar.')).toBeInTheDocument()
    expect(screen.queryByText(/Runtime de desarrollo detectado/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/payloads externos/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/runtime-pack release pendiente/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/^Capacidades afectadas:/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/antes de habilitar OCR/i)).not.toBeInTheDocument()
  })

  it('shows release bootstrap blockers when the runtime wiring actually fails', async () => {
    depsMocks.checkAllDeps.mockResolvedValueOnce([
      { id: 'Python', status: { type: 'installed', version: '3.11.9' }, version: '3.11.9' },
      { id: 'Fastembed', status: { type: 'installed', version: '0.7.0' }, version: '0.7.0' },
      {
        id: 'PaddlePaddle',
        status: { type: 'installed', version: '3.2.2' },
        version: '3.2.2',
      },
      { id: 'PaddleOcr', status: { type: 'installed', version: '3.5.0' }, version: '3.5.0' },
      { id: 'FasterWhisper', status: { type: 'installed', version: '1.2.1' }, version: '1.2.1' },
      { id: 'Spacy', status: { type: 'installed', version: '3.8.7' }, version: '3.8.7' },
      { id: 'SpacyModelEs', status: { type: 'installed', version: '3.8.0' }, version: '3.8.0' },
    ])
    depsMocks.getRuntimeStatus.mockResolvedValueOnce({
      state: 'blocked_source_unavailable',
      packVersion: '2026.05.0',
      repairNeeded: false,
      repairAvailable: false,
      summary: 'No hay una fuente confiable disponible para bootstrap',
      blockedCapabilities: ['ocr', 'transcription', 'nlp'],
      details: ['Trusted remote bootstrap source wiring is not implemented yet'],
      guidance: ['Reintentá cuando exista una fuente confiable'],
      bootstrapEligible: false,
      bootstrapRequired: true,
      activeOperation: null,
    })
    depsMocks.getUvStatus.mockResolvedValueOnce({
      uv_ready: false,
      uv_path: null,
      uv_version: null,
      uv_source: null,
      uv_compatible_for_dev: false,
      venv_exists: true,
      venv_path: '/runtime/venv',
      uv_warning: null,
      release_runtime_ready: false,
      release_runtime_state: 'blocked_source_unavailable',
      dev_fallback_available: false,
      dev_fallback_reason: 'En Windows dev el fallback online no está habilitado.',
    })

    render(DependenciasTab)

    expect(
      await screen.findByText('No hay una fuente confiable disponible para bootstrap'),
    ).toBeInTheDocument()
    expect(screen.getByText(/Trusted remote bootstrap source wiring/i)).toBeInTheDocument()
    expect(screen.getByText(/Reintentá cuando exista una fuente confiable/i)).toBeInTheDocument()
    expect(screen.getByText(/En Windows dev el fallback online no está habilitado/i)).toBeInTheDocument()
    expect(screen.getByText(/antes de habilitar OCR/i)).toBeInTheDocument()
    expect(screen.getByText(/^Capacidades afectadas:/i)).toBeInTheDocument()
  })

  it('labels healthy embedded runtime without a managed venv honestly', async () => {
    depsMocks.getUvStatus.mockResolvedValueOnce({
      uv_ready: true,
      uv_path: 'C:\\Users\\test\\AppData\\Roaming\\com.entropia.pro.desktop\\runtime\\2026.05.0\\uv\\uv.exe',
      uv_version: '0.6.14 (a4cec56dc 2025-04-09)',
      uv_source: 'managed-runtime',
      uv_compatible_for_dev: true,
      venv_exists: false,
      venv_path: null,
      uv_warning: null,
      release_runtime_ready: true,
      release_runtime_state: 'healthy',
      dev_fallback_available: false,
      dev_fallback_reason: null,
    })

    render(DependenciasTab)

    expect(await screen.findByText(/runtime embebido · sin venv administrado/i)).toBeInTheDocument()
    expect(screen.queryByText(/sin entorno virtual$/i)).not.toBeInTheDocument()
  })

  it('requires typed confirmation before resetting the environment', async () => {
    render(DependenciasTab)

    await fireEvent.click(await screen.findByRole('button', { name: 'Resetear entorno' }))

    const dialog = screen.getByRole('alertdialog')
    expect(dialog).toBeInTheDocument()
    expect(dialog).toHaveAttribute('aria-modal', 'true')
    expect(dialog).toHaveAccessibleName('Confirmar reseteo del entorno')
    expect(dialog).toHaveAccessibleDescription(/Esta acción elimina el entorno administrado/i)
    expect(screen.getByText('Confirmar reseteo del entorno')).toBeInTheDocument()
    const confirmButton = screen.getByRole('button', { name: 'Confirmar reseteo' })
    expect(confirmButton).toBeDisabled()

    await fireEvent.input(screen.getByLabelText('Confirmación requerida'), {
      target: { value: 'resetear' },
    })
    expect(confirmButton).toBeDisabled()
    expect(depsMocks.resetDeps).not.toHaveBeenCalled()

    await fireEvent.input(screen.getByLabelText('Confirmación requerida'), {
      target: { value: 'resetear entorno' },
    })
    expect(confirmButton).not.toBeDisabled()
    await fireEvent.click(confirmButton)

    await waitFor(() => {
      expect(depsMocks.resetDeps).toHaveBeenCalledTimes(1)
    })
  })

  it('closes reset confirmation with Escape and restores focus to the trigger', async () => {
    render(DependenciasTab)

    const resetButton = await screen.findByRole('button', { name: 'Resetear entorno' })
    resetButton.focus()
    await fireEvent.click(resetButton)

    const dialog = screen.getByRole('alertdialog', { name: 'Confirmar reseteo del entorno' })
    await waitFor(() => {
      expect(screen.getByLabelText('Confirmación requerida')).toHaveFocus()
    })

    await fireEvent.keyDown(dialog, { key: 'Escape' })

    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    expect(resetButton).toHaveFocus()
  })

  it('shows a non-crash uv warning when a different system uv version is detected', async () => {
    depsMocks.getUvStatus.mockResolvedValueOnce({
      uv_ready: false,
      uv_path: '/usr/bin/uv',
      uv_version: '0.10.3',
      uv_source: 'system-dev-fallback',
      uv_compatible_for_dev: true,
      venv_exists: false,
      venv_path: null,
      uv_warning:
        'Se detectó uv 0.10.3 en /usr/bin/uv, pero EntropIA Pro espera uv 0.6.14 para instalaciones administradas. En desarrollo esto explica el warning, no una caída de la app.',
      release_runtime_ready: false,
      release_runtime_state: 'fixture',
      dev_fallback_available: true,
      dev_fallback_reason:
        'Windows debug: si falta el runtime de release, EntropIA Pro puede crear un venv local usando Python/uv del sistema. Esto NO valida ni reemplaza el contrato de runtime-pack de release.',
    })

    render(DependenciasTab)

    expect(await screen.findByText(/Detectado: uv 0\.10\.3 en \/usr\/bin\/uv/i)).toBeInTheDocument()
    expect(screen.getByText(/no una caída de la app/i)).toBeInTheDocument()
    expect(screen.getByText(/Fallback dev disponible/i)).toBeInTheDocument()
    expect(screen.getByText(/NO valida ni reemplaza el contrato/i)).toBeInTheDocument()
  })

  it('keeps install disabled when fixture runtime has no usable dev fallback', async () => {
    depsMocks.checkAllDeps.mockResolvedValueOnce([
      { id: 'Python', status: { type: 'missing' }, version: null },
    ])
    depsMocks.getRuntimeStatus.mockResolvedValueOnce({
      state: 'fixture',
      packVersion: '2026.05.0',
      repairNeeded: false,
      repairAvailable: false,
      summary: 'Runtime fixture',
      blockedCapabilities: ['ocr', 'transcription', 'nlp'],
      details: [],
      guidance: [],
      bootstrapEligible: false,
      bootstrapRequired: true,
      activeOperation: null,
    })
    depsMocks.getUvStatus.mockResolvedValueOnce({
      uv_ready: false,
      uv_path: null,
      uv_version: null,
      uv_source: null,
      uv_compatible_for_dev: false,
      venv_exists: false,
      venv_path: null,
      uv_warning: null,
      release_runtime_ready: false,
      release_runtime_state: 'fixture',
      dev_fallback_available: false,
      dev_fallback_reason: 'Fallback de desarrollo no disponible: falta Python 3.11+ y también falta un uv del sistema utilizable.',
    })

    render(DependenciasTab)

    const button = await screen.findByRole('button', { name: 'Instalar todo' })
    expect(button).toBeDisabled()
    expect(
      screen.getByText(/Necesit.s runtime release hidratado\/compatible o un fallback de desarrollo disponible para esta plataforma/i),
    ).toBeInTheDocument()
  })

  it('updates from deps complete events without re-running dependency probes', async () => {
    let completeHandler:
      | ((event: { results: Array<{ id: string; status: { type: string }; version: string | null }> }) => void)
      | undefined
    depsMocks.onDepsComplete.mockImplementation(async (handler) => {
      completeHandler = handler
      return vi.fn()
    })

    render(DependenciasTab)

    await waitFor(() => {
      expect(depsMocks.checkAllDeps).toHaveBeenCalledTimes(1)
      expect(completeHandler).toBeDefined()
    })

    completeHandler?.({
      results: [
        { id: 'Python', status: { type: 'installed' }, version: '3.11.9' },
        { id: 'Fastembed', status: { type: 'missing' }, version: null },
      ],
    })

    await waitFor(() => {
      expect(depsMocks.getRuntimeStatus).toHaveBeenCalledTimes(2)
    })
    expect(depsMocks.checkAllDeps).toHaveBeenCalledTimes(1)
  })

  it('shows blocked bootstrap reason and active operation progress honestly', async () => {
    depsMocks.checkAllDeps.mockResolvedValueOnce([
      { id: 'Python', status: { type: 'installed', version: '3.11.9' }, version: '3.11.9' },
      { id: 'FasterWhisper', status: { type: 'missing' }, version: null },
    ])
    depsMocks.getRuntimeStatus.mockResolvedValueOnce({
      state: 'blocked_source_unavailable',
      packVersion: '2026.05.0',
      repairNeeded: false,
      repairAvailable: false,
      summary: 'No hay una fuente confiable disponible',
      blockedCapabilities: ['ocr', 'transcription'],
      details: ['manifest not published'],
      guidance: ['Reintentá cuando exista una fuente confiable'],
      bootstrapEligible: false,
      bootstrapRequired: true,
      activeOperation: {
        kind: 'bootstrap',
        stage: 'blocked',
        summary: 'Bootstrap bloqueado por falta de source',
        progressPercent: 45,
        downloadedBytes: 45,
        totalBytes: 100,
        retryable: true,
      },
    })

    render(DependenciasTab)

    expect(await screen.findByText(/No hay una fuente confiable disponible/i)).toBeInTheDocument()
    expect(screen.getByText(/manifest not published/i)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Reparar runtime' })).not.toBeInTheDocument()
  })

  it('does not resurrect stale bootstrap progress after a healthy runtime status', async () => {
    let statusHandler: (status: unknown) => void = vi.fn()
    let progressHandler: (operation: unknown) => void = vi.fn()
    depsMocks.onRuntimeStatus.mockImplementation(async (handler) => {
      statusHandler = handler
      return vi.fn()
    })
    depsMocks.onRuntimeProgress.mockImplementation(async (handler) => {
      progressHandler = handler
      return vi.fn()
    })
    depsMocks.getRuntimeStatus.mockResolvedValueOnce({
      state: 'damaged',
      packVersion: '2026.05.0',
      repairNeeded: true,
      repairAvailable: false,
      summary: 'Runtime dañado',
      blockedCapabilities: ['ocr'],
      details: [],
      guidance: [],
      bootstrapEligible: true,
      bootstrapRequired: true,
      activeOperation: null,
    })

    render(DependenciasTab)

    await waitFor(() => expect(progressHandler).toBeDefined())
    expect(await screen.findByText(/Runtime dañado/i)).toBeInTheDocument()
    progressHandler?.({
      kind: 'bootstrap',
      stage: 'downloading',
      summary: 'Descargando runtime remoto confiable',
      progressPercent: 35,
      downloadedBytes: null,
      totalBytes: null,
      retryable: true,
    })

    expect(await screen.findByText(/35% · Descargando runtime remoto confiable/i)).toBeInTheDocument()

    statusHandler?.({
      state: 'healthy',
      packVersion: '2026.05.0',
      repairNeeded: false,
      repairAvailable: false,
      summary: 'Runtime listo',
      blockedCapabilities: [],
      details: [],
      guidance: [],
      bootstrapEligible: false,
      bootstrapRequired: false,
      activeOperation: null,
    })
    progressHandler?.({
      kind: 'bootstrap',
      stage: 'downloading',
      summary: 'Descargando runtime remoto confiable',
      progressPercent: 35,
      downloadedBytes: null,
      totalBytes: null,
      retryable: true,
    })

    await waitFor(() => {
      expect(screen.queryByText(/35% · Descargando runtime remoto confiable/i)).not.toBeInTheDocument()
    })
  })
})
