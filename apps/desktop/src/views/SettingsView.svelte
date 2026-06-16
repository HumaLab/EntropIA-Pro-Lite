<script module lang="ts">
  /**
   * Serializable snapshot of every user-editable settings value. Compared
   * against the baseline captured at load/save time to detect unsaved edits.
   */
  export type SettingsSnapshotInput = {
    apiKey: string
    model: string
    embeddingModel: string
    embeddingProvider: string
    localEmbeddingModelDir: string
    llmMode: string
    sttMode: string
    ocrhMode: string
    localModelSourceUrl: string
    localModelFilename: string
    assemblyAiApiKey: string
    assemblyAiCollectionSpeakerLabels: boolean
    glmOcrApiKey: string
    selectedLocale: string
    ocrCorrectionPrompt: string
    summaryPrompt: string
    nerPrompt: string
    tripletsPrompt: string
    modelParamsByFlow: Record<string, Record<string, string>>
    ragParams: Record<string, string>
  }

  export function buildSettingsSnapshot(input: SettingsSnapshotInput): string {
    return JSON.stringify(input)
  }

  /** Dirty = a baseline exists and the current snapshot differs from it. */
  export function hasUnsavedSettingsChanges(
    savedSnapshot: string | null,
    currentSnapshot: string
  ): boolean {
    return savedSnapshot !== null && savedSnapshot !== currentSnapshot
  }
</script>

