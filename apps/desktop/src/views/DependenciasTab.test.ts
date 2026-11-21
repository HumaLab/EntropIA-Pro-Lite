import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { LOCAL_ML } from '$lib/capabilities'
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
  localSetupStatus: vi.fn(),
  localSetupInstall: vi.fn(),
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

// The pure status rules (`runtimeNeedsAttention`, `runtimeBlocksCurrentUse`,
// `shouldShowRuntimeRepairAction`) stay real, so these tests break when the
// rules change; only the Tauri calls are faked.
vi.mock('$lib/runtime', async (importOriginal) => ({
  ...(await importOriginal<typeof import('$lib/runtime')>()),
  getRuntimeStatus: depsMocks.getRuntimeStatus,
  repairRuntime: depsMocks.repairRuntime,
  onRuntimeStatus: depsMocks.onRuntimeStatus,
  onRuntimeProgress: depsMocks.onRuntimeProgress,
  runtimeCanBootstrapAutomatically: depsMocks.runtimeCanBootstrapAutomatically,
}))

vi.mock('$lib/local-setup', async (importOriginal) => ({
  ...(await importOriginal<typeof import('$lib/local-setup')>()),
  localSetupStatus: depsMocks.localSetupStatus,
  localSetupInstall: depsMocks.localSetupInstall,
}))

const READY_SETUP = {
  localMl: true,
  components: [
    {
      id: 'runtime',
      label: 'Base de IA local (runtime)',
      ready: true,
      detail: 'Runtime listo',
      enables: 'OCR local',
    },
    {
      id: 'llm',
      label: 'Modelo de texto local (Gemma)',
      ready: true,
      detail: 'Instalado: gemma.gguf (2,5 GB).',
      enables: 'Chat',
    },
  ],
}

/** A release build whose runtime pack has not been downloaded yet. */
const RELEASE_RUNTIME_PENDING_UV = {
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
  dev_fallback_reason:
    'En release no hay fallback de desarrollo: necesitás un runtime-pack hidratado/compatible o una fuente bootstrap confiable.',
}

