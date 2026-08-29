import { beforeEach, describe, expect, it, vi } from 'vitest'
import { get } from 'svelte/store'

const { settingsGetMock, settingsSetMock } = vi.hoisted(() => ({
  settingsGetMock: vi.fn(),
  settingsSetMock: vi.fn(),
}))

vi.mock('$lib/settings', async () => {
  const actual = await vi.importActual<typeof import('$lib/settings')>('$lib/settings')
  return {
    ...actual,
    settingsGet: settingsGetMock,
    settingsSet: settingsSetMock,
  }
})

describe('i18n', () => {
  beforeEach(async () => {
    settingsGetMock.mockReset().mockResolvedValue(null)
    settingsSetMock.mockReset().mockResolvedValue(undefined)

    const { locale } = await import('./i18n')
    locale.set('es')
  })

  it('defaults to spanish when no preference is stored', async () => {
    const { initLocale, locale, t } = await import('./i18n')

    await initLocale()

    expect(settingsGetMock).toHaveBeenCalledWith('language')
    expect(get(locale)).toBe('es')
    expect(t('app.initializing')).toBe('Inicializando...')
  })

  it('loads a saved language preference when it exists', async () => {
    settingsGetMock.mockResolvedValueOnce('en')

    const { initLocale, locale, t } = await import('./i18n')

    await initLocale()

    expect(get(locale)).toBe('en')
    expect(t('app.initializing')).toBe('Initializing...')
  })

  it('persists locale changes through frontend settings', async () => {
    const { setLocale, locale } = await import('./i18n')

    await setLocale('en')

    expect(settingsSetMock).toHaveBeenCalledWith('language', 'en')
    expect(get(locale)).toBe('en')
  })

  it('exposes db browser action copy in both locales', async () => {
    const { locale, t } = await import('./i18n')

    expect(t('dbBrowser.copyCell')).toBe('Copiar')
    expect(t('dbBrowser.pageSizeLabel')).toBe('Filas por página')
    expect(t('dbBrowser.exportJson')).toBe('Exportar JSON')
    expect(t('dbBrowser.exportCsv')).toBe('Exportar CSV')

    locale.set('en')

    expect(t('dbBrowser.copyCell')).toBe('Copy')
    expect(t('dbBrowser.pageSizeLabel')).toBe('Rows per page')
    expect(t('dbBrowser.exportJson')).toBe('Export JSON')
    expect(t('dbBrowser.exportCsv')).toBe('Export CSV')
  })

  it('exposes rag chat copy in both locales', async () => {
    const { locale, t } = await import('./i18n')

    expect(t('nav.ragChat')).toBe('Chat')
    expect(t('topbar.ragChatAria')).toBe('Abrir chat de investigación')
    expect(t('topbar.ragChatTitle')).toBe('Chat de investigación')
    expect(t('ragChat.title')).toBe('Chat de investigación')
    expect(t('ragChat.subtitle')).toBe(
      'Consultá la base de conocimiento de transcripciones y documentos OCR'
    )
    expect(t('ragChat.placeholder')).toBe('Escribí tu pregunta…')
    expect(t('ragChat.send')).toBe('Enviar')
    expect(t('ragChat.thinking')).toBe('Buscando en la base de conocimiento…')
    expect(t('ragChat.sources')).toBe('Fuentes')
    expect(t('ragChat.noResults')).toBe(
      'No encontré contenido relevante en la base de conocimiento para esa pregunta.'
    )
    expect(t('ragChat.emptyState')).toBe(
      'Hacé una pregunta sobre tus transcripciones y documentos. Las respuestas citan las fuentes.'
    )
    expect(t('ragChat.errorGeneric')).toBe('Ocurrió un error al consultar.')
    expect(t('ragChat.clear')).toBe('Nueva conversación')
    expect(t('ragChat.openSource')).toBe('Abrir fuente')
    expect(t('ragChat.conversations')).toBe('Conversaciones')
    expect(t('ragChat.noConversations')).toBe('Sin conversaciones todavía')
    expect(t('ragChat.deleteConversation')).toBe('Eliminar conversación')
    expect(t('ragChat.deleteConversationTitle')).toBe('¿Eliminar esta conversación?')
    expect(t('ragChat.deleteConversationMessage')).toBe(
      'Se va a eliminar la conversación y sus mensajes. Esta acción no se puede deshacer.'
    )
    expect(t('ragChat.confirmDelete')).toBe('Eliminar')
    expect(t('ragChat.editConversationName')).toBe('Editar nombre de la conversación')
    expect(t('ragChat.emptyConversationTitle')).toBe('El nombre de la conversación no puede estar vacío.')
    expect(t('ragChat.updateConversationTitleError')).toBe(
      'No se pudo guardar el nombre de la conversación.'
    )

    locale.set('en')

    expect(t('nav.ragChat')).toBe('Chat')
    expect(t('topbar.ragChatAria')).toBe('Open research chat')
    expect(t('topbar.ragChatTitle')).toBe('Research chat')
    expect(t('ragChat.title')).toBe('Research chat')
    expect(t('ragChat.subtitle')).toBe(
      'Query the knowledge base of transcriptions and OCR documents'
    )
    expect(t('ragChat.placeholder')).toBe('Type your question…')
    expect(t('ragChat.send')).toBe('Send')
    expect(t('ragChat.thinking')).toBe('Searching the knowledge base…')
    expect(t('ragChat.sources')).toBe('Sources')
    expect(t('ragChat.noResults')).toBe(
      'I did not find relevant content in the knowledge base for that question.'
    )
    expect(t('ragChat.emptyState')).toBe(
      'Ask a question about your transcriptions and documents. Answers cite their sources.'
    )
    expect(t('ragChat.errorGeneric')).toBe('Something went wrong while querying.')
    expect(t('ragChat.clear')).toBe('New conversation')
    expect(t('ragChat.openSource')).toBe('Open source')
    expect(t('ragChat.conversations')).toBe('Conversations')
    expect(t('ragChat.noConversations')).toBe('No conversations yet')
    expect(t('ragChat.deleteConversation')).toBe('Delete conversation')
    expect(t('ragChat.deleteConversationTitle')).toBe('Delete this conversation?')
    expect(t('ragChat.deleteConversationMessage')).toBe(
      'The conversation and its messages will be deleted. This action cannot be undone.'
    )
    expect(t('ragChat.confirmDelete')).toBe('Delete')
    expect(t('ragChat.editConversationName')).toBe('Edit conversation name')
    expect(t('ragChat.emptyConversationTitle')).toBe('The conversation name cannot be empty.')
    expect(t('ragChat.updateConversationTitleError')).toBe(
      'The conversation name could not be saved.'
    )
  })

  it('exposes settings prompts and model params copy in both locales', async () => {
    const { locale, t } = await import('./i18n')

    expect(t('settings.prompts.validate')).toBe('Validar cambios')
    expect(t('settings.promptValidation.valid')).toBe('Prompt válido.')
    expect(t('settings.promptValidation.missingText')).toBe('Debe incluir el placeholder {text}.')
    expect(t('settings.getApiKeyLink', { provider: 'OpenRouter' })).toBe(
      'Obtener API key en OpenRouter'
    )
    expect(t('settings.modelParams.invalidParam', { flow: 'Summary', param: 'topP' })).toBe(
      'Parámetro inválido en Summary: topP'
    )
    expect(t('settings.modelParams.hint.temperature')).toBe(
      'Temperatura: gradúa la creatividad de la respuesta generada (0-2)'
    )
    expect(t('settings.modelParams.hint.model')).toBe(
      'Modelo OpenRouter usado exclusivamente por este proceso'
    )
    expect(t('settings.modelParams.hint.stopSequences')).toBe(
      'Secuencias de parada: hasta 4 coincidencias exactas, una por línea'
    )
    expect(t('settings.ragParamsTab')).toBe('RAG Params')
    expect(t('settings.ragParams.title')).toBe('RAG Params')
    expect(t('settings.ragParams.description')).toBe(
      'Estos parámetros ajustan la recuperación del chat de investigación. Los valores mostrados son los vigentes.'
    )
    expect(t('settings.ragParams.hint.temperature')).toBe(
      'Temperatura: gradúa la creatividad del modelo (0-2)'
    )
    expect(t('settings.ragParams.hint.maxTokens')).toBe(
      'Respuesta: limita tokens generados por el modelo (64-16000)'
    )
    expect(t('settings.ragParams.invalidParam', { param: 'topK' })).toBe(
      'Parámetro RAG inválido: topK'
    )
    expect(t('settings.ragParams.snippetVsContext')).toBe(
      'snippetMaxChars no puede superar contextMaxChars.'
    )
    expect(t('settings.ragParams.restoreDefaults')).toBe('Restaurar defaults')

    locale.set('en')

    expect(t('settings.prompts.validate')).toBe('Validate changes')
    expect(t('settings.promptValidation.valid')).toBe('Prompt is valid.')
    expect(t('settings.promptValidation.missingText')).toBe(
      'It must include the {text} placeholder.'
    )
    expect(t('settings.getApiKeyLink', { provider: 'OpenRouter' })).toBe(
      'Get an API key at OpenRouter'
    )
    expect(t('settings.modelParams.invalidParam', { flow: 'Summary', param: 'topP' })).toBe(
      'Invalid parameter in Summary: topP'
    )
    expect(t('settings.modelParams.hint.temperature')).toBe(
      'Temperature: controls creativity in the generated response (0-2)'
    )
    expect(t('settings.modelParams.hint.model')).toBe(
      'OpenRouter model used exclusively by this process'
    )
    expect(t('settings.modelParams.hint.stopSequences')).toBe(
      'Stop sequences: up to 4 exact matches, one per line'
    )
    expect(t('settings.ragParamsTab')).toBe('RAG Params')
    expect(t('settings.ragParams.title')).toBe('RAG Params')
    expect(t('settings.ragParams.description')).toBe(
      'These parameters tune retrieval for the research chat. The values shown are the ones currently in effect.'
    )
    expect(t('settings.ragParams.hint.temperature')).toBe(
      "Temperature: controls the model's creativity level (0-2)"
    )
    expect(t('settings.ragParams.hint.maxTokens')).toBe(
      'Response: limits tokens generated by the model (64-16000)'
    )
    expect(t('settings.ragParams.invalidParam', { param: 'topK' })).toBe(
      'Invalid RAG parameter: topK'
    )
    expect(t('settings.ragParams.snippetVsContext')).toBe(
      'snippetMaxChars cannot exceed contextMaxChars.'
    )
    expect(t('settings.ragParams.restoreDefaults')).toBe('Restore defaults')
  })

  it('preserves local model wiring copy in both locales', async () => {
    const { locale, t } = await import('./i18n')

    // LLM mode
    expect(t('settings.llmModeTitle')).toBe('Modo LLM')
    expect(t('settings.llmMode.local.label')).toBe('Local')
    expect(t('settings.llmMode.local.description')).toBe(
      'Motor LLM local vía llama.cpp. Sin conexión a internet.'
    )
    expect(t('settings.llmMode.auto.label')).toBe('Automático')
    // Embeddings local provider
    expect(t('settings.embeddingProvider.local.label')).toBe('Local ONNX')
    expect(t('settings.embeddingProvider.localPath')).toBe('Carpeta del modelo local BGE-M3')
    expect(t('settings.embeddingProvider.installLocal')).toBe('Instalar BGE-M3 local')
    // STT / OCRH local modes
    expect(t('settings.sttMode.local.label')).toBe('Local')
    expect(t('settings.ocrhMode.local.label')).toBe('Local')
    // Badges + local model download UI
    expect(t('settings.badge.available')).toBe('Disponible')
    expect(t('settings.badge.downloadable')).toBe('Descargable')
    expect(t('settings.localModel.title')).toBe('Modelo local (Gemma)')
    expect(t('settings.localModel.download')).toBe('Descargar modelo')
    expect(t('settings.localModel.downloadError')).toBe('Error en la descarga')

    locale.set('en')

    expect(t('settings.llmModeTitle')).toBe('LLM mode')
    expect(t('settings.llmMode.local.label')).toBe('Local')
    expect(t('settings.llmMode.local.description')).toBe(
      'Local LLM engine via llama.cpp. No internet required.'
    )
    expect(t('settings.llmMode.auto.label')).toBe('Automatic')
    expect(t('settings.embeddingProvider.local.label')).toBe('Local ONNX')
    expect(t('settings.embeddingProvider.localPath')).toBe('Local BGE-M3 model folder')
    expect(t('settings.embeddingProvider.installLocal')).toBe('Install local BGE-M3')
    expect(t('settings.sttMode.local.label')).toBe('Local')
    expect(t('settings.ocrhMode.local.label')).toBe('Local')
    expect(t('settings.badge.available')).toBe('Available')
    expect(t('settings.badge.downloadable')).toBe('Downloadable')
    expect(t('settings.localModel.title')).toBe('Local model (Gemma)')
    expect(t('settings.localModel.download')).toBe('Download model')
    expect(t('settings.localModel.downloadError')).toBe('Download failed')
  })
  it('translates extracted-text actions, states, and export formats in Spanish and English', async () => {
    const { locale, t } = await import('./i18n')

    locale.set('es')
    expect(t('item.extractedTextTab')).toBe('Texto extraído')
    expect(t('item.copyExtractedText')).toBe('Copiar')
    expect(t('item.copyExtractedTextAria')).toBe('Copiar texto extraído')
    expect(t('item.copyExtractedTextSuccess')).toBe('Texto extraído copiado')
    expect(t('item.copyExtractedTextError')).toBe('No se pudo copiar el texto extraído')
    expect(t('item.downloadExtractedText')).toBe('Descargar')
    expect(t('item.downloadExtractedTextAria')).toBe('Descargar texto extraído')
    expect(t('item.downloadExtractedTextMenu')).toBe('Formatos de descarga del texto extraído')
    expect(t('item.exportExtractedTextMarkdown')).toBe('Markdown (.md)')
    expect(t('item.exportExtractedTextPdf')).toBe('PDF (.pdf)')
    expect(t('item.exportExtractedTextDocx')).toBe('Word (.docx)')
    expect(t('item.exportExtractedTextWorking')).toBe('Generando exportación…')
    expect(t('item.exportExtractedTextError')).toBe('No se pudo generar la exportación')
    expect(t('item.ocrImageUnavailable')).toBe('Imagen OCR no disponible')
    expect(t('item.extractedText')).toBe('Texto extraído')
    expect(t('item.audioTranscription')).toBe('Transcripción de audio')
    expect(t('item.noExtractedText')).toBe('Todavía no hay texto extraído para este documento.')
    expect(t('item.extractionFailed', { error: 'boom' })).toBe('Falló la extracción: boom')
    expect(t('item.transcriptionFailed', { error: 'boom' })).toBe('Falló la transcripción: boom')
    expect(t('item.ocrMethodUnknown')).toBe('desconocido')
    expect(t('item.characters', { count: 12 })).toBe('12 caracteres')
    expect(t('item.audioDurationSeconds', { count: 9 })).toBe('9s')
    expect(t('item.transcription')).toBe('Transcripción')

    locale.set('en')
    expect(t('item.extractedTextTab')).toBe('Extracted text')
    expect(t('item.copyExtractedText')).toBe('Copy')
    expect(t('item.copyExtractedTextAria')).toBe('Copy extracted text')
    expect(t('item.copyExtractedTextSuccess')).toBe('Extracted text copied')
    expect(t('item.copyExtractedTextError')).toBe('Could not copy extracted text')
    expect(t('item.downloadExtractedText')).toBe('Download')
    expect(t('item.downloadExtractedTextAria')).toBe('Download extracted text')
    expect(t('item.downloadExtractedTextMenu')).toBe('Extracted text download formats')
    expect(t('item.exportExtractedTextMarkdown')).toBe('Markdown (.md)')
    expect(t('item.exportExtractedTextPdf')).toBe('PDF (.pdf)')
    expect(t('item.exportExtractedTextDocx')).toBe('Word (.docx)')
    expect(t('item.exportExtractedTextWorking')).toBe('Generating export…')
    expect(t('item.exportExtractedTextError')).toBe('Could not generate export')
    expect(t('item.ocrImageUnavailable')).toBe('OCR image unavailable')
    expect(t('item.extractedText')).toBe('Extracted text')
    expect(t('item.audioTranscription')).toBe('Audio transcription')
    expect(t('item.noExtractedText')).toBe('There is no extracted text for this document yet.')
    expect(t('item.extractionFailed', { error: 'boom' })).toBe('Extraction failed: boom')
    expect(t('item.transcriptionFailed', { error: 'boom' })).toBe('Transcription failed: boom')
    expect(t('item.ocrMethodUnknown')).toBe('unknown')
    expect(t('item.characters', { count: 12 })).toBe('12 chars')
    expect(t('item.audioDurationSeconds', { count: 9 })).toBe('9s')
    expect(t('item.transcription')).toBe('Transcription')
  })
})