<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { get } from 'svelte/store'
  import { locale, isLocale, t, type Locale } from '$lib/i18n'
  import { navigation } from '$lib/navigation'
  import { registerEscapeInterceptor } from '$lib/keyboard'
  import { openExternalUrlFromClick } from '$lib/external-links'
  import {
    settingsGet,
    settingsGetAll,
    settingsSet,
    testOpenrouterConnection,
    testAssemblyaiConnection,
    testGlmOcrConnection,
    SETTINGS_KEYS,
    DEFAULT_OPENROUTER_MODEL,
    DEFAULT_OPENROUTER_EMBEDDING_MODEL,
    DEFAULT_LLM_MODE,
    DEFAULT_EMBEDDING_PROVIDER,
    DEFAULT_STT_MODE,
    DEFAULT_OCRH_MODE,
    DEFAULT_PROMPTS,
    DEFAULT_MODEL_PARAMS,
    DEFAULT_MODEL_PARAMS_BY_FLOW,
    DEFAULT_RAG_PARAMS,
    type EmbeddingProvider,
    type LlmMode,
    type OcrhMode,
    type SttMode,
    type ModelInfo,
  } from '$lib/settings'
  import {
    llmLocalModelInfo,
    llmOpenModelsDir,
    llmDownloadModel,
    type LocalModelInfo,
    type LlmDownloadProgressPayload,
    type LlmDownloadCompletePayload,
    type LlmDownloadErrorPayload,
  } from '$lib/llm'
  import {
    embeddingLocalModelInfo,
    embeddingOpenModelsDir,
    embeddingDownloadModel,
    type LocalEmbeddingModelInfo,
    type EmbeddingDownloadProgressPayload,
    type EmbeddingDownloadCompletePayload,
    type EmbeddingDownloadErrorPayload,
  } from '$lib/embeddings'
  import { isCriticalMissing, onCriticalMissingChange } from '$lib/deps'
  import { listen, type UnlistenFn } from '@tauri-apps/api/event'
  import { ActionIcon, Button, Card, ConfirmDialog, Input, TabButton, TabList } from '@entropia/ui'
  import DependenciasTab from './DependenciasTab.svelte'
  import LogsTab from './LogsTab.svelte'
  import SyncSettingsCard from './SyncSettingsCard.svelte'

  // Tab state — auto-open deps tab if critical deps are missing (Pro-only behaviour).
  let hasDepsWarning = $state(isCriticalMissing())
  const unsubDeps = onCriticalMissingChange((v) => {
    hasDepsWarning = v
  })
  type SettingsTab =
    | 'api'
    | 'prompts'
    | 'modelParams'
    | 'ragParams'
    | 'sync'
    | 'dependencias'
    | 'logs'
  let activeTab = $state<SettingsTab>(isCriticalMissing() ? 'dependencias' : 'api')

  // State
  let apiKey = $state('')
  let maskedApiKey = $state('')
  let showApiKey = $state(false)
  let model = $state(DEFAULT_OPENROUTER_MODEL)
  let embeddingProvider = $state<EmbeddingProvider>(DEFAULT_EMBEDDING_PROVIDER)
  let embeddingModel = $state(DEFAULT_OPENROUTER_EMBEDDING_MODEL)
  let localEmbeddingModelDir = $state('')
  let localEmbeddingModel = $state<LocalEmbeddingModelInfo | null>(null)
  let llmMode = $state<LlmMode>(DEFAULT_LLM_MODE)
  let sttMode = $state<SttMode>(DEFAULT_STT_MODE)
  let ocrhMode = $state<OcrhMode>(DEFAULT_OCRH_MODE)
  let localAvailable = $state(false)
  let localModel = $state<LocalModelInfo | null>(null)
  let selectedLocale = $state<Locale>('es')
  let languageTouched = $state(false)
  let assemblyAiApiKey = $state('')
  let maskedAssemblyAiApiKey = $state('')
  let showAssemblyAiApiKey = $state(false)
  let assemblyAiCollectionSpeakerLabels = $state(true)
  let glmOcrApiKey = $state('')
  let maskedGlmOcrApiKey = $state('')
  let showGlmOcrApiKey = $state(false)
  let ocrCorrectionPrompt = $state<string>(DEFAULT_PROMPTS.ocrCorrectionPrompt)
  let summaryPrompt = $state<string>(DEFAULT_PROMPTS.summaryPrompt)
  let nerPrompt = $state<string>(DEFAULT_PROMPTS.nerPrompt)
  let tripletsPrompt = $state<string>(DEFAULT_PROMPTS.tripletsPrompt)
  type PromptKey = keyof typeof DEFAULT_PROMPTS
  type ValidationFeedback = { tone: 'success' | 'error'; text: string } | null
  let promptValidationFeedback = $state<Record<PromptKey, ValidationFeedback>>({
    ocrCorrectionPrompt: null,
    summaryPrompt: null,
    nerPrompt: null,
    tripletsPrompt: null,
  })
  type ModelParamFlow = 'ocrCorrection' | 'summary' | 'ner' | 'triplets'
  type EditableModelParams = {
    temperature: string
    maxTokens: string
    topP: string
    topK: string
    presencePenalty: string
    frequencyPenalty: string
    stopSequences: string
  }
  const MODEL_PARAM_FLOWS: Array<{ id: ModelParamFlow; label: string }> = [
    { id: 'ocrCorrection', label: 'OCR correction' },
    { id: 'summary', label: 'Summary' },
    { id: 'ner', label: 'NER' },
    { id: 'triplets', label: 'Triplets' },
  ]
  const MODEL_PARAM_KEYS: Record<ModelParamFlow, Record<keyof EditableModelParams, string>> = {
    ocrCorrection: {
      temperature: SETTINGS_KEYS.LLM_OCR_CORRECTION_TEMPERATURE,
      maxTokens: SETTINGS_KEYS.LLM_OCR_CORRECTION_MAX_TOKENS,
      topP: SETTINGS_KEYS.LLM_OCR_CORRECTION_TOP_P,
      topK: SETTINGS_KEYS.LLM_OCR_CORRECTION_TOP_K,
      presencePenalty: SETTINGS_KEYS.LLM_OCR_CORRECTION_PRESENCE_PENALTY,
      frequencyPenalty: SETTINGS_KEYS.LLM_OCR_CORRECTION_FREQUENCY_PENALTY,
      stopSequences: SETTINGS_KEYS.LLM_OCR_CORRECTION_STOP_SEQUENCES,
    },
    summary: {
      temperature: SETTINGS_KEYS.LLM_SUMMARY_TEMPERATURE,
      maxTokens: SETTINGS_KEYS.LLM_SUMMARY_MAX_TOKENS,
      topP: SETTINGS_KEYS.LLM_SUMMARY_TOP_P,
      topK: SETTINGS_KEYS.LLM_SUMMARY_TOP_K,
      presencePenalty: SETTINGS_KEYS.LLM_SUMMARY_PRESENCE_PENALTY,
      frequencyPenalty: SETTINGS_KEYS.LLM_SUMMARY_FREQUENCY_PENALTY,
      stopSequences: SETTINGS_KEYS.LLM_SUMMARY_STOP_SEQUENCES,
    },
    ner: {
      temperature: SETTINGS_KEYS.LLM_NER_TEMPERATURE,
      maxTokens: SETTINGS_KEYS.LLM_NER_MAX_TOKENS,
      topP: SETTINGS_KEYS.LLM_NER_TOP_P,
      topK: SETTINGS_KEYS.LLM_NER_TOP_K,
      presencePenalty: SETTINGS_KEYS.LLM_NER_PRESENCE_PENALTY,
      frequencyPenalty: SETTINGS_KEYS.LLM_NER_FREQUENCY_PENALTY,
      stopSequences: SETTINGS_KEYS.LLM_NER_STOP_SEQUENCES,
    },
    triplets: {
      temperature: SETTINGS_KEYS.LLM_TRIPLETS_TEMPERATURE,
      maxTokens: SETTINGS_KEYS.LLM_TRIPLETS_MAX_TOKENS,
      topP: SETTINGS_KEYS.LLM_TRIPLETS_TOP_P,
      topK: SETTINGS_KEYS.LLM_TRIPLETS_TOP_K,
      presencePenalty: SETTINGS_KEYS.LLM_TRIPLETS_PRESENCE_PENALTY,
      frequencyPenalty: SETTINGS_KEYS.LLM_TRIPLETS_FREQUENCY_PENALTY,
      stopSequences: SETTINGS_KEYS.LLM_TRIPLETS_STOP_SEQUENCES,
    },
  }
  let modelParamsByFlow = $state<Record<ModelParamFlow, EditableModelParams>>({
    ocrCorrection: { ...DEFAULT_MODEL_PARAMS_BY_FLOW.ocrCorrection },
    summary: { ...DEFAULT_MODEL_PARAMS_BY_FLOW.summary },
    ner: { ...DEFAULT_MODEL_PARAMS_BY_FLOW.ner },
    triplets: { ...DEFAULT_MODEL_PARAMS_BY_FLOW.triplets },
  })
  let modelParamsError = $state<string | null>(null)

  type EditableRagParams = {
    topK: string
    minSimilarity: string
    candidatesPerLeg: string
    rrfK: string
    snippetMaxChars: string
    contextMaxChars: string
    historyTurns: string
    historyTurnMaxChars: string
    temperature: string
    maxTokens: string
  }
  const RAG_PARAM_KEYS: Record<keyof EditableRagParams, string> = {
    topK: SETTINGS_KEYS.RAG_TOP_K,
    minSimilarity: SETTINGS_KEYS.RAG_MIN_SIMILARITY,
    candidatesPerLeg: SETTINGS_KEYS.RAG_CANDIDATES_PER_LEG,
    rrfK: SETTINGS_KEYS.RAG_RRF_K,
    snippetMaxChars: SETTINGS_KEYS.RAG_SNIPPET_MAX_CHARS,
    contextMaxChars: SETTINGS_KEYS.RAG_CONTEXT_MAX_CHARS,
    historyTurns: SETTINGS_KEYS.RAG_HISTORY_TURNS,
    historyTurnMaxChars: SETTINGS_KEYS.RAG_HISTORY_TURN_MAX_CHARS,
    temperature: SETTINGS_KEYS.RAG_TEMPERATURE,
    maxTokens: SETTINGS_KEYS.RAG_MAX_TOKENS,
  }
  let ragParams = $state<EditableRagParams>({ ...DEFAULT_RAG_PARAMS })
  let ragParamsError = $state<string | null>(null)

  // Test connection state
  let testing = $state(false)
  let testResult = $state<{ success: boolean; message: string } | null>(null)
  let testingAssemblyAi = $state(false)
  let assemblyAiTestResult = $state<{ success: boolean; message: string } | null>(null)
  let testingGlmOcr = $state(false)
  let glmOcrTestResult = $state<{ success: boolean; message: string } | null>(null)
  let availableModels = $state<ModelInfo[]>([])
  let loadSettingsError = $state<string | null>(null)

  const hasOpenRouterCredential = $derived(Boolean(apiKey.trim() || maskedApiKey))
  const hasAssemblyAiCredential = $derived(Boolean(assemblyAiApiKey.trim() || maskedAssemblyAiApiKey))
  const hasGlmOcrCredential = $derived(Boolean(glmOcrApiKey.trim() || maskedGlmOcrApiKey))

  const SECRET_REF_PREFIX = 'secret_ref:'
  const LANGUAGE_KEY = SETTINGS_KEYS.LANGUAGE
  const LEGACY_LOCAL_EMBEDDING_MODEL_DIR = 'resources/models/embeddings/bge-m3'
  const PROVIDER_LINKS = {
    openrouter: 'https://openrouter.ai/settings/keys',
    assemblyai: 'https://www.assemblyai.com/app/account',
    glmOcr: 'https://z.ai/manage-apikey/apikey-list',
  } as const

  // Local model download state
  let downloading = $state(false)
  let downloadPct = $state(0)
  let downloadError = $state<string | null>(null)
  let localModelSourceUrl = $state('')
  let localModelFilename = $state('')
  let downloadUnlisteners: UnlistenFn[] = []
  let embeddingDownloading = $state(false)
  let embeddingDownloadPct = $state(0)
  let embeddingDownloadFile = $state('')
  let embeddingDownloadError = $state<string | null>(null)

  // Save state
  let saving = $state(false)
  let saveFeedback = $state<{ tone: 'success' | 'error'; text: string } | null>(null)

  // Unsaved-changes guard: baseline snapshot captured after load/save.
  let savedSnapshot = $state<string | null>(null)
  let showDiscardConfirm = $state(false)

  let currentModeLabel = $derived(
    llmMode === 'local'
      ? t('settings.llmMode.local.label')
      : llmMode === 'openrouter'
        ? t('settings.llmMode.openrouter.label')
        : t('settings.llmMode.auto.label')
  )

  let currentModeDescription = $derived(
    llmMode === 'local'
      ? t('settings.llmMode.local.summary')
      : llmMode === 'openrouter'
        ? t('settings.llmMode.openrouter.summary')
        : t('settings.llmMode.auto.summary')
  )

  let currentSttModeDescription = $derived(
    sttMode === 'local'
      ? t('settings.sttMode.local.summary')
      : sttMode === 'assemblyai'
        ? t('settings.sttMode.assemblyai.summary')
        : t('settings.sttMode.auto.summary')
  )

  let currentOcrhModeDescription = $derived(
    ocrhMode === 'local'
      ? t('settings.ocrhMode.local.summary')
      : ocrhMode === 'glm_ocr'
        ? t('settings.ocrhMode.glm_ocr.summary')
        : t('settings.ocrhMode.auto.summary')
  )

  const currentSnapshot = $derived(
    buildSettingsSnapshot({
      apiKey,
      model,
      embeddingModel,
      embeddingProvider,
      localEmbeddingModelDir,
      llmMode,
      sttMode,
      ocrhMode,
      localModelSourceUrl,
      localModelFilename,
      assemblyAiApiKey,
      assemblyAiCollectionSpeakerLabels,
      glmOcrApiKey,
      selectedLocale,
      ocrCorrectionPrompt,
      summaryPrompt,
      nerPrompt,
      tripletsPrompt,
      modelParamsByFlow,
      ragParams,
    })
  )
  const isDirty = $derived(hasUnsavedSettingsChanges(savedSnapshot, currentSnapshot))

  const activeLocale = $derived($locale)

  onDestroy(() => {
    unsubDeps()
    downloadUnlisteners.forEach((fn) => fn())
    downloadUnlisteners = []
  })

  onMount(() => {
    void loadInitialSettings()
    void registerDownloadListeners()
    // Escape must not silently discard unsaved edits: when dirty, ask for
    // confirmation instead of navigating back.
    return registerEscapeInterceptor(() => {
      if (!isDirty) return false
      showDiscardConfirm = true
      return true
    })
  })

  function handleDiscardConfirm() {
    showDiscardConfirm = false
    navigation.back()
  }

  async function loadInitialSettings() {
    loadSettingsError = null

    try {
      const [
        storedKey,
        storedModel,
        storedEmbeddingProvider,
        storedEmbeddingModel,
        storedLocalEmbeddingModelDir,
        storedMode,
        storedSttMode,
        storedOcrhMode,
        storedAssemblyAiKey,
        storedAssemblyAiSpeakerLabels,
        storedGlmOcrKey,
        storedLanguage,
        storedOcrCorrectionPrompt,
        storedSummaryPrompt,
        storedNerPrompt,
        storedTripletsPrompt,
        modelInfo,
        embeddingModelInfo,
      ] = await Promise.all([
        settingsGet(SETTINGS_KEYS.OPENROUTER_API_KEY),
        settingsGet(SETTINGS_KEYS.OPENROUTER_MODEL),
        settingsGet(SETTINGS_KEYS.EMBEDDING_PROVIDER),
        settingsGet(SETTINGS_KEYS.OPENROUTER_EMBEDDING_MODEL),
        settingsGet(SETTINGS_KEYS.LOCAL_EMBEDDING_MODEL_DIR),
        settingsGet(SETTINGS_KEYS.LLM_MODE),
        settingsGet(SETTINGS_KEYS.STT_MODE),
        settingsGet(SETTINGS_KEYS.OCRH_MODE),
        settingsGet(SETTINGS_KEYS.ASSEMBLYAI_API_KEY),
        settingsGet(SETTINGS_KEYS.ASSEMBLYAI_SPEAKER_LABELS),
        settingsGet(SETTINGS_KEYS.GLM_OCR_API_KEY),
        settingsGet(LANGUAGE_KEY),
        settingsGet(SETTINGS_KEYS.OCR_CORRECTION_PROMPT),
        settingsGet(SETTINGS_KEYS.SUMMARY_PROMPT),
        settingsGet(SETTINGS_KEYS.NER_PROMPT),
        settingsGet(SETTINGS_KEYS.TRIPLETS_PROMPT),
        llmLocalModelInfo().catch(() => null),
        embeddingLocalModelInfo().catch(() => null),
      ])
      const settingsMap = new Map((await settingsGetAll()).map((entry) => [entry.key, entry.value]))

      if (storedKey?.startsWith(SECRET_REF_PREFIX)) {
        apiKey = ''
        maskedApiKey = t('settings.keyStoredInCredentialManager')
      } else if (storedKey) {
        apiKey = storedKey
        maskedApiKey = maskKey(storedKey)
      }
      if (storedModel) model = storedModel
      if (storedEmbeddingProvider === 'api' || storedEmbeddingProvider === 'local') {
        embeddingProvider = storedEmbeddingProvider
      }
      if (storedEmbeddingModel) embeddingModel = storedEmbeddingModel
      if (
        storedLocalEmbeddingModelDir &&
        !isLegacyLocalEmbeddingModelDir(storedLocalEmbeddingModelDir)
      ) {
        localEmbeddingModelDir = storedLocalEmbeddingModelDir
      }
      if (storedMode) llmMode = storedMode as LlmMode
      if (storedSttMode) sttMode = storedSttMode as SttMode
      if (storedOcrhMode) ocrhMode = storedOcrhMode as OcrhMode
      if (storedAssemblyAiKey?.startsWith(SECRET_REF_PREFIX)) {
        assemblyAiApiKey = ''
        maskedAssemblyAiApiKey = t('settings.keyStoredInCredentialManager')
      } else if (storedAssemblyAiKey) {
        assemblyAiApiKey = storedAssemblyAiKey
        maskedAssemblyAiApiKey = maskKey(storedAssemblyAiKey, 5)
      }
      assemblyAiCollectionSpeakerLabels = parseEnabledByDefault(storedAssemblyAiSpeakerLabels)
      if (storedGlmOcrKey?.startsWith(SECRET_REF_PREFIX)) {
        glmOcrApiKey = ''
        maskedGlmOcrApiKey = t('settings.keyStoredInCredentialManager')
      } else if (storedGlmOcrKey) {
        glmOcrApiKey = storedGlmOcrKey
        maskedGlmOcrApiKey = maskKey(storedGlmOcrKey, 0)
      }
      if (!languageTouched) {
        selectedLocale = isLocale(storedLanguage) ? storedLanguage : get(locale)
      }
      localModel = modelInfo
      localAvailable = modelInfo?.available ?? false
      localModelSourceUrl = modelInfo?.source_url ?? ''
      localModelFilename = modelInfo?.filename ?? ''
      localEmbeddingModel = embeddingModelInfo
      ocrCorrectionPrompt = storedOcrCorrectionPrompt?.trim() || DEFAULT_PROMPTS.ocrCorrectionPrompt
      summaryPrompt = storedSummaryPrompt?.trim() || DEFAULT_PROMPTS.summaryPrompt
      nerPrompt = storedNerPrompt?.trim() || DEFAULT_PROMPTS.nerPrompt
      tripletsPrompt = storedTripletsPrompt?.trim() || DEFAULT_PROMPTS.tripletsPrompt
      for (const flow of MODEL_PARAM_FLOWS) {
        modelParamsByFlow[flow.id] = readModelParamsFromSettings(settingsMap, flow.id)
      }
      ragParams = readRagParamsFromSettings(settingsMap)
      savedSnapshot = currentSnapshot
    } catch (e) {
      loadSettingsError = e instanceof Error ? e.message : String(e)
    }
  }

  async function registerDownloadListeners() {
    // Listen to local model download events (Pro-only local inference wiring).
    downloadUnlisteners.push(
      await listen<LlmDownloadProgressPayload>('llm:download_progress', (event) => {
        downloading = true
        downloadPct = event.payload.pct
        downloadError = null
      }),
      await listen<LlmDownloadCompletePayload>('llm:download_complete', async () => {
        downloading = false
        downloadPct = 100
        downloadError = null
        localModel = await llmLocalModelInfo().catch(() => null)
        localAvailable = localModel?.available ?? false
      }),
      await listen<LlmDownloadErrorPayload>('llm:download_error', (event) => {
        downloading = false
        downloadPct = 0
        downloadError = event.payload.error
      }),
      await listen<EmbeddingDownloadProgressPayload>('embedding:download_progress', (event) => {
        embeddingDownloading = true
        embeddingDownloadPct = event.payload.pct
        embeddingDownloadFile = event.payload.file
        embeddingDownloadError = null
      }),
      await listen<EmbeddingDownloadCompletePayload>('embedding:download_complete', async () => {
        embeddingDownloading = false
        embeddingDownloadPct = 100
        embeddingDownloadFile = ''
        embeddingDownloadError = null
        localEmbeddingModel = await embeddingLocalModelInfo().catch(() => null)
      }),
      await listen<EmbeddingDownloadErrorPayload>('embedding:download_error', (event) => {
        embeddingDownloading = false
        embeddingDownloadPct = 0
        embeddingDownloadFile = ''
        embeddingDownloadError = event.payload.error
      })
    )
  }

  function maskKey(key: string, prefixLength = 4): string {
    const trimmed = key.trim()
    if (!trimmed) return ''
    if (trimmed.length <= prefixLength + 4) return '*'.repeat(trimmed.length)
    return `${trimmed.slice(0, prefixLength)}****...****${trimmed.slice(-4)}`
  }

  function parseEnabledByDefault(value: string | null): boolean {
    const normalized = value?.trim().toLowerCase()
    if (!normalized) return true
    return !['0', 'false', 'no', 'off'].includes(normalized)
  }

  // Los params numéricos viajan como TEXTO a Rust (str::parse): solo se acepta
  // lo que Rust puede parsear — enteros planos y decimales planos (sin '12.0'
  // para enteros, sin notación '1e3' ni '0x10'). Number() de JS es más laxo.
  const INTEGER_TEXT_PATTERN = /^[+-]?\d+$/
  const DECIMAL_TEXT_PATTERN = /^[+-]?(\d+(\.\d+)?|\.\d+)$/

  function validNumberText(value: string | null, min: number, max: number): string | null {
    const trimmed = value?.trim()
    if (!trimmed || !DECIMAL_TEXT_PATTERN.test(trimmed)) return null
    const parsed = Number(trimmed)
    return Number.isFinite(parsed) && parsed >= min && parsed <= max ? trimmed : null
  }

  function validIntegerText(value: string | null, min: number, max: number): string | null {
    const trimmed = value?.trim()
    if (!trimmed || !INTEGER_TEXT_PATTERN.test(trimmed)) return null
    const parsed = Number(trimmed)
    return Number.isInteger(parsed) && parsed >= min && parsed <= max ? trimmed : null
  }

  /** Canonicaliza texto numérico ya validado ('007' → '7', '.5' → '0.5'); vacío queda vacío. */
  function normalizedNumericText(value: string): string {
    const trimmed = value.trim()
    return trimmed ? String(Number(trimmed)) : ''
  }

  function readModelParamsFromSettings(
    settingsMap: Map<string, string>,
    flow: ModelParamFlow
  ): EditableModelParams {
    const keys = MODEL_PARAM_KEYS[flow]
    return {
      temperature:
        validNumberText(settingsMap.get(keys.temperature) ?? null, 0, 2) ??
        DEFAULT_MODEL_PARAMS.temperature,
      maxTokens:
        validIntegerText(settingsMap.get(keys.maxTokens) ?? null, 1, 32000) ??
        DEFAULT_MODEL_PARAMS.maxTokens,
      topP: validNumberText(settingsMap.get(keys.topP) ?? null, 0, 1) ?? DEFAULT_MODEL_PARAMS.topP,
      topK: validIntegerText(settingsMap.get(keys.topK) ?? null, 1, 1000) ?? DEFAULT_MODEL_PARAMS.topK,
      presencePenalty:
        validNumberText(settingsMap.get(keys.presencePenalty) ?? null, -2, 2) ??
        DEFAULT_MODEL_PARAMS.presencePenalty,
      frequencyPenalty:
        validNumberText(settingsMap.get(keys.frequencyPenalty) ?? null, -2, 2) ??
        DEFAULT_MODEL_PARAMS.frequencyPenalty,
      stopSequences: settingsMap.get(keys.stopSequences) ?? DEFAULT_MODEL_PARAMS.stopSequences,
    }
  }

  function validateModelParams(): string | null {
    for (const flow of MODEL_PARAM_FLOWS) {
      const params = modelParamsByFlow[flow.id]
      const checks: Array<[string, string, (value: string) => boolean]> = [
        ['temperature', params.temperature, (value) => !value.trim() || validNumberText(value, 0, 2) !== null],
        ['maxTokens', params.maxTokens, (value) => !value.trim() || validIntegerText(value, 1, 32000) !== null],
        ['topP', params.topP, (value) => !value.trim() || validNumberText(value, 0, 1) !== null],
        ['topK', params.topK, (value) => !value.trim() || validIntegerText(value, 1, 1000) !== null],
        ['presencePenalty', params.presencePenalty, (value) => !value.trim() || validNumberText(value, -2, 2) !== null],
        ['frequencyPenalty', params.frequencyPenalty, (value) => !value.trim() || validNumberText(value, -2, 2) !== null],
      ]
      const invalid = checks.find(([_, value, isValid]) => !isValid(value))
      if (invalid) return t('settings.modelParams.invalidParam', { flow: flow.label, param: invalid[0] })
    }
    return null
  }

  function readRagParamsFromSettings(settingsMap: Map<string, string>): EditableRagParams {
    const keys = RAG_PARAM_KEYS
    return {
      topK: validIntegerText(settingsMap.get(keys.topK) ?? null, 1, 20) ?? DEFAULT_RAG_PARAMS.topK,
      minSimilarity:
        validNumberText(settingsMap.get(keys.minSimilarity) ?? null, 0, 1) ?? DEFAULT_RAG_PARAMS.minSimilarity,
      candidatesPerLeg:
        validIntegerText(settingsMap.get(keys.candidatesPerLeg) ?? null, 4, 200) ??
        DEFAULT_RAG_PARAMS.candidatesPerLeg,
      rrfK: validIntegerText(settingsMap.get(keys.rrfK) ?? null, 1, 500) ?? DEFAULT_RAG_PARAMS.rrfK,
      snippetMaxChars:
        validIntegerText(settingsMap.get(keys.snippetMaxChars) ?? null, 200, 8000) ??
        DEFAULT_RAG_PARAMS.snippetMaxChars,
      contextMaxChars:
        validIntegerText(settingsMap.get(keys.contextMaxChars) ?? null, 1000, 60000) ??
        DEFAULT_RAG_PARAMS.contextMaxChars,
      historyTurns:
        validIntegerText(settingsMap.get(keys.historyTurns) ?? null, 0, 20) ?? DEFAULT_RAG_PARAMS.historyTurns,
      historyTurnMaxChars:
        validIntegerText(settingsMap.get(keys.historyTurnMaxChars) ?? null, 100, 4000) ??
        DEFAULT_RAG_PARAMS.historyTurnMaxChars,
      temperature:
        validNumberText(settingsMap.get(keys.temperature) ?? null, 0, 2) ?? DEFAULT_RAG_PARAMS.temperature,
      maxTokens:
        validIntegerText(settingsMap.get(keys.maxTokens) ?? null, 64, 32000) ?? DEFAULT_RAG_PARAMS.maxTokens,
    }
  }

  /** Valor RAG efectivo para chequeos cross-field: texto editado o default si quedó vacío. */
  function effectiveRagNumber(param: keyof EditableRagParams): number {
    return Number(ragParams[param].trim() || DEFAULT_RAG_PARAMS[param])
  }

  function validateRagParams(): string | null {
    const checks: Array<[keyof EditableRagParams, (value: string) => boolean]> = [
      ['topK', (value) => !value.trim() || validIntegerText(value, 1, 20) !== null],
      ['minSimilarity', (value) => !value.trim() || validNumberText(value, 0, 1) !== null],
      ['candidatesPerLeg', (value) => !value.trim() || validIntegerText(value, 4, 200) !== null],
      ['rrfK', (value) => !value.trim() || validIntegerText(value, 1, 500) !== null],
      ['snippetMaxChars', (value) => !value.trim() || validIntegerText(value, 200, 8000) !== null],
      ['contextMaxChars', (value) => !value.trim() || validIntegerText(value, 1000, 60000) !== null],
      ['historyTurns', (value) => !value.trim() || validIntegerText(value, 0, 20) !== null],
      ['historyTurnMaxChars', (value) => !value.trim() || validIntegerText(value, 100, 4000) !== null],
      ['temperature', (value) => !value.trim() || validNumberText(value, 0, 2) !== null],
      ['maxTokens', (value) => !value.trim() || validIntegerText(value, 64, 32000) !== null],
    ]
    const invalid = checks.find(([param, isValid]) => !isValid(ragParams[param]))
    if (invalid) return t('settings.ragParams.invalidParam', { param: invalid[0] })
    // Invariante cross-field (espejo del clamp del backend): el snippet no
    // puede superar el presupuesto total de contexto.
    if (effectiveRagNumber('snippetMaxChars') > effectiveRagNumber('contextMaxChars')) {
      return t('settings.ragParams.snippetVsContext')
    }
    return null
  }

  function promptValue(key: PromptKey): string {
    if (key === 'ocrCorrectionPrompt') return ocrCorrectionPrompt
    if (key === 'summaryPrompt') return summaryPrompt
    if (key === 'nerPrompt') return nerPrompt
    return tripletsPrompt
  }

  function validatePromptContract(key: PromptKey, value = promptValue(key)): string | null {
    const prompt = value.trim()
    if (!prompt) return t('settings.promptValidation.empty')
    if ((key === 'ocrCorrectionPrompt' || key === 'summaryPrompt') && !prompt.includes('{text}')) {
      return t('settings.promptValidation.missingText')
    }
    if (key === 'nerPrompt') {
      if (!prompt.includes('{text}')) return t('settings.promptValidation.nerMissingText')
      const requiredLabels = ['PER', 'LOC', 'ORG', 'DATE', 'MISC']
      const missing = requiredLabels.filter((label) => !prompt.includes(label))
      if (missing.length > 0) return t('settings.promptValidation.nerMissingLabels', { labels: missing.join(', ') })
    }
    if (key === 'tripletsPrompt') {
      if (!prompt.includes('{text}')) return t('settings.promptValidation.tripletsMissingText')
      const requiredKeys = ['subject', 'predicate', 'object']
      const missing = requiredKeys.filter((label) => !prompt.includes(label))
      if (missing.length > 0) return t('settings.promptValidation.tripletsMissingKeys', { keys: missing.join(', ') })
    }
    return null
  }

  function validatePrompt(key: PromptKey): boolean {
    const error = validatePromptContract(key)
    promptValidationFeedback[key] = error
      ? { tone: 'error', text: error }
      : { tone: 'success', text: t('settings.promptValidation.valid') }
    return !error
  }

  function validateAllPrompts(): string | null {
    const keys: PromptKey[] = ['ocrCorrectionPrompt', 'summaryPrompt', 'nerPrompt', 'tripletsPrompt']
    for (const key of keys) {
      const error = validatePromptContract(key)
      if (error) {
        promptValidationFeedback[key] = { tone: 'error', text: error }
        return `${promptLabel(key)}: ${error}`
      }
      promptValidationFeedback[key] = { tone: 'success', text: t('settings.promptValidation.valid') }
    }
    return null
  }

  function promptLabel(key: PromptKey): string {
    if (key === 'ocrCorrectionPrompt') return 'OCR correction prompt'
    if (key === 'summaryPrompt') return 'Summary prompt'
    if (key === 'nerPrompt') return 'NER prompt'
    return 'Triplets prompt'
  }

  async function handleTestConnection() {
    if (!hasOpenRouterCredential) {
      testResult = { success: false, message: t('settings.enterApiKey') }
      return
    }
    testing = true
    testResult = null
    try {
      const models = await testOpenrouterConnection(apiKey.trim())
      availableModels = models
      testResult = {
        success: true,
        message: t('settings.connectionReady', { count: models.length }),
      }
    } catch (e) {
      testResult = {
        success: false,
        message: e instanceof Error ? e.message : String(e),
      }
    } finally {
      testing = false
    }
  }

  async function handleTestAssemblyAiConnection() {
    if (!hasAssemblyAiCredential) {
      assemblyAiTestResult = { success: false, message: t('settings.enterAssemblyAiApiKey') }
      return
    }

    testingAssemblyAi = true
    assemblyAiTestResult = null
    try {
      await testAssemblyaiConnection(assemblyAiApiKey.trim())
      assemblyAiTestResult = {
        success: true,
        message: t('settings.assemblyAiConnectionReady'),
      }
    } catch (e) {
      assemblyAiTestResult = {
        success: false,
        message: e instanceof Error ? e.message : String(e),
      }
    } finally {
      testingAssemblyAi = false
    }
  }

  async function handleTestGlmOcrConnection() {
    if (!hasGlmOcrCredential) {
      glmOcrTestResult = { success: false, message: t('settings.enterGlmOcrApiKey') }
      return
    }

    testingGlmOcr = true
    glmOcrTestResult = null
    try {
      await testGlmOcrConnection(glmOcrApiKey.trim())
      glmOcrTestResult = {
        success: true,
        message: t('settings.glmOcrConnectionReady'),
      }
    } catch (e) {
      glmOcrTestResult = {
        success: false,
        message: e instanceof Error ? e.message : String(e),
      }
    } finally {
      testingGlmOcr = false
    }
  }

  async function handleSave() {
    saving = true
    saveFeedback = null
    const promptError = validateAllPrompts()
    if (promptError) {
      saving = false
      saveFeedback = { tone: 'error', text: promptError }
      activeTab = 'prompts'
      return
    }
    modelParamsError = validateModelParams()
    if (modelParamsError) {
      saving = false
      saveFeedback = { tone: 'error', text: modelParamsError }
      activeTab = 'modelParams'
      return
    }
    ragParamsError = validateRagParams()
    if (ragParamsError) {
      saving = false
      saveFeedback = { tone: 'error', text: ragParamsError }
      activeTab = 'ragParams'
      return
    }
    try {
      const writes: Promise<void>[] = [
        settingsSet(SETTINGS_KEYS.OPENROUTER_MODEL, model),
        // Pro is local-first: persist the user-selected modes, not hardcoded
        // cloud ones. The selectors carry the real local/remote/auto choice.
        settingsSet(SETTINGS_KEYS.EMBEDDING_PROVIDER, embeddingProvider),
        settingsSet(
          SETTINGS_KEYS.OPENROUTER_EMBEDDING_MODEL,
          embeddingModel.trim() || DEFAULT_OPENROUTER_EMBEDDING_MODEL
        ),
        settingsSet(SETTINGS_KEYS.LOCAL_EMBEDDING_MODEL_DIR, localEmbeddingModelDir.trim()),
        settingsSet(SETTINGS_KEYS.LLM_MODE, llmMode),
        settingsSet(SETTINGS_KEYS.STT_MODE, sttMode),
        settingsSet(
          SETTINGS_KEYS.ASSEMBLYAI_SPEAKER_LABELS,
          assemblyAiCollectionSpeakerLabels ? 'true' : 'false'
        ),
        settingsSet(SETTINGS_KEYS.OCRH_MODE, ocrhMode),
        settingsSet(LANGUAGE_KEY, selectedLocale),
        settingsSet(SETTINGS_KEYS.LOCAL_MODEL_SOURCE_URL, localModelSourceUrl.trim()),
        settingsSet(
          SETTINGS_KEYS.LOCAL_MODEL_FILENAME,
          (localModelFilename.trim() || localModel?.filename) ?? ''
        ),
        settingsSet(SETTINGS_KEYS.OCR_CORRECTION_PROMPT, ocrCorrectionPrompt.trim() || DEFAULT_PROMPTS.ocrCorrectionPrompt),
        settingsSet(SETTINGS_KEYS.SUMMARY_PROMPT, summaryPrompt.trim() || DEFAULT_PROMPTS.summaryPrompt),
        settingsSet(SETTINGS_KEYS.NER_PROMPT, nerPrompt.trim() || DEFAULT_PROMPTS.nerPrompt),
        settingsSet(SETTINGS_KEYS.TRIPLETS_PROMPT, tripletsPrompt.trim() || DEFAULT_PROMPTS.tripletsPrompt),
      ]
      for (const flow of MODEL_PARAM_FLOWS) {
        const params = modelParamsByFlow[flow.id]
        const keys = MODEL_PARAM_KEYS[flow.id]
        writes.push(
          settingsSet(keys.temperature, normalizedNumericText(params.temperature) || DEFAULT_MODEL_PARAMS.temperature),
          settingsSet(keys.maxTokens, normalizedNumericText(params.maxTokens)),
          settingsSet(keys.topP, normalizedNumericText(params.topP)),
          settingsSet(keys.topK, normalizedNumericText(params.topK)),
          settingsSet(keys.presencePenalty, normalizedNumericText(params.presencePenalty)),
          settingsSet(keys.frequencyPenalty, normalizedNumericText(params.frequencyPenalty)),
          settingsSet(keys.stopSequences, params.stopSequences)
        )
      }
      for (const param of Object.keys(RAG_PARAM_KEYS) as Array<keyof EditableRagParams>) {
        // Se persiste el valor canónico ('0.20' → '0.2') para que el texto
        // guardado coincida con lo que Rust parsea y la UI relee.
        writes.push(
          settingsSet(
            RAG_PARAM_KEYS[param],
            normalizedNumericText(ragParams[param]) || DEFAULT_RAG_PARAMS[param]
          )
        )
      }
      if (apiKey.trim()) writes.push(settingsSet(SETTINGS_KEYS.OPENROUTER_API_KEY, apiKey.trim()))
      if (assemblyAiApiKey.trim()) writes.push(settingsSet(SETTINGS_KEYS.ASSEMBLYAI_API_KEY, assemblyAiApiKey.trim()))
      if (glmOcrApiKey.trim()) writes.push(settingsSet(SETTINGS_KEYS.GLM_OCR_API_KEY, glmOcrApiKey.trim()))
      await Promise.all(writes)
      if (apiKey.trim()) maskedApiKey = maskKey(apiKey)
      if (assemblyAiApiKey.trim()) maskedAssemblyAiApiKey = maskKey(assemblyAiApiKey, 5)
      if (glmOcrApiKey.trim()) maskedGlmOcrApiKey = maskKey(glmOcrApiKey, 0)
      savedSnapshot = currentSnapshot
      saveFeedback = {
        tone: 'success',
        text: t('settings.saved'),
      }
      setTimeout(() => {
        saveFeedback = null
      }, 3000)
    } catch (e) {
      saveFeedback = {
        tone: 'error',
        text: `Error: ${e instanceof Error ? e.message : String(e)}`,
      }
    } finally {
      saving = false
    }
  }

  function handleModelSelect(modelId: string) {
    model = modelId
  }

  function formatBytes(bytes: number | null): string {
    if (bytes == null) return '—'
    if (bytes === 0) return '0 B'
    const k = 1024
    const sizes = ['B', 'KB', 'MB', 'GB', 'TB']
    const i = Math.floor(Math.log(bytes) / Math.log(k))
    return `${parseFloat((bytes / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`
  }

  function handleLanguageChange(event: Event) {
    const nextLocale = (event.target as HTMLSelectElement).value as Locale
    languageTouched = true
    selectedLocale = nextLocale
    locale.set(nextLocale)
  }

  async function handleDownloadModel() {
    if (downloading) return
    const sourceUrl = localModelSourceUrl.trim() || localModel?.source_url || ''
    const filename = localModelFilename.trim() || localModel?.filename || ''
    if (!sourceUrl) {
      downloadError = t('settings.localModel.sourceUrlRequired')
      return
    }
    downloading = true
    downloadPct = 0
    downloadError = null
    try {
      await Promise.all([
        settingsSet(SETTINGS_KEYS.LOCAL_MODEL_SOURCE_URL, sourceUrl),
        settingsSet(SETTINGS_KEYS.LOCAL_MODEL_FILENAME, filename),
      ])
      await llmDownloadModel()
    } catch (e) {
      downloading = false
      downloadError = e instanceof Error ? e.message : String(e)
    }
  }

  function isLegacyLocalEmbeddingModelDir(value: string): boolean {
    const normalized = value.trim().replaceAll('\\', '/').replace(/^\.\//, '').toLowerCase()
    return (
      normalized === LEGACY_LOCAL_EMBEDDING_MODEL_DIR ||
      normalized.endsWith(`/${LEGACY_LOCAL_EMBEDDING_MODEL_DIR}`)
    )
  }

  async function handleDownloadEmbeddingModel() {
    if (embeddingDownloading) return
    embeddingDownloading = true
    embeddingDownloadPct = 0
    embeddingDownloadError = null
    try {
      await Promise.all([
        settingsSet(SETTINGS_KEYS.EMBEDDING_PROVIDER, 'local'),
        settingsSet(
          SETTINGS_KEYS.OPENROUTER_EMBEDDING_MODEL,
          embeddingModel.trim() || DEFAULT_OPENROUTER_EMBEDDING_MODEL
        ),
        settingsSet(SETTINGS_KEYS.LOCAL_EMBEDDING_MODEL_DIR, localEmbeddingModelDir.trim()),
      ])
      embeddingProvider = 'local'
      await embeddingDownloadModel()
    } catch (e) {
      embeddingDownloading = false
      embeddingDownloadError = e instanceof Error ? e.message : String(e)
    }
  }

  async function resetPrompt(key: keyof typeof DEFAULT_PROMPTS) {
    const value = DEFAULT_PROMPTS[key]
    if (key === 'ocrCorrectionPrompt') ocrCorrectionPrompt = value
    if (key === 'summaryPrompt') summaryPrompt = value
    if (key === 'nerPrompt') nerPrompt = value
    if (key === 'tripletsPrompt') tripletsPrompt = value
    promptValidationFeedback[key] = null
  }

  function resetModelParams(flow: ModelParamFlow) {
    modelParamsByFlow[flow] = { ...DEFAULT_MODEL_PARAMS_BY_FLOW[flow] }
    modelParamsError = null
  }

  function resetRagParams() {
    ragParams = { ...DEFAULT_RAG_PARAMS }
    ragParamsError = null
  }

  async function openProviderLink(event: MouseEvent, url: string, providerName: string) {
    try {
      await openExternalUrlFromClick(event, url)
    } catch (error) {
      console.error(`[Settings] No se pudo abrir el enlace de ${providerName}`, error)
    }
  }
</script>

{#key activeLocale}
  <div class="settings-view page-shell" data-locale={activeLocale}>
    <section class="page-header settings-view__header">
      <div class="page-header__content">
        <span class="page-header__eyebrow">{t('settings.preferences')}</span>
        <h1>{t('settings.title')}</h1>
        <p>{t('settings.subtitle')}</p>
        <span class="page-header__meta">{t('settings.currentMode', { mode: currentModeLabel })}</span>
      </div>

      <div class="page-toolbar settings-view__toolbar">
        <Button variant="primary" onclick={handleSave} disabled={saving}>
          {saving ? t('settings.saving') : t('settings.save')}
        </Button>
      </div>
    </section>

    <TabList aria-label={t('settings.tabsAria')}>
      <TabButton active={activeTab === 'api'} onclick={() => (activeTab = 'api')}>
        {t('settings.remoteApisTab')}
      </TabButton>
      <TabButton active={activeTab === 'prompts'} onclick={() => (activeTab = 'prompts')}>
        {t('settings.promptsTab')}
      </TabButton>
      <TabButton active={activeTab === 'modelParams'} onclick={() => (activeTab = 'modelParams')}>
        {t('settings.modelParamsTab')}
      </TabButton>
      <TabButton active={activeTab === 'ragParams'} onclick={() => (activeTab = 'ragParams')}>
        {t('settings.ragParamsTab')}
      </TabButton>
      <TabButton active={activeTab === 'sync'} onclick={() => (activeTab = 'sync')}>
        {t('settings.syncTab')}
      </TabButton>
      <TabButton active={activeTab === 'dependencias'} onclick={() => (activeTab = 'dependencias')}>
        {t('settings.dependenciesTab')}{#if hasDepsWarning}<span class="settings-tab__badge"></span>{/if}
      </TabButton>
      <TabButton active={activeTab === 'logs'} onclick={() => (activeTab = 'logs')}>
        {t('settings.logsTab')}
      </TabButton>
    </TabList>

    {#if activeTab === 'api'}
    {#if saveFeedback}
      <p
        class="surface-message"
        class:surface-message--error={saveFeedback.tone === 'error'}
        class:surface-message--success={saveFeedback.tone === 'success'}
      >
        {saveFeedback.text}
      </p>
    {/if}

    {#if loadSettingsError}
      <div class="surface-message surface-message--error settings__load-error" role="alert">
        <span>{t('settings.loadError', { error: loadSettingsError })}</span>
        <Button variant="secondary" size="sm" onclick={loadInitialSettings}>
          {t('settings.retryLoad')}
        </Button>
      </div>
    {/if}

    <Card>
      <section class="settings-card-section">
        <div class="settings-card-section__copy">
          <h2>{t('settings.languageTitle')}</h2>
          <p>{t('settings.languageDescription')}</p>
        </div>

        <div class="settings__field settings__field--stacked">
          <label class="settings__label" for="language-select">{t('settings.languageLabel')}</label>
          <select
            id="language-select"
            class="settings__input settings__input--select"
            bind:value={selectedLocale}
            onchange={handleLanguageChange}
          >
            <option value="es">{t('settings.languageOptionEs')}</option>
            <option value="en">{t('settings.languageOptionEn')}</option>
          </select>
        </div>
      </section>
    </Card>

    <Card>
      <section class="settings-card-section">
        <div class="settings-card-section__copy">
          <h2>{t('settings.llmModeTitle')}</h2>
          <p>{currentModeDescription}</p>
        </div>

        <div class="settings__mode-options">
          <label class="settings__radio" class:active={llmMode === 'local'}>
            <input type="radio" name="llm_mode" value="local" bind:group={llmMode} />
            <div class="settings__radio-content">
              <strong>{t('settings.llmMode.local.label')}</strong>
              <span class="settings__radio-desc">
                {t('settings.llmMode.local.description')}
                {#if localModel?.exists}
                  <span class="settings__badge settings__badge--ok">{t('settings.badge.available')}</span>
                {:else if localModel?.can_auto_download || localAvailable}
                  <span class="settings__badge settings__badge--warn">{t('settings.badge.downloadable')}</span>
                {:else}
                  <span class="settings__badge settings__badge--warn">{t('settings.badge.notFound')}</span>
                {/if}
              </span>
            </div>
          </label>

          <label class="settings__radio" class:active={llmMode === 'openrouter'}>
            <input type="radio" name="llm_mode" value="openrouter" bind:group={llmMode} />
            <div class="settings__radio-content">
              <strong>{t('settings.llmMode.openrouter.label')}</strong>
              <span class="settings__radio-desc">{t('settings.llmMode.openrouter.description')}</span>
            </div>
          </label>

          <label class="settings__radio" class:active={llmMode === 'auto'}>
            <input type="radio" name="llm_mode" value="auto" bind:group={llmMode} />
            <div class="settings__radio-content">
              <strong>{t('settings.llmMode.auto.label')}</strong>
              <span class="settings__radio-desc">{t('settings.llmMode.auto.description')}</span>
            </div>
          </label>
        </div>
      </section>
    </Card>

    <Card>
      <section class="settings-card-section">
        <div class="settings-card-section__copy">
          <h2>{t('settings.localModel.title')}</h2>
          <p>{t('settings.localModel.description')}</p>
        </div>

        {#if localModel}
          <div class="settings__local-model">
            <div class="settings__local-model-row">
              <span class="settings__label">{t('settings.localModel.status')}</span>
              {#if localModel.exists}
                <span class="settings__badge settings__badge--ok">{t('settings.localModel.found')}</span>
                <span class="settings__local-model-size">{formatBytes(localModel.size_bytes)}</span>
              {:else if localModel.can_auto_download}
                <span class="settings__badge settings__badge--warn">{t('settings.localModel.downloadable')}</span>
              {:else}
                <span class="settings__badge settings__badge--warn">{t('settings.localModel.missing')}</span>
              {/if}
            </div>

            <div class="settings__local-model-row">
              <span class="settings__label">{t('settings.localModel.path')}</span>
              <code class="settings__local-model-path">{localModel.path}</code>
            </div>

            {#if !localModel.exists}
              <p class="settings__local-model-guide">
                {t('settings.localModel.guide')}
                <code>{localModel.filename}</code>
              </p>

              <div class="settings__field settings__field--stacked">
                <label class="settings__label" for="local-model-filename">{t('settings.localModel.filename')}</label>
                <input
                  id="local-model-filename"
                  type="text"
                  class="settings__input"
                  bind:value={localModelFilename}
                  placeholder={localModel?.filename ?? ''}
                />
              </div>

              <div class="settings__field settings__field--stacked">
                <label class="settings__label" for="local-model-source">{t('settings.localModel.sourceUrl')}</label>
                <input
                  id="local-model-source"
                  type="text"
                  class="settings__input"
                  bind:value={localModelSourceUrl}
                  placeholder="https://…"
                />
              </div>

              {#if downloading}
                <div class="settings__download-progress">
                  <span class="settings__download-progress-bar" style="width: {downloadPct}%"></span>
                  <span class="settings__download-progress-text">{downloadPct}% — {t('settings.localModel.downloading')}</span>
                </div>
              {:else}
                <Button
                  variant="primary"
                  size="sm"
                  onclick={handleDownloadModel}
                  disabled={!localModelSourceUrl.trim()}
                >
                  {t('settings.localModel.download')}
                </Button>
              {/if}

              {#if downloadError}
                <p class="surface-message surface-message--error">{downloadError}</p>
              {/if}
            {/if}

            <Button variant="secondary" size="sm" onclick={() => llmOpenModelsDir()}>
              {t('settings.localModel.openFolder')}
            </Button>
          </div>
        {:else}
          <p class="settings__hint">{t('settings.localModel.loading')}</p>
        {/if}
      </section>
    </Card>

    <Card>
      <section class="settings-card-section">
        <div class="settings-card-section__copy">
          <h2>{t('settings.embeddingProvider.title')}</h2>
          <p>{t('settings.embeddingProvider.description')}</p>
        </div>

        <div class="settings__mode-options">
          <label class="settings__radio" class:active={embeddingProvider === 'api'}>
            <input type="radio" name="embedding_provider" value="api" bind:group={embeddingProvider} />
            <div class="settings__radio-content">
              <strong>{t('settings.embeddingProvider.api.label')}</strong>
              <span class="settings__radio-desc">{t('settings.embeddingProvider.api.description')}</span>
            </div>
          </label>

          <label class="settings__radio" class:active={embeddingProvider === 'local'}>
            <input type="radio" name="embedding_provider" value="local" bind:group={embeddingProvider} />
            <div class="settings__radio-content">
              <strong>{t('settings.embeddingProvider.local.label')}</strong>
              <span class="settings__radio-desc">{t('settings.embeddingProvider.local.description')}</span>
            </div>
          </label>
        </div>

        {#if embeddingProvider === 'local'}
          <div class="settings__field settings__field--stacked">
            <label class="settings__label" for="local-embedding-model-dir">
              {t('settings.embeddingProvider.localPath')}
            </label>
            <input
              id="local-embedding-model-dir"
              type="text"
              class="settings__input"
              bind:value={localEmbeddingModelDir}
              placeholder={t('settings.embeddingProvider.localPathPlaceholder')}
            />
            <p class="settings__hint">{t('settings.embeddingProvider.localPathHint')}</p>
          </div>

          {#if localEmbeddingModel}
            <div class="settings__local-model">
              <div class="settings__local-model-row">
                <span class="settings__label">{t('settings.embeddingProvider.localStatus')}</span>
                {#if localEmbeddingModel.available}
                  <span class="settings__badge settings__badge--ok">{t('settings.embeddingProvider.localComplete')}</span>
                {:else}
                  <span class="settings__badge settings__badge--warn">{t('settings.embeddingProvider.localIncomplete')}</span>
                {/if}
              </div>

              <div class="settings__local-model-row">
                <span class="settings__label">{t('settings.embeddingProvider.localPath')}</span>
                <code class="settings__local-model-path">{localEmbeddingModel.directory}</code>
              </div>

              <p class="settings__hint">
                {t('settings.embeddingProvider.localInstallHint', { repo: localEmbeddingModel.source_repo })}
              </p>

              {#if localEmbeddingModel.missing_files.length > 0}
                <ul class="settings__hint">
                  {#each localEmbeddingModel.missing_files as file (file.filename)}
                    <li><code>{file.filename}</code> ← {file.source_path} ({formatBytes(file.size_bytes)})</li>
                  {/each}
                </ul>
              {/if}

              {#if embeddingDownloading}
                <div class="settings__download-progress">
                  <span class="settings__download-progress-bar" style="width: {embeddingDownloadPct}%"></span>
                  <span class="settings__download-progress-text">
                    {embeddingDownloadPct}% — {embeddingDownloadFile || t('settings.embeddingProvider.downloading')}
                  </span>
                </div>
              {:else if !localEmbeddingModel.available}
                <Button variant="primary" size="sm" onclick={handleDownloadEmbeddingModel}>
                  {t('settings.embeddingProvider.installLocal')}
                </Button>
              {/if}

              {#if embeddingDownloadError}
                <p class="surface-message surface-message--error">{embeddingDownloadError}</p>
              {/if}

              <Button variant="secondary" size="sm" onclick={() => embeddingOpenModelsDir()}>
                {t('settings.embeddingProvider.openLocalFolder')}
              </Button>
            </div>
          {/if}
        {:else}
          <p class="settings__hint settings__hint--privacy">
            {t('settings.embeddingProvider.apiPrivacyNotice')}
          </p>
        {/if}
      </section>
    </Card>

    <Card>
      <section class="settings-card-section">
        <div class="settings-card-section__copy">
          <h2>{t('settings.openrouter.title')}</h2>
          <p>{t('settings.openrouter.description')}</p>
          <a
            class="settings__provider-link"
            href={PROVIDER_LINKS.openrouter}
            onclick={(event) => openProviderLink(event, PROVIDER_LINKS.openrouter, 'OpenRouter')}
          >
            <span>{t('settings.getApiKeyLink', { provider: 'OpenRouter' })}</span>
            <ActionIcon name="external-link" size={14} />
          </a>
        </div>

        <div class="settings__field settings__field--stacked">
          <label class="settings__label" for="api-key">{t('settings.apiKey')}</label>
          <div class="settings__input-row">
            {#if showApiKey}
              <input
                id="api-key"
                type="text"
                class="settings__input"
                bind:value={apiKey}
                placeholder={t('settings.apiKeyPlaceholder')}
              />
            {:else}
              <input
                id="api-key"
                type="password"
                class="settings__input"
                bind:value={apiKey}
                placeholder={t('settings.apiKeyPlaceholder')}
              />
            {/if}
            <button
              class="settings__icon-btn"
              type="button"
              onclick={() => (showApiKey = !showApiKey)}
              title={showApiKey ? t('settings.hideApiKey') : t('settings.showApiKey')}
              aria-label={showApiKey ? t('settings.hideApiKey') : t('settings.showApiKey')}
            >
              <ActionIcon name={showApiKey ? 'eye-off' : 'eye'} size={15} />
            </button>
            <Button
              variant="secondary"
              size="sm"
              onclick={handleTestConnection}
              disabled={testing || !hasOpenRouterCredential}
            >
              {testing ? t('settings.testingConnection') : t('settings.testConnection')}
            </Button>
          </div>

          {#if maskedApiKey}
            <p class="settings__hint">{t('settings.loadedKey', { key: maskedApiKey })}</p>
          {/if}

          {#if testResult}
            <p
              class="surface-message settings__feedback"
              class:surface-message--success={testResult.success}
              class:surface-message--error={!testResult.success}
            >
              {testResult.message}
            </p>
          {/if}
        </div>

        <div class="settings__field settings__field--stacked">
          <Input
            label={t('settings.model')}
            type="text"
            bind:value={model}
            placeholder={t('settings.modelPlaceholder')}
          />

          {#if availableModels.length > 0}
            <div class="settings__model-list">
              <p class="settings__model-list-title">{t('settings.suggestedModels')}</p>
              {#each availableModels
                .filter((m) => m.id.includes('gemma') || m.id.includes('llama') || m.id.includes('mistral') || m.id.includes('qwen') || m.id.includes('claude') || m.id.includes('gpt'))
                .slice(0, 15) as m (m.id)}
                <button
                  class="settings__model-option"
                  type="button"
                  class:selected={model === m.id}
                  onclick={() => handleModelSelect(m.id)}
                >
                  <span class="settings__model-id">{m.id}</span>
                  <span class="settings__model-ctx">{Math.round(m.context_length / 1024)}k ctx</span>
                </button>
              {/each}
            </div>
          {/if}
        </div>

        <div class="settings__field settings__field--stacked">
          <Input
            label={t('settings.embeddingProvider.model')}
            type="text"
            bind:value={embeddingModel}
            placeholder={DEFAULT_OPENROUTER_EMBEDDING_MODEL}
          />
          <p class="settings__hint">{t('settings.embeddingProvider.modelHint')}</p>
        </div>
      </section>
    </Card>

    <Card>
      <section class="settings-card-section">
        <div class="settings-card-section__copy">
          <h2>{t('settings.sttModeTitle')}</h2>
          <p>{currentSttModeDescription}</p>
        </div>

        <div class="settings__mode-options">
          <label class="settings__radio" class:active={sttMode === 'local'}>
            <input type="radio" name="stt_mode" value="local" bind:group={sttMode} />
            <div class="settings__radio-content">
              <strong>{t('settings.sttMode.local.label')}</strong>
              <span class="settings__radio-desc">{t('settings.sttMode.local.description')}</span>
            </div>
          </label>

          <label class="settings__radio" class:active={sttMode === 'assemblyai'}>
            <input type="radio" name="stt_mode" value="assemblyai" bind:group={sttMode} />
            <div class="settings__radio-content">
              <strong>{t('settings.sttMode.assemblyai.label')}</strong>
              <span class="settings__radio-desc">{t('settings.sttMode.assemblyai.description')}</span>
            </div>
          </label>

          <label class="settings__radio" class:active={sttMode === 'auto'}>
            <input type="radio" name="stt_mode" value="auto" bind:group={sttMode} />
            <div class="settings__radio-content">
              <strong>{t('settings.sttMode.auto.label')}</strong>
              <span class="settings__radio-desc">{t('settings.sttMode.auto.description')}</span>
            </div>
          </label>
        </div>

        {#if sttMode !== 'local'}
          <p class="settings__hint settings__hint--privacy">{t('settings.sttPrivacyNotice')}</p>
        {/if}
      </section>
    </Card>

    <Card>
      <section class="settings-card-section">
        <div class="settings-card-section__copy">
          <h2>{t('settings.assemblyai.title')}</h2>
          <p>{t('settings.assemblyai.description')}</p>
          <a
            class="settings__provider-link"
            href={PROVIDER_LINKS.assemblyai}
            onclick={(event) => openProviderLink(event, PROVIDER_LINKS.assemblyai, 'AssemblyAI')}
          >
            <span>{t('settings.getApiKeyLink', { provider: 'AssemblyAI' })}</span>
            <ActionIcon name="external-link" size={14} />
          </a>
        </div>

        <div class="settings__field settings__field--stacked">
          <label class="settings__label" for="assemblyai-api-key">{t('settings.apiKey')}</label>
          <div class="settings__input-row">
            <input
              id="assemblyai-api-key"
              type={showAssemblyAiApiKey ? 'text' : 'password'}
              class="settings__input"
              bind:value={assemblyAiApiKey}
              placeholder={t('settings.assemblyAiApiKeyPlaceholder')}
            />
            <button
              class="settings__icon-btn"
              type="button"
              onclick={() => (showAssemblyAiApiKey = !showAssemblyAiApiKey)}
              title={showAssemblyAiApiKey ? t('settings.hideApiKey') : t('settings.showApiKey')}
              aria-label={showAssemblyAiApiKey ? t('settings.hideApiKey') : t('settings.showApiKey')}
            >
              <ActionIcon name={showAssemblyAiApiKey ? 'eye-off' : 'eye'} size={15} />
            </button>
            <Button
              variant="secondary"
              size="sm"
              onclick={handleTestAssemblyAiConnection}
              disabled={testingAssemblyAi || !hasAssemblyAiCredential}
            >
              {testingAssemblyAi ? t('settings.testingConnection') : t('settings.testConnection')}
            </Button>
          </div>

          {#if maskedAssemblyAiApiKey}
            <p class="settings__hint">{t('settings.loadedKey', { key: maskedAssemblyAiApiKey })}</p>
          {/if}

          {#if assemblyAiTestResult}
            <p
              class="surface-message settings__feedback"
              class:surface-message--success={assemblyAiTestResult.success}
              class:surface-message--error={!assemblyAiTestResult.success}
            >
              {assemblyAiTestResult.message}
            </p>
          {/if}
        </div>

        <div class="settings__field settings__field--stacked">
          <label class="settings__label" for="assemblyai-speaker-labels">
            {t('settings.assemblyAiSpeakerLabels')}
          </label>
          <select
            id="assemblyai-speaker-labels"
            class="settings__input settings__input--select"
            bind:value={assemblyAiCollectionSpeakerLabels}
          >
            <option value={true}>{t('settings.optionEnabled')}</option>
            <option value={false}>{t('settings.optionDisabled')}</option>
          </select>
          <p class="settings__hint">{t('settings.assemblyAiSpeakerLabelsHint')}</p>
        </div>
      </section>
    </Card>

    <Card>
      <section class="settings-card-section">
        <div class="settings-card-section__copy">
          <h2>{t('settings.ocrhModeTitle')}</h2>
          <p>{currentOcrhModeDescription}</p>
        </div>

        <div class="settings__mode-options">
          <label class="settings__radio" class:active={ocrhMode === 'local'}>
            <input type="radio" name="ocrh_mode" value="local" bind:group={ocrhMode} />
            <div class="settings__radio-content">
              <strong>{t('settings.ocrhMode.local.label')}</strong>
              <span class="settings__radio-desc">{t('settings.ocrhMode.local.description')}</span>
            </div>
          </label>

          <label class="settings__radio" class:active={ocrhMode === 'glm_ocr'}>
            <input type="radio" name="ocrh_mode" value="glm_ocr" bind:group={ocrhMode} />
            <div class="settings__radio-content">
              <strong>{t('settings.ocrhMode.glm_ocr.label')}</strong>
              <span class="settings__radio-desc">{t('settings.ocrhMode.glm_ocr.description')}</span>
            </div>
          </label>

          <label class="settings__radio" class:active={ocrhMode === 'auto'}>
            <input type="radio" name="ocrh_mode" value="auto" bind:group={ocrhMode} />
            <div class="settings__radio-content">
              <strong>{t('settings.ocrhMode.auto.label')}</strong>
              <span class="settings__radio-desc">{t('settings.ocrhMode.auto.description')}</span>
            </div>
          </label>
        </div>

        {#if ocrhMode !== 'local'}
          <p class="settings__hint settings__hint--privacy">{t('settings.ocrhPrivacyNotice')}</p>
        {/if}
      </section>
    </Card>

    <Card>
      <section class="settings-card-section">
        <div class="settings-card-section__copy">
          <h2>{t('settings.glmOcr.title')}</h2>
          <p>{t('settings.glmOcr.description')}</p>
          <a
            class="settings__provider-link"
            href={PROVIDER_LINKS.glmOcr}
            onclick={(event) => openProviderLink(event, PROVIDER_LINKS.glmOcr, 'Z.ai')}
          >
            <span>{t('settings.getApiKeyLink', { provider: 'Z.ai' })}</span>
            <ActionIcon name="external-link" size={14} />
          </a>
        </div>

        <div class="settings__field settings__field--stacked">
          <label class="settings__label" for="glm-ocr-api-key">{t('settings.apiKey')}</label>
          <div class="settings__input-row">
            <input
              id="glm-ocr-api-key"
              type={showGlmOcrApiKey ? 'text' : 'password'}
              class="settings__input"
              bind:value={glmOcrApiKey}
              placeholder={t('settings.glmOcrApiKeyPlaceholder')}
            />
            <button
              class="settings__icon-btn"
              type="button"
              onclick={() => (showGlmOcrApiKey = !showGlmOcrApiKey)}
              title={showGlmOcrApiKey ? t('settings.hideApiKey') : t('settings.showApiKey')}
              aria-label={showGlmOcrApiKey ? t('settings.hideApiKey') : t('settings.showApiKey')}
            >
              <ActionIcon name={showGlmOcrApiKey ? 'eye-off' : 'eye'} size={15} />
            </button>
            <Button
              variant="secondary"
              size="sm"
              onclick={handleTestGlmOcrConnection}
              disabled={testingGlmOcr || !hasGlmOcrCredential}
            >
              {testingGlmOcr ? t('settings.testingConnection') : t('settings.testConnection')}
            </Button>
          </div>

          {#if maskedGlmOcrApiKey}
            <p class="settings__hint">{t('settings.loadedKey', { key: maskedGlmOcrApiKey })}</p>
          {/if}

          {#if glmOcrTestResult}
            <p
              class="surface-message settings__feedback"
              class:surface-message--success={glmOcrTestResult.success}
              class:surface-message--error={!glmOcrTestResult.success}
            >
              {glmOcrTestResult.message}
            </p>
          {/if}
        </div>
      </section>
    </Card>

    {:else if activeTab === 'prompts'}
      {#if saveFeedback}
        <p
          class="surface-message"
          class:surface-message--error={saveFeedback.tone === 'error'}
          class:surface-message--success={saveFeedback.tone === 'success'}
        >
          {saveFeedback.text}
        </p>
      {/if}

      <Card>
        <section class="settings-card-section settings-card-section--vertical">
          <div class="settings-card-section__copy">
            <h2>{t('settings.prompts.title')}</h2>
            <p>{t('settings.prompts.descriptionLead')} <code>{'{text}'}</code> {t('settings.prompts.descriptionTrail')}</p>
          </div>

          <div class="settings__prompt-grid">
            <div class="settings__field settings__field--stacked settings__prompt-card">
              <label class="settings__label" for="ocr-correction-prompt">OCR correction prompt</label>
              <textarea id="ocr-correction-prompt" class="settings__textarea" rows="12" bind:value={ocrCorrectionPrompt}></textarea>
              {#if promptValidationFeedback.ocrCorrectionPrompt}
                <p class="settings__validation" class:settings__validation--error={promptValidationFeedback.ocrCorrectionPrompt.tone === 'error'}>{promptValidationFeedback.ocrCorrectionPrompt.text}</p>
              {/if}
              <div class="settings__button-row">
                <Button variant="secondary" size="sm" onclick={() => validatePrompt('ocrCorrectionPrompt')}>{t('settings.prompts.validate')}</Button>
                <Button variant="secondary" size="sm" onclick={() => resetPrompt('ocrCorrectionPrompt')}>{t('settings.prompts.restoreDefault')}</Button>
              </div>
            </div>
            <div class="settings__field settings__field--stacked settings__prompt-card">
              <label class="settings__label" for="summary-prompt">Summary prompt</label>
              <textarea id="summary-prompt" class="settings__textarea" rows="10" bind:value={summaryPrompt}></textarea>
              {#if promptValidationFeedback.summaryPrompt}
                <p class="settings__validation" class:settings__validation--error={promptValidationFeedback.summaryPrompt.tone === 'error'}>{promptValidationFeedback.summaryPrompt.text}</p>
              {/if}
              <div class="settings__button-row">
                <Button variant="secondary" size="sm" onclick={() => validatePrompt('summaryPrompt')}>{t('settings.prompts.validate')}</Button>
                <Button variant="secondary" size="sm" onclick={() => resetPrompt('summaryPrompt')}>{t('settings.prompts.restoreDefault')}</Button>
              </div>
            </div>
            <div class="settings__field settings__field--stacked settings__prompt-card">
              <label class="settings__label" for="ner-prompt">NER prompt</label>
              <textarea id="ner-prompt" class="settings__textarea" rows="8" bind:value={nerPrompt}></textarea>
              {#if promptValidationFeedback.nerPrompt}
                <p class="settings__validation" class:settings__validation--error={promptValidationFeedback.nerPrompt.tone === 'error'}>{promptValidationFeedback.nerPrompt.text}</p>
              {/if}
              <div class="settings__button-row">
                <Button variant="secondary" size="sm" onclick={() => validatePrompt('nerPrompt')}>{t('settings.prompts.validate')}</Button>
                <Button variant="secondary" size="sm" onclick={() => resetPrompt('nerPrompt')}>{t('settings.prompts.restoreDefault')}</Button>
              </div>
            </div>
            <div class="settings__field settings__field--stacked settings__prompt-card">
              <label class="settings__label" for="triplets-prompt">Triplets prompt</label>
              <textarea id="triplets-prompt" class="settings__textarea" rows="10" bind:value={tripletsPrompt}></textarea>
              {#if promptValidationFeedback.tripletsPrompt}
                <p class="settings__validation" class:settings__validation--error={promptValidationFeedback.tripletsPrompt.tone === 'error'}>{promptValidationFeedback.tripletsPrompt.text}</p>
              {/if}
              <div class="settings__button-row">
                <Button variant="secondary" size="sm" onclick={() => validatePrompt('tripletsPrompt')}>{t('settings.prompts.validate')}</Button>
                <Button variant="secondary" size="sm" onclick={() => resetPrompt('tripletsPrompt')}>{t('settings.prompts.restoreDefault')}</Button>
              </div>
            </div>
          </div>
        </section>
      </Card>

    {:else if activeTab === 'modelParams'}
      {#if saveFeedback}
        <p
          class="surface-message"
          class:surface-message--error={saveFeedback.tone === 'error'}
          class:surface-message--success={saveFeedback.tone === 'success'}
        >
          {saveFeedback.text}
        </p>
      {/if}

      <Card>
        <section class="settings-card-section settings-card-section--vertical">
          <div class="settings-card-section__copy">
            <h2>{t('settings.modelParams.title')}</h2>
            <p><strong>{t('settings.modelParams.advancedLabel')}</strong> {t('settings.modelParams.description')}</p>
          </div>

          {#if modelParamsError}
            <p class="surface-message surface-message--error">{modelParamsError}</p>
          {/if}

          <div class="settings__params-grid settings__params-grid--flows">
            {#each MODEL_PARAM_FLOWS as flow (flow.id)}
              <div class="settings__field settings__field--stacked settings__param-card">
                <h3>{flow.label}</h3>
                <div class="settings__param-card-grid">
                  <Input label="temperature (0-2)" type="text" bind:value={modelParamsByFlow[flow.id].temperature} />
                  <Input label="maxTokens (1-32000, vacío = default)" type="text" bind:value={modelParamsByFlow[flow.id].maxTokens} />
                  <Input label="topP (0-1, opcional)" type="text" bind:value={modelParamsByFlow[flow.id].topP} />
                  <Input label="topK (1-1000, opcional)" type="text" bind:value={modelParamsByFlow[flow.id].topK} />
                  <Input label="presencePenalty (-2 a 2)" type="text" bind:value={modelParamsByFlow[flow.id].presencePenalty} />
                  <Input label="frequencyPenalty (-2 a 2)" type="text" bind:value={modelParamsByFlow[flow.id].frequencyPenalty} />
                  <div class="settings__field settings__field--stacked settings__field--wide">
                    <label class="settings__label" for={`${flow.id}-stop-sequences`}>stopSequences</label>
                    <textarea id={`${flow.id}-stop-sequences`} class="settings__textarea" rows="3" bind:value={modelParamsByFlow[flow.id].stopSequences} placeholder={t('settings.modelParams.stopSequencesPlaceholder')}></textarea>
                  </div>
                </div>
                <Button variant="secondary" size="sm" onclick={() => resetModelParams(flow.id)}>{t('settings.modelParams.restoreDefaults')}</Button>
              </div>
            {/each}
          </div>
        </section>
      </Card>

    {:else if activeTab === 'ragParams'}
      {#if saveFeedback}
        <p
          class="surface-message"
          class:surface-message--error={saveFeedback.tone === 'error'}
          class:surface-message--success={saveFeedback.tone === 'success'}
        >
          {saveFeedback.text}
        </p>
      {/if}

      <Card>
        <section class="settings-card-section settings-card-section--vertical">
          <div class="settings-card-section__copy">
            <h2>{t('settings.ragParams.title')}</h2>
            <p>{t('settings.ragParams.description')}</p>
          </div>

          {#if ragParamsError}
            <p class="surface-message surface-message--error">{ragParamsError}</p>
          {/if}

          <div class="settings__params-grid">
            <div class="settings__field settings__field--stacked settings__param-card">
              <div class="settings__param-card-grid">
                <Input label="topK (1-20)" type="text" bind:value={ragParams.topK} />
                <Input label="minSimilarity (0-1, 0 = off)" type="text" bind:value={ragParams.minSimilarity} />
                <Input label="candidatesPerLeg (4-200)" type="text" bind:value={ragParams.candidatesPerLeg} />
                <Input label="rrfK (1-500)" type="text" bind:value={ragParams.rrfK} />
                <Input label="snippetMaxChars (200-8000)" type="text" bind:value={ragParams.snippetMaxChars} />
                <Input label="contextMaxChars (1000-60000)" type="text" bind:value={ragParams.contextMaxChars} />
                <Input label="historyTurns (0-20)" type="text" bind:value={ragParams.historyTurns} />
                <Input label="historyTurnMaxChars (100-4000)" type="text" bind:value={ragParams.historyTurnMaxChars} />
                <Input label="temperature (0-2)" type="text" bind:value={ragParams.temperature} />
                <Input label="maxTokens (64-32000)" type="text" bind:value={ragParams.maxTokens} />
              </div>
              <Button variant="secondary" size="sm" onclick={resetRagParams}>{t('settings.ragParams.restoreDefaults')}</Button>
            </div>
          </div>
        </section>
      </Card>

    {:else if activeTab === 'dependencias'}
      <DependenciasTab />

    {:else if activeTab === 'sync'}
      <SyncSettingsCard />
    {:else if activeTab === 'logs'}
      <LogsTab />
    {/if}

    {#if showDiscardConfirm}
      <ConfirmDialog
        title={t('settings.discardTitle')}
        titleId="settings-discard-title"
        message={t('settings.discardMessage')}
        cancelLabel={t('settings.discardCancel')}
        confirmLabel={t('settings.discardConfirm')}
        variant="destructive"
        oncancel={() => (showDiscardConfirm = false)}
        onconfirm={handleDiscardConfirm}
      />
    {/if}
  </div>
{/key}

<style>
  .settings-view {
    min-height: 100%;
  }

  .settings-view__toolbar {
    justify-content: flex-end;
    flex: 1;
    align-self: center;
  }

  .settings-view__header {
    border-color: color-mix(in srgb, var(--color-success) 18%, var(--color-hairline));
    background:
      linear-gradient(180deg, color-mix(in srgb, var(--color-success-soft) 62%, transparent), transparent 70%),
      color-mix(in srgb, var(--color-surface-glass) 72%, transparent);
    box-shadow: var(--shadow-sm);
    backdrop-filter: blur(10px);
  }

  .settings-view__header .page-header__eyebrow {
    color: color-mix(in srgb, var(--color-success) 78%, white 22%);
  }

  .settings-view__header .page-header__meta {
    color: var(--color-text-secondary);
    line-height: 1.5;
  }

  .settings-tab__badge {
    display: inline-block;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--color-warning);
    margin-left: var(--space-1);
    vertical-align: middle;
    animation: tab-badge-pulse 2s ease-in-out 3;
  }

  @keyframes tab-badge-pulse {
    0%, 100% { box-shadow: 0 0 0 0 transparent; }
    50% { box-shadow: 0 0 0 3px color-mix(in srgb, var(--color-warning) 25%, transparent); }
  }

  .settings-view :global(.card) {
    border-color: color-mix(in srgb, var(--color-success) 14%, var(--color-hairline));
    background:
      linear-gradient(180deg, color-mix(in srgb, var(--color-success-soft) 34%, transparent), transparent 72%),
      color-mix(in srgb, var(--color-surface-glass) 74%, transparent);
    box-shadow: var(--shadow-sm);
    backdrop-filter: blur(10px);
  }

  .settings-view :global(.card__header),
  .settings-view :global(.card__footer) {
    background-color: color-mix(in srgb, var(--color-surface-glass) 70%, transparent);
    border-color: color-mix(in srgb, var(--color-success) 12%, var(--color-hairline));
  }

  .settings-view :global(.card__body) {
    background: transparent;
  }

  .settings-card-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  .settings-card-section--vertical {
    align-items: stretch;
  }

  .settings-card-section__copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .settings-card-section__copy h2 {
    margin: 0;
    font-size: var(--font-size-base);
    font-weight: var(--font-weight-semibold);
    letter-spacing: -0.01em;
  }

  .settings-card-section__copy p,
  .settings__hint {
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    line-height: 1.6;
    margin: 0;
  }

  .settings__provider-link {
    align-items: center;
    display: inline-flex;
    gap: var(--space-1);
    width: fit-content;
    color: var(--color-accent);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
    text-decoration: none;
  }

  .settings__provider-link:hover {
    text-decoration: underline;
  }

  .settings__mode-options {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .settings__radio {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-4);
    border: 1px solid color-mix(in srgb, var(--color-hairline) 78%, transparent);
    border-radius: var(--radius-md);
    cursor: pointer;
    background: color-mix(in srgb, var(--color-surface-glass) 76%, transparent);
    transition:
      border-color var(--transition-smooth),
      background-color var(--transition-smooth),
      box-shadow var(--transition-smooth),
      transform var(--transition-smooth);
  }

  .settings__radio:hover {
    border-color: color-mix(in srgb, var(--color-accent) 18%, var(--color-hairline));
    background: color-mix(in srgb, var(--color-surface-glass) 86%, transparent);
    transform: translateY(-1px);
  }

  .settings__radio.active {
    border-color: var(--color-accent);
    background: color-mix(in srgb, var(--color-accent) 10%, var(--color-surface-glass));
    box-shadow: var(--shadow-sm);
  }

  .settings__radio input[type='radio'] {
    margin-top: 3px;
    accent-color: var(--color-accent);
  }

  .settings__radio-content {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .settings__radio-content strong {
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
  }

  .settings__radio-desc {
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    line-height: 1.5;
  }

  .settings__badge {
    display: inline-block;
    margin-left: var(--space-2);
    padding: 2px 8px;
    border-radius: var(--radius-full);
    font-size: 10px;
    font-weight: var(--font-weight-medium);
    vertical-align: middle;
  }
  .settings__badge--ok {
    background: var(--color-success-soft);
    color: var(--color-success);
  }
  .settings__badge--warn {
    background: var(--color-warning-soft);
    color: var(--color-warning);
  }

  .settings__field {
    margin-bottom: var(--space-1);
  }

  .settings__field--stacked {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .settings__field--wide {
    grid-column: 1 / -1;
  }

  .settings__prompt-grid,
  .settings__params-grid {
    display: grid;
    gap: var(--space-4);
  }

  .settings__prompt-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    align-items: stretch;
  }

  .settings__params-grid {
    grid-template-columns: repeat(auto-fit, minmax(260px, 1fr));
  }

  .settings__params-grid--flows {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    align-items: stretch;
  }

  .settings__prompt-card {
    min-height: 430px;
    margin-bottom: 0;
    padding: var(--space-4);
    border: 1px solid color-mix(in srgb, var(--color-hairline) 72%, transparent);
    border-radius: var(--radius-lg);
    background: color-mix(in srgb, var(--color-surface-glass) 70%, transparent);
  }

  .settings__prompt-card .settings__textarea {
    flex: 1;
    min-height: 300px;
  }

  .settings__button-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-top: auto;
  }

  .settings__prompt-card .settings__button-row :global(.btn) {
    align-self: flex-start;
    margin-top: 0;
  }

  .settings__validation {
    margin: 0;
    color: var(--color-success);
    font-size: var(--font-size-sm);
    line-height: 1.5;
  }

  .settings__validation--error {
    color: var(--color-danger);
  }

  .settings__param-card {
    min-height: 520px;
    margin-bottom: 0;
    padding: var(--space-4);
    border: 1px solid color-mix(in srgb, var(--color-hairline) 72%, transparent);
    border-radius: var(--radius-lg);
    background: color-mix(in srgb, var(--color-surface-glass) 70%, transparent);
  }

  .settings__param-card h3 {
    margin: 0;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
  }

  .settings__param-card-grid {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-3);
  }

  .settings__param-card :global(.btn) {
    align-self: flex-start;
    margin-top: auto;
  }

  .settings__textarea {
    width: 100%;
    border: 1px solid color-mix(in srgb, var(--color-hairline) 78%, transparent);
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--color-surface-glass) 78%, transparent);
    color: var(--color-text-primary);
    padding: var(--space-3);
    font: inherit;
    font-family: var(--font-mono, monospace);
    font-size: var(--font-size-sm);
    line-height: 1.5;
    resize: vertical;
  }

  .settings__textarea:focus {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
    background: color-mix(in srgb, var(--color-surface-glass) 88%, transparent);
  }

  @media (max-width: 760px) {
    .settings__prompt-grid,
    .settings__params-grid--flows {
      grid-template-columns: 1fr;
    }

    .settings__param-card-grid {
      grid-template-columns: 1fr;
    }
  }

  .settings__label {
    display: block;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    color: color-mix(in srgb, var(--color-text-secondary) 86%, white 14%);
    margin-bottom: var(--space-1);
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }

  .settings__input-row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    align-items: center;
  }

  .settings__input {
    flex: 1;
    min-height: var(--control-height-md);
    padding: 0 var(--space-3);
    border: 1px solid color-mix(in srgb, var(--color-hairline) 78%, transparent);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--color-surface-glass) 78%, transparent);
    color: var(--color-text-primary);
    font-family: var(--font-mono, monospace);
    font-size: var(--font-size-sm);
  }

  .settings__input:focus {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
    background: color-mix(in srgb, var(--color-surface-glass) 88%, transparent);
  }

  .settings__input--select {
    max-width: 240px;
    font-family: var(--font-sans);
  }

  .settings__icon-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: var(--control-height-md);
    height: var(--control-height-md);
    border: 1px solid color-mix(in srgb, var(--color-hairline) 78%, transparent);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--color-surface-glass) 78%, transparent);
    color: var(--color-text-secondary);
    cursor: pointer;
    font-size: var(--font-size-sm);
    transition:
      background-color var(--transition-base),
      border-color var(--transition-base),
      color var(--transition-base),
      box-shadow var(--transition-base);
  }

  .settings__icon-btn:hover {
    border-color: color-mix(in srgb, var(--color-accent) 18%, var(--color-hairline));
    background: color-mix(in srgb, var(--color-surface-glass) 88%, transparent);
  }

  .settings__icon-btn:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .settings-view :global(.input-field__input) {
    border-color: color-mix(in srgb, var(--color-hairline) 78%, transparent);
    background-color: color-mix(in srgb, var(--color-surface-glass) 78%, transparent);
  }

  .settings-view :global(.input-field__input:focus),
  .settings-view :global(.input-field__input:focus-visible) {
    background-color: color-mix(in srgb, var(--color-surface-glass) 88%, transparent);
  }

  .settings-view :global(.btn--secondary) {
    border-color: color-mix(in srgb, var(--color-hairline) 78%, transparent);
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.04), transparent 55%),
      color-mix(in srgb, var(--color-surface-glass) 78%, transparent);
    box-shadow: none;
  }

  .settings-view :global(.btn--secondary:hover:not(:disabled)) {
    border-color: color-mix(in srgb, var(--color-accent) 18%, var(--color-hairline));
    background-color: color-mix(in srgb, var(--color-surface-glass) 88%, transparent);
  }

  .settings__feedback {
    margin: 0;
    line-height: 1.55;
  }

  .settings__load-error {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .settings__hint--privacy {
    margin: 0;
    padding: var(--space-3);
    border: 1px solid color-mix(in srgb, var(--color-warning) 35%, transparent);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--color-warning) 10%, var(--color-surface-glass));
  }

  .settings__model-list {
    max-height: 240px;
    overflow-y: auto;
    border: 1px solid color-mix(in srgb, var(--color-hairline) 78%, transparent);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--color-surface-glass) 72%, transparent);
  }

  .settings__model-list-title {
    padding: var(--space-2) var(--space-3);
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    border-bottom: 1px solid color-mix(in srgb, var(--color-hairline) 72%, transparent);
  }
  .settings__model-option {
    display: flex;
    justify-content: space-between;
    align-items: center;
    width: 100%;
    padding: var(--space-2) var(--space-3);
    border: none;
    background: transparent;
    cursor: pointer;
    font-family: var(--font-sans);
    font-size: var(--font-size-sm);
    text-align: left;
    transition: background-color var(--transition-smooth);
  }
  .settings__model-option:hover {
    background: color-mix(in srgb, var(--color-surface-glass) 82%, transparent);
  }

  .settings__model-option.selected {
    background: color-mix(in srgb, var(--color-accent) 10%, var(--color-surface-glass));
    font-weight: var(--font-weight-medium);
  }

  .settings__model-option + .settings__model-option {
    border-top: 1px solid var(--color-border-subtle);
  }

  .settings__model-id {
    color: var(--color-text-primary);
  }

  .settings__model-ctx {
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .settings__local-model {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .settings__local-model-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    flex-wrap: wrap;
  }

  .settings__local-model-path {
    font-family: var(--font-mono, monospace);
    font-size: var(--font-size-xs);
    background: var(--color-surface-sunken);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-sm);
    color: var(--color-text-secondary);
    word-break: break-all;
  }

  .settings__local-model-size {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    font-family: var(--font-mono, monospace);
  }

  .settings__local-model-guide {
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    margin: 0;
    line-height: 1.5;
  }

  .settings__local-model-guide code {
    font-family: var(--font-mono, monospace);
    background: var(--color-surface-sunken);
    padding: 2px 6px;
    border-radius: var(--radius-sm);
    font-size: var(--font-size-xs);
  }

  .settings__download-progress {
    position: relative;
    height: 24px;
    background: var(--color-surface-sunken);
    border-radius: var(--radius-md);
    overflow: hidden;
  }

  .settings__download-progress-bar {
    position: absolute;
    top: 0;
    left: 0;
    height: 100%;
    background: var(--color-accent);
    opacity: 0.25;
    transition: width 0.2s ease;
  }

  .settings__download-progress-text {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-primary);
    z-index: 1;
  }

  @media (max-width: 720px) {
    .settings-view__toolbar,
    .settings__input-row {
      width: 100%;
    }

    .settings-view__toolbar :global(.btn),
    .settings__input-row :global(.btn) {
      width: 100%;
    }

    .settings__icon-btn {
      flex: 0 0 auto;
    }
  }
</style>