// The tab exists only in Pro; in Lite `lib/runtime` turns its rules off on purpose.
describe.runIf(LOCAL_ML)('DependenciasTab', () => {
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
    depsMocks.localSetupStatus.mockResolvedValue(READY_SETUP)
    depsMocks.localSetupInstall.mockResolvedValue([])
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

  // The listeners are registered one `await` at a time after the initial
  // state refresh. A tab torn down meanwhile (switching Settings tabs) used
  // to push the late ones into a list its onDestroy had already walked,
  // leaving them live for the rest of the session (drop-dup fix).
  it('releases listeners that finish registering after the tab is gone', async () => {
    const pending: Array<(unlisten: () => void) => void> = []
    const deferred = () =>
      new Promise<() => void>((resolve) => {
        pending.push(resolve)
      })
    depsMocks.onDepsProgress.mockImplementation(deferred)
    depsMocks.onDepsComplete.mockImplementation(deferred)
    depsMocks.onDepsError.mockImplementation(deferred)
    depsMocks.onRuntimeStatus.mockImplementation(deferred)
    depsMocks.onRuntimeProgress.mockImplementation(deferred)
    depsMocks.listen.mockImplementation(deferred)

    const { unmount } = render(DependenciasTab)
    await waitFor(() => expect(pending.length).toBeGreaterThan(0))
    unmount()

    const unlistens: Array<ReturnType<typeof vi.fn>> = []
    while (pending.length > 0) {
      const unlisten = vi.fn()
      unlistens.push(unlisten)
      pending.shift()!(unlisten)
      await new Promise((resolve) => setTimeout(resolve, 0))
    }

    expect(unlistens.length).toBeGreaterThan(1)
    for (const unlisten of unlistens) expect(unlisten).toHaveBeenCalledOnce()
  })

  // A failed registration used to abandon the listeners that had already
  // landed: the sequential array literal never completed, so nothing called
  // their unlisten (same leak class as the destroy race above).
  it('releases the listeners already registered and shows a banner when a later registration fails', async () => {
    const unlistens: Array<ReturnType<typeof vi.fn>> = []
    depsMocks.listen.mockImplementation(async (name: string) => {
      if (name === 'embedding:download_progress') throw new Error('listen failed')
      const unlisten = vi.fn()
      unlistens.push(unlisten)
      return unlisten
    })

    render(DependenciasTab)

    await waitFor(() => {
      expect(unlistens).toHaveLength(3)
      for (const unlisten of unlistens) expect(unlisten).toHaveBeenCalledOnce()
    })
    expect(await screen.findByText(/Error al registrar listeners/)).toBeInTheDocument()
  })

  it('registers no listener when the tab is gone before the initial refresh settles', async () => {
    let resolveCheck: ((results: unknown[]) => void) | undefined
    depsMocks.checkAllDeps.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveCheck = resolve
        })
    )

    const { unmount } = render(DependenciasTab)
    await waitFor(() => expect(resolveCheck).toBeDefined())
    unmount()
    resolveCheck!([])
    await new Promise((resolve) => setTimeout(resolve, 0))

    expect(depsMocks.onDepsProgress).not.toHaveBeenCalled()
    expect(depsMocks.listen).not.toHaveBeenCalled()
  })

  it('ignores an event Tauri still delivers after the tab is gone', async () => {
    const handlers = new Map<string, () => unknown>()
    depsMocks.listen.mockImplementation(async (name: string, handler: () => unknown) => {
      handlers.set(name, handler)
      return vi.fn()
    })

    const { unmount } = render(DependenciasTab)
    await waitFor(() => expect(handlers.has('qwen-correct:download_error')).toBe(true))
    unmount()
    depsMocks.localSetupStatus.mockClear()

    await handlers.get('llm:download_complete')!()

    expect(depsMocks.localSetupStatus).not.toHaveBeenCalled()
  })

  it('lists every local model, explains the GPU-only ones and prepares only what this computer can use', async () => {
    depsMocks.localSetupStatus.mockResolvedValue({
      localMl: true,
      components: [
        ...READY_SETUP.components,
        {
          id: 'parakeet',
          label: 'Transcripción local (Parakeet)',
          ready: false,
          detail: 'Falta descargar unos 2,6 GB.',
          enables: 'Dictado',
        },
        {
          id: 'glm_ocr',
          label: 'OCRH local (GLM-OCR)',
          ready: false,
          detail: 'Faltan los modelos (unos 1,4 GB).',
          enables: 'OCRH',
          unavailableReason:
            'Necesita una GPU NVIDIA y no se detectó ninguna en este equipo, así que no se descarga.',
        },
      ],
    })
    depsMocks.localSetupInstall.mockResolvedValue([
      { id: 'parakeet', started: true, message: 'descarga iniciada' },
    ])
    render(DependenciasTab)

    expect(await screen.findByText('Transcripción local (Parakeet)')).toBeInTheDocument()
    expect(screen.getByText('Falta descargar unos 2,6 GB.')).toBeInTheDocument()
    expect(screen.getByText(/Necesita una GPU NVIDIA/)).toBeInTheDocument()
    expect(screen.getByText('No aplica en este equipo')).toBeInTheDocument()
    // Only one row can be downloaded here.
    expect(screen.getAllByRole('button', { name: 'Descargar' })).toHaveLength(1)

    await fireEvent.click(screen.getByRole('button', { name: 'Preparar EntropIA Pro' }))
    await waitFor(() => expect(depsMocks.localSetupInstall).toHaveBeenCalledWith(['parakeet']))
  })

  it('shows on the row why a download could not start', async () => {
    depsMocks.localSetupStatus.mockResolvedValue({
      localMl: true,
      components: [
        {
          id: 'reranker',
          label: 'Reranker local (BGE v2 M3)',
          ready: false,
          detail: 'Falta descargar (unos 560 MB).',
          enables: 'Chat',
        },
      ],
    })
    depsMocks.localSetupInstall.mockResolvedValue([
      { id: 'reranker', started: false, message: 'no se pudo iniciar: sin espacio en disco' },
    ])
    render(DependenciasTab)

    await fireEvent.click(await screen.findByRole('button', { name: 'Descargar' }))
    expect(await screen.findByText('no se pudo iniciar: sin espacio en disco')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Reintentar' })).toBeInTheDocument()
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
      repairAvailable: true,
      summary: 'Runtime de IA pendiente de descarga para linux-x86_64',
      blockedCapabilities: ['ocr', 'transcription', 'nlp'],
      details: [
        'La app 0.0.10 arrancó correctamente, pero el runtime de IA para linux-x86_64 todavía está pendiente de descarga (app_version declarada: 0.0.10).',
      ],
      guidance: [
        'Esto no indica una caída: el runtime de IA se descarga automáticamente cuando hay una fuente confiable; mientras tanto, OCR, NLP y transcripción quedan en pausa.',
      ],
      bootstrapEligible: true,
      bootstrapRequired: true,
      activeOperation: {
        kind: 'bootstrap',
        stage: 'planning_download',
        summary: 'EntropIA puede bootstrapear un runtime confiable',
        progressPercent: null,
        downloadedBytes: null,
        totalBytes: 1024,
        retryable: true,
      },
    })

    render(DependenciasTab)

    expect(await screen.findByText(/Runtime de IA pendiente de descarga/i)).toBeInTheDocument()
    expect(
      screen.getByText(/se descarga automáticamente cuando hay una fuente confiable/i)
    ).toBeInTheDocument()
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

  it('in a dev build, does not surface fixture runtime messaging when local deps are installed', async () => {
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
      summary: 'Runtime de IA pendiente de descarga para linux-x86_64',
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
      dev_fallback_available: true,
      dev_fallback_reason: null,
    })

    render(DependenciasTab)

    expect(
      await screen.findByText('Todas las dependencias están instaladas y listas para usar.')
    ).toBeInTheDocument()
    expect(screen.queryByText(/Runtime de IA pendiente de descarga/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/payloads externos/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/runtime-pack release pendiente/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/^Capacidades afectadas:/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/antes de habilitar OCR/i)).not.toBeInTheDocument()
  })

  it('in a release, does not claim ready while the fixture pack still needs its download (audit 6)', async () => {
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
      summary: 'Runtime de IA pendiente de descarga para linux-x86_64',
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

    // An old venv answers the dependency probe, but the runtime the app will
    // use is still the fixture pack: the block must be visible.
    expect(await screen.findByText(/Runtime de IA pendiente de descarga/i)).toBeInTheDocument()
    expect(
      screen.queryByText('Todas las dependencias están instaladas y listas para usar.')
    ).not.toBeInTheDocument()
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
      details: ['Trusted remote bootstrap source is not configured in this environment'],
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
      await screen.findByText('No hay una fuente confiable disponible para bootstrap')
    ).toBeInTheDocument()
    expect(screen.getByText(/not configured in this environment/i)).toBeInTheDocument()
    expect(screen.getByText(/Reintentá cuando exista una fuente confiable/i)).toBeInTheDocument()
    expect(
      screen.getByText(/En Windows dev el fallback online no está habilitado/i)
    ).toBeInTheDocument()
    expect(screen.getByText(/antes de habilitar OCR/i)).toBeInTheDocument()
    expect(screen.getByText(/^Capacidades afectadas:/i)).toBeInTheDocument()
  })

  it('shows a dedicated no_matching_release message when the manifest has no entry for this version', async () => {
    depsMocks.checkAllDeps.mockResolvedValueOnce([
      { id: 'Python', status: { type: 'installed', version: '3.11.9' }, version: '3.11.9' },
    ])
    depsMocks.getRuntimeStatus.mockResolvedValueOnce({
      state: 'blocked_source_unavailable',
      packVersion: '2026.05.0',
      repairNeeded: false,
      repairAvailable: false,
      summary: 'No hay una fuente confiable disponible para bootstrap',
      blockedCapabilities: ['ocr'],
      details: ['No bootstrap release matches app_version=1.0.19 platform=windows-x86_64'],
      guidance: ['Reintentá cuando cambien las condiciones o se publique una fuente válida.'],
      bootstrapEligible: false,
      bootstrapRequired: true,
      blockedReason: 'no_matching_release',
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
      dev_fallback_reason: null,
    })

    render(DependenciasTab)

    expect(
      await screen.findByText('Esta versión de EntropIA Pro todavía no tiene su runtime publicado.')
    ).toBeInTheDocument()
    expect(screen.queryByText(/aún no está publicada/)).not.toBeInTheDocument()
  })

  it('keeps the generic source-unavailable text when no blocked reason is present', async () => {
    depsMocks.checkAllDeps.mockResolvedValueOnce([
      { id: 'Python', status: { type: 'installed', version: '3.11.9' }, version: '3.11.9' },
    ])
    depsMocks.getRuntimeStatus.mockResolvedValueOnce({
      state: 'blocked_source_unavailable',
      packVersion: '2026.05.0',
      repairNeeded: false,
      repairAvailable: false,
      summary: 'No hay una fuente confiable disponible para bootstrap',
      blockedCapabilities: ['ocr'],
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
      dev_fallback_reason: null,
    })

    render(DependenciasTab)

    expect(await screen.findByText(/aún no está publicada/)).toBeInTheDocument()
    expect(
      screen.queryByText('Esta versión de EntropIA Pro todavía no tiene su runtime publicado.')
    ).not.toBeInTheDocument()
  })

  it('labels healthy embedded runtime without a managed venv honestly', async () => {
    depsMocks.getUvStatus.mockResolvedValueOnce({
      uv_ready: true,
      uv_path:
        'C:\\Users\\test\\AppData\\Roaming\\com.entropia.pro.desktop\\runtime\\2026.05.0\\uv\\uv.exe',
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

    const dialog = screen.getByRole('dialog')
    expect(dialog).toBeInTheDocument()
    expect(dialog).toHaveAttribute('aria-modal', 'true')
    expect(dialog).toHaveAccessibleName('Confirmar reseteo del entorno')
    expect(dialog).toHaveAccessibleDescription(/Esta acción borra el entorno de IA administrado/i)
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

    const dialog = screen.getByRole('dialog', { name: 'Confirmar reseteo del entorno' })
    await waitFor(() => {
      expect(screen.getByLabelText('Confirmación requerida')).toHaveFocus()
    })

    await fireEvent.keyDown(dialog, { key: 'Escape' })

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
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
      dev_fallback_reason:
        'Fallback de desarrollo no disponible: falta Python 3.11+ y también falta un uv del sistema utilizable.',
    })

    render(DependenciasTab)

    const button = await screen.findByRole('button', { name: 'Instalar todo' })
    expect(button).toBeDisabled()
    expect(screen.getByText(/Primero tiene que estar lista la base de IA/i)).toBeInTheDocument()
  })

  it('updates from deps complete events without re-running dependency probes', async () => {
    let completeHandler:
      | ((event: {
          results: Array<{ id: string; status: { type: string }; version: string | null }>
        }) => void)
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

  it('offers an actionable re-check for a source-unavailable runtime', async () => {
    depsMocks.checkAllDeps.mockResolvedValueOnce([
      { id: 'Python', status: { type: 'installed', version: '3.11.9' }, version: '3.11.9' },
      { id: 'PaddleOcr', status: { type: 'missing' }, version: null },
    ])
    depsMocks.getRuntimeStatus.mockResolvedValueOnce({
      state: 'blocked_source_unavailable',
      packVersion: '2026.05.0',
      repairNeeded: false,
      repairAvailable: false,
      summary: 'No hay una fuente confiable disponible',
      blockedCapabilities: ['ocr'],
      details: ['manifest not published'],
      guidance: ['Reintentá cuando exista una fuente confiable'],
      bootstrapEligible: false,
      bootstrapRequired: true,
      activeOperation: null,
    })

    render(DependenciasTab)

    expect(
      await screen.findByText(/Todavía no hay una fuente de descarga confiable/i)
    ).toBeInTheDocument()
    expect(screen.queryByText(/sin conexión/i)).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Reparar runtime' })).not.toBeInTheDocument()

    const initialChecks = depsMocks.getRuntimeStatus.mock.calls.length
    const recheckButton = await screen.findByRole('button', { name: 'Volver a verificar' })
    await fireEvent.click(recheckButton)

    await waitFor(() => {
      expect(depsMocks.getRuntimeStatus.mock.calls.length).toBeGreaterThan(initialChecks)
    })
  })

  it('explains offline distinctly and exposes a retry-connection action', async () => {
    depsMocks.checkAllDeps.mockResolvedValueOnce([
      { id: 'Python', status: { type: 'installed', version: '3.11.9' }, version: '3.11.9' },
      { id: 'PaddleOcr', status: { type: 'missing' }, version: null },
    ])
    depsMocks.getRuntimeStatus.mockResolvedValueOnce({
      state: 'blocked_offline',
      packVersion: '2026.05.0',
      repairNeeded: false,
      repairAvailable: false,
      summary: 'Sin conexión para descargar el runtime',
      blockedCapabilities: ['ocr', 'transcription', 'nlp'],
      details: ['No network reachable'],
      guidance: ['Reintentá cuando recuperes la conexión'],
      bootstrapEligible: false,
      bootstrapRequired: true,
      activeOperation: null,
    })

    render(DependenciasTab)

    expect(await screen.findByText(/parece que estás sin conexión/i)).toBeInTheDocument()
    expect(
      screen.queryByText(/Todavía no hay una fuente de descarga confiable/i)
    ).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Reintentar conexión' })).toBeInTheDocument()
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

    // The listeners register one await at a time after the initial refresh;
    // wait for the real registrations instead of the vi.fn() placeholders.
    await waitFor(() => {
      expect(depsMocks.onRuntimeStatus).toHaveBeenCalled()
      expect(depsMocks.onRuntimeProgress).toHaveBeenCalled()
    })
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

    expect(
      await screen.findByText(/35% · Descargando runtime remoto confiable/i)
    ).toBeInTheDocument()

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
      expect(
        screen.queryByText(/35% · Descargando runtime remoto confiable/i)
      ).not.toBeInTheDocument()
    })
  })

  // Store 1.8.2: while the runtime downloaded on its own, the card said the
  // local features "no van a funcionar" and that no dev environment replaced it.
  it('says a downloading runtime only pauses the local features', async () => {
    depsMocks.getRuntimeStatus.mockResolvedValueOnce({
      state: 'fixture',
      packVersion: '2026.11.6',
      repairNeeded: false,
      repairAvailable: false,
      summary: 'Runtime de IA pendiente de descarga para windows-x86_64',
      blockedCapabilities: ['ocr', 'transcription', 'nlp'],
      details: [],
      guidance: [],
      bootstrapEligible: true,
      bootstrapRequired: true,
      activeOperation: null,
    })
    depsMocks.getUvStatus.mockResolvedValueOnce({
      ...RELEASE_RUNTIME_PENDING_UV,
    })

    render(DependenciasTab)

    expect(await screen.findByText(/se descarga sola la primera vez/i)).toBeInTheDocument()
    expect(screen.queryByText(/no van a funcionar/i)).not.toBeInTheDocument()
    expect(screen.queryByText(/Tampoco hay un entorno de desarrollo/i)).not.toBeInTheDocument()
  })

  it('keeps the no-fallback note when the runtime cannot download on its own', async () => {
    depsMocks.getRuntimeStatus.mockResolvedValueOnce({
      state: 'fixture',
      packVersion: '2026.11.6',
      repairNeeded: false,
      repairAvailable: false,
      summary: 'Runtime fixture',
      blockedCapabilities: ['ocr'],
      details: [],
      guidance: [],
      bootstrapEligible: false,
      bootstrapRequired: true,
      activeOperation: null,
    })
    depsMocks.getUvStatus.mockResolvedValueOnce({ ...RELEASE_RUNTIME_PENDING_UV })

    render(DependenciasTab)

    expect(await screen.findByText(/Tampoco hay un entorno de desarrollo/i)).toBeInTheDocument()
  })

  // Store 1.8.2: the release no-fallback line stayed on screen after the
  // runtime turned healthy, until the tab was opened again.
  it('drops the release no-fallback line once the runtime turns healthy', async () => {
    let statusHandler: (status: unknown) => void = vi.fn()
    depsMocks.onRuntimeStatus.mockImplementation(async (handler) => {
      statusHandler = handler
      return vi.fn()
    })
    depsMocks.getRuntimeStatus.mockResolvedValueOnce({
      state: 'fixture',
      packVersion: '2026.11.6',
      repairNeeded: false,
      repairAvailable: false,
      summary: 'Runtime de IA pendiente de descarga',
      blockedCapabilities: ['ocr'],
      details: [],
      guidance: [],
      bootstrapEligible: true,
      bootstrapRequired: true,
      activeOperation: null,
    })
    depsMocks.getUvStatus.mockResolvedValueOnce({ ...RELEASE_RUNTIME_PENDING_UV })

    render(DependenciasTab)

    await waitFor(() => {
      expect(depsMocks.onRuntimeStatus).toHaveBeenCalled()
    })
    statusHandler?.({
      state: 'healthy',
      packVersion: '2026.11.6',
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

    // The healthy status has landed once the pending card is gone.
    await waitFor(() => {
      expect(screen.queryByText(/Runtime de IA pendiente de descarga/i)).not.toBeInTheDocument()
    })
    await waitFor(() => {
      expect(screen.queryByText(/no hay fallback de desarrollo/i)).not.toBeInTheDocument()
    })
    expect(depsMocks.getUvStatus).toHaveBeenCalledTimes(2)
  })

  it('shows per-dependency install progress as a labelled progress bar', async () => {
    depsMocks.checkAllDeps.mockResolvedValueOnce([
      { id: 'Spacy', status: { type: 'installing', percent: 40 }, version: null },
    ])

    render(DependenciasTab)

    const bar = await screen.findByRole('progressbar', { name: 'Instalando spaCy (NER)' })
    expect(bar).toHaveAttribute('aria-valuenow', '40')
    expect(screen.getByText('40%')).toBeInTheDocument()
  })

  it('shows overall install progress as a progress bar named by its visible label', async () => {
    depsMocks.checkAllDeps.mockResolvedValue([
      { id: 'Python', status: { type: 'installed', version: '3.11.9' }, version: '3.11.9' },
      { id: 'Spacy', status: { type: 'missing' }, version: null },
    ])
    depsMocks.installAllDeps.mockReturnValue(new Promise(() => undefined))

    render(DependenciasTab)

    await fireEvent.click(await screen.findByRole('button', { name: 'Instalar todo' }))

    const bar = await screen.findByRole('progressbar', { name: '50% instalado' })
    expect(bar).toHaveAttribute('aria-valuenow', '50')
  })

  // Audit item 17: installs and downloads keep running while the user is away.
  it('keeps Instalar todo in progress after leaving and coming back, until it ends', async () => {
    depsMocks.checkAllDeps.mockResolvedValue([
      { id: 'Python', status: { type: 'installed', version: '3.11.9' }, version: '3.11.9' },
      { id: 'Spacy', status: { type: 'missing' }, version: null },
    ])
    let finishInstall: () => void = () => {}
    depsMocks.installAllDeps.mockReturnValue(
      new Promise<void>((resolve) => {
        finishInstall = resolve
      })
    )

    const first = render(DependenciasTab)
    await fireEvent.click(await screen.findByRole('button', { name: 'Instalar todo' }))
    first.unmount()

    render(DependenciasTab)
    expect(await screen.findByRole('progressbar', { name: /instalado$/ })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Instalar todo' })).toBeNull()

    finishInstall()
    expect(await screen.findByRole('button', { name: 'Instalar todo' })).toBeInTheDocument()
    expect(depsMocks.installAllDeps).toHaveBeenCalledOnce()
  })

  it('keeps a model download in progress after leaving and coming back', async () => {
    depsMocks.localSetupStatus.mockResolvedValue({
      localMl: true,
      components: [
        {
          id: 'reranker',
          label: 'Reranker local (BGE v2 M3)',
          ready: false,
          detail: 'Falta descargar (unos 560 MB).',
          enables: 'Chat',
        },
      ],
    })
    depsMocks.localSetupInstall.mockResolvedValue([
      { id: 'reranker', started: true, message: 'descarga iniciada' },
    ])

    const first = render(DependenciasTab)
    await fireEvent.click(await screen.findByRole('button', { name: 'Descargar' }))
    await waitFor(() => expect(depsMocks.localSetupInstall).toHaveBeenCalledWith(['reranker']))
    first.unmount()

    render(DependenciasTab)
    expect(await screen.findByText('Reranker local (BGE v2 M3)')).toBeInTheDocument()
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Descargar' })).toBeNull())
  })

  it('renders the required and version labels as status badges', async () => {
    render(DependenciasTab)

    const required = await screen.findByText('Requerido')
    expect(required.closest('.status-badge')).not.toBeNull()
    expect(screen.getByText('3.11.9').closest('.status-badge')).not.toBeNull()
  })

  it('confirms the reset in a portalled dialog that keeps the typed phrase off a stray backdrop click', async () => {
    render(DependenciasTab)

    await fireEvent.click(await screen.findByRole('button', { name: 'Resetear entorno' }))

    const dialog = screen.getByRole('dialog', { name: 'Confirmar reseteo del entorno' })
    await fireEvent.input(screen.getByLabelText('Confirmación requerida'), {
      target: { value: 'resetear' },
    })
    const overlay = dialog.parentElement as HTMLElement
    await fireEvent.click(overlay)

    expect(screen.getByRole('dialog')).toBeInTheDocument()
    expect(screen.getByLabelText('Confirmación requerida')).toHaveValue('resetear')
  })
})
