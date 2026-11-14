import { describe, expect, it, vi, beforeEach } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import {
  HELP_APPROVAL_OUTCOME_EVENT,
  HELP_MANUAL_URL,
  HELP_NO_OPENROUTER_KEY,
  HELP_STEP_EVENT,
  describeHelpStep,
  formatHistoryDate,
  helpApprovalAnswer,
  helpAsk,
  helpDiagnosticsPreview,
  helpFaqList,
  helpHistoryAppend,
  helpHistoryClear,
  helpHistoryDelete,
  helpHistoryGet,
  helpHistoryList,
  helpStatus,
  helpStop,
  isNoKeyError,
  listenHelpApproval,
  listenHelpApprovalOutcome,
  listenHelpSteps,
  newHelpConversationId,
  openHelpManual,
  type HelpApprovalOutcome,
  type HelpApprovalRequest,
  type HelpStepEvent,
} from './help'

describe('help backend wrappers', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset()
  })

  it('publishes the online manual URL', () => {
    expect(HELP_MANUAL_URL).toBe(
      'https://humalab.github.io/EntropIA-Pro-Lite/manual-usuario/manual-usuario.html'
    )
  })

  it('names the missing-key rejection the backend sends', () => {
    expect(HELP_NO_OPENROUTER_KEY).toBe('help_no_openrouter_key')
  })

  it('helpStatus calls help_status and returns the key flag', async () => {
    vi.mocked(invoke).mockResolvedValue({ hasOpenrouterKey: true })

    await expect(helpStatus()).resolves.toEqual({ hasOpenrouterKey: true })
    expect(invoke).toHaveBeenCalledWith('help_status')
  })

  it('helpDiagnosticsPreview returns the exact report text', async () => {
    vi.mocked(invoke).mockResolvedValue('Producto: EntropIA Lite\n- algo')

    await expect(helpDiagnosticsPreview()).resolves.toBe('Producto: EntropIA Lite\n- algo')
    expect(invoke).toHaveBeenCalledWith('help_diagnostics_preview')
  })

  it('helpAsk sends question, history and the opted-in diagnostics', async () => {
    vi.mocked(invoke).mockResolvedValue({ answer: 'hacelo así', sources: [] })
    const history = [{ role: 'user' as const, content: 'hola' }]

    await helpAsk('¿Cómo importo?', history, 'reporte exacto')

    expect(invoke).toHaveBeenCalledWith('help_ask', {
      question: '¿Cómo importo?',
      history,
      diagnostics: 'reporte exacto',
    })
  })

  it('helpAsk leaves diagnostics out when the checkbox is off', async () => {
    vi.mocked(invoke).mockResolvedValue({ answer: 'hacelo así', sources: [] })

    await helpAsk('¿Cómo importo?', [], null)

    expect(invoke).toHaveBeenCalledWith('help_ask', {
      question: '¿Cómo importo?',
      history: [],
    })
    const args = vi.mocked(invoke).mock.calls[0]?.[1] as Record<string, unknown>
    expect('diagnostics' in args).toBe(false)
  })

  it('helpAsk sends at most the last 12 turns of history', async () => {
    vi.mocked(invoke).mockResolvedValue({ answer: 'hacelo así', sources: [] })
    const history = Array.from({ length: 16 }, (_, index) => ({
      role: (index % 2 === 0 ? 'user' : 'assistant') as 'user' | 'assistant',
      content: `turno ${index}`,
    }))

    await helpAsk('pregunta', history, null)

    const args = vi.mocked(invoke).mock.calls[0]?.[1] as {
      history: Array<{ content: string }>
    }
    expect(args.history).toHaveLength(12)
    expect(args.history[0]?.content).toBe('turno 4')
    expect(args.history[11]?.content).toBe('turno 15')
  })

  it('helpAsk sends documents:true only when the box is ticked', async () => {
    vi.mocked(invoke).mockResolvedValue({ answer: 'hacelo así', sources: [], documents: [] })

    await helpAsk('¿Qué dice mi acta?', [], null, { documents: true })

    expect(invoke).toHaveBeenCalledWith('help_ask', {
      question: '¿Qué dice mi acta?',
      history: [],
      documents: true,
    })
  })

  it('helpAsk leaves documents out when the box is off', async () => {
    vi.mocked(invoke).mockResolvedValue({ answer: 'hacelo así', sources: [] })

    await helpAsk('¿Qué dice mi acta?', [], null)

    expect(invoke).toHaveBeenCalledWith('help_ask', {
      question: '¿Qué dice mi acta?',
      history: [],
    })
    const args = vi.mocked(invoke).mock.calls[0]?.[1] as Record<string, unknown>
    expect('documents' in args).toBe(false)
  })

  it('helpAsk passes the local flag through', async () => {
    vi.mocked(invoke).mockResolvedValue({
      answer: 'hacelo así',
      sources: [{ chapter: 'Capítulo 1. Inicio rápido', title: '1.3. Crear colección' }],
      local: true,
    })

    await expect(helpAsk('¿Cómo creo mi primera colección?', [], null)).resolves.toEqual({
      answer: 'hacelo así',
      sources: [{ chapter: 'Capítulo 1. Inicio rápido', title: '1.3. Crear colección' }],
      local: true,
    })
  })

  it('helpFaqList returns the starter groups and questions', async () => {
    vi.mocked(invoke).mockResolvedValue([
      { group: 'Primeros pasos', question: '¿Cómo importo un PDF?' },
      { group: 'Preguntas frecuentes', question: '¿Cómo hago una copia?' },
    ])

    await expect(helpFaqList()).resolves.toEqual([
      { group: 'Primeros pasos', question: '¿Cómo importo un PDF?' },
      { group: 'Preguntas frecuentes', question: '¿Cómo hago una copia?' },
    ])
    expect(invoke).toHaveBeenCalledWith('help_faq_list')
  })

  it('isNoKeyError recognizes the backend missing-key rejection', () => {
    expect(isNoKeyError('help_no_openrouter_key')).toBe(true)
    expect(isNoKeyError(new Error('help_no_openrouter_key'))).toBe(true)
    expect(isNoKeyError(new Error('invoke help_ask failed: help_no_openrouter_key'))).toBe(true)
    expect(isNoKeyError(new Error('boom'))).toBe(false)
    expect(isNoKeyError(null)).toBe(false)
    expect(isNoKeyError(undefined)).toBe(false)
  })

  it('openHelpManual opens the published manual through the desktop bridge', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)

    await openHelpManual()

    expect(invoke).toHaveBeenCalledWith('open_external_url', { url: HELP_MANUAL_URL })
  })

  it('helpApprovalAnswer sends the card answer to the backend', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)

    await helpApprovalAnswer('help-apr-1', true)
    expect(invoke).toHaveBeenCalledWith('help_approval_answer', {
      id: 'help-apr-1',
      approved: true,
    })

    await helpApprovalAnswer('help-apr-1', false)
    expect(invoke).toHaveBeenCalledWith('help_approval_answer', {
      id: 'help-apr-1',
      approved: false,
    })
  })

  it('listenHelpApproval relays the help://approval payload', async () => {
    const { listen } = await import('@tauri-apps/api/event')
    const seen: HelpApprovalRequest[] = []

    await listenHelpApproval((request) => seen.push(request))

    expect(listen).toHaveBeenCalledWith('help://approval', expect.any(Function))
    const handler = vi
      .mocked(listen)
      .mock.calls.find(([name]) => name === 'help://approval')?.[1] as
      ((event: { payload: HelpApprovalRequest }) => void) | undefined
    handler?.({
      payload: {
        id: 'help-apr-1',
        tool: 'collections_create',
        summary: 'Crear la colección «SIMAPE»',
      },
    })
    expect(seen).toEqual([
      {
        id: 'help-apr-1',
        tool: 'collections_create',
        summary: 'Crear la colección «SIMAPE»',
      },
    ])
  })

  it('helpAsk passes requestId through when the panel generates one', async () => {
    vi.mocked(invoke).mockResolvedValue({ answer: 'listo', sources: [] })

    await helpAsk('¿Cómo importo?', [], null, { requestId: 'r-1' })

    expect(invoke).toHaveBeenCalledWith('help_ask', {
      question: '¿Cómo importo?',
      history: [],
      requestId: 'r-1',
    })
  })

  it('helpAsk leaves requestId out for older callers', async () => {
    vi.mocked(invoke).mockResolvedValue({ answer: 'listo', sources: [] })

    await helpAsk('¿Cómo importo?', [], null)

    const args = vi.mocked(invoke).mock.calls[0]?.[1] as Record<string, unknown>
    expect('requestId' in args).toBe(false)
  })

  it('helpStop invokes help_stop with the run id', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)

    await helpStop('r-1')

    expect(invoke).toHaveBeenCalledWith('help_stop', { requestId: 'r-1' })
  })

  it('listenHelpSteps relays the help://step payload', async () => {
    const { listen } = await import('@tauri-apps/api/event')
    const seen: HelpStepEvent[] = []

    await listenHelpSteps((event) => seen.push(event))

    expect(listen).toHaveBeenCalledWith('help://step', expect.any(Function))
    expect(HELP_STEP_EVENT).toBe('help://step')
    const handler = vi
      .mocked(listen)
      .mock.calls.filter(([name]) => name === 'help://step')
      .at(-1)?.[1] as ((event: { payload: HelpStepEvent }) => void) | undefined
    handler?.({
      payload: { requestId: 'r-1', tool: 'collections_list', args: '{}', status: 'ok' },
    })
    expect(seen).toEqual([{ requestId: 'r-1', tool: 'collections_list', args: '{}', status: 'ok' }])
  })
})

describe('describeHelpStep (T24)', () => {
  it('names the common tools in spanish', () => {
    const es = (tool: string, args = '{}', status: 'ok' | 'denied' | 'failed' = 'ok') =>
      describeHelpStep({ tool, args, status }, 'es')
    expect(es('collections_list')).toBe('Listó colecciones')
    expect(es('documents_get_text')).toBe('Leyó texto de un documento')
    expect(es('hlab_entropia_page')).toBe('Consultó la página de HLab')
  })

  it('names the common tools in english', () => {
    const en = (tool: string, args = '{}', status: 'ok' | 'denied' | 'failed' = 'ok') =>
      describeHelpStep({ tool, args, status }, 'en')
    expect(en('collections_list')).toBe('Listed collections')
    expect(en('documents_get_text')).toBe('Read text from a document')
    expect(en('hlab_entropia_page')).toBe('Checked the HLab page')
  })

  it('carries the query, name or path the args hold', () => {
    expect(
      describeHelpStep({ tool: 'search_items', args: '{"query":"actas"}', status: 'ok' }, 'es')
    ).toBe('Buscó documentos: «actas»')
    expect(
      describeHelpStep({ tool: 'search_items', args: '{"query":"actas"}', status: 'ok' }, 'en')
    ).toBe('Searched documents: “actas”')
    expect(
      describeHelpStep(
        { tool: 'collections_create', args: '{"name":"SIMAPE"}', status: 'ok' },
        'es'
      )
    ).toBe('Creó la colección «SIMAPE»')
    expect(
      describeHelpStep(
        { tool: 'collections_create', args: '{"name":"SIMAPE"}', status: 'ok' },
        'en'
      )
    ).toBe('Created the “SIMAPE” collection')
    expect(
      describeHelpStep({ tool: 'repo_read', args: '{"path":"README.md"}', status: 'ok' }, 'es')
    ).toBe('Leyó del repositorio: README.md')
    expect(
      describeHelpStep({ tool: 'repo_read', args: '{"path":"README.md"}', status: 'ok' }, 'en')
    ).toBe('Read from the repository: README.md')
  })

  it('marks denied and failed steps in both languages', () => {
    expect(describeHelpStep({ tool: 'collections_list', args: '{}', status: 'denied' }, 'es')).toBe(
      'Listó colecciones (denegado)'
    )
    expect(describeHelpStep({ tool: 'collections_list', args: '{}', status: 'failed' }, 'es')).toBe(
      'Listó colecciones (falló)'
    )
    expect(describeHelpStep({ tool: 'collections_list', args: '{}', status: 'denied' }, 'en')).toBe(
      'Listed collections (denied)'
    )
    expect(describeHelpStep({ tool: 'collections_list', args: '{}', status: 'failed' }, 'en')).toBe(
      'Listed collections (failed)'
    )
  })

  it('falls back to the tool name for unknown tools', () => {
    expect(
      describeHelpStep({ tool: 'futura_herramienta', args: '{"a":1}', status: 'ok' }, 'es')
    ).toBe('futura_herramienta')
    expect(
      describeHelpStep({ tool: 'futura_herramienta', args: '{"a":1}', status: 'denied' }, 'en')
    ).toBe('futura_herramienta (denied)')
  })
})

describe('describeHelpStep approval tools (T26)', () => {
  const UUID = '29189757-1545-411c-b1cc-93132f7aaab7'
  const pending = (target?: string) => ({
    tool: 'collections_delete',
    args: `{"collectionId":"${UUID}"}`,
    status: 'pending' as const,
    ...(target === undefined ? {} : { target }),
  })

  it('asks while pending and tells the result once decided, in spanish', () => {
    expect(describeHelpStep(pending('SIMAPE'), 'es')).toBe('Pidió borrar la colección «SIMAPE»')
    expect(describeHelpStep({ ...pending('SIMAPE'), status: 'ok' }, 'es')).toBe(
      'Borró la colección «SIMAPE»'
    )
    expect(describeHelpStep({ ...pending('SIMAPE'), status: 'denied' }, 'es')).toBe(
      'Borrado rechazado: la colección «SIMAPE»'
    )
    expect(describeHelpStep({ ...pending('SIMAPE'), status: 'failed' }, 'es')).toBe(
      'No se pudo borrar la colección «SIMAPE»'
    )
    expect(describeHelpStep({ ...pending('SIMAPE'), status: 'expired' }, 'es')).toBe(
      'El pedido para borrar la colección «SIMAPE» venció'
    )
  })

  it('asks while pending and tells the result once decided, in english', () => {
    expect(describeHelpStep(pending('SIMAPE'), 'en')).toBe(
      'Asked to delete the “SIMAPE” collection'
    )
    expect(describeHelpStep({ ...pending('SIMAPE'), status: 'ok' }, 'en')).toBe(
      'Deleted the “SIMAPE” collection'
    )
    expect(describeHelpStep({ ...pending('SIMAPE'), status: 'denied' }, 'en')).toBe(
      'Deletion rejected: the “SIMAPE” collection'
    )
    expect(describeHelpStep({ ...pending('SIMAPE'), status: 'failed' }, 'en')).toBe(
      'Could not delete the “SIMAPE” collection'
    )
    expect(describeHelpStep({ ...pending('SIMAPE'), status: 'expired' }, 'en')).toBe(
      'The request to delete the “SIMAPE” collection expired'
    )
  })

  it('never shows a raw UUID when the name is known', () => {
    for (const locale of ['es', 'en'] as const) {
      for (const status of ['pending', 'ok', 'denied', 'failed', 'expired'] as const) {
        const label = describeHelpStep({ ...pending('SIMAPE'), status }, locale)
        expect(label).toContain('SIMAPE')
        expect(label).not.toContain(UUID)
      }
    }
  })

  it('falls back to a generic when the name is unknown, never to the id', () => {
    expect(describeHelpStep(pending(), 'es')).toBe('Pidió borrar una colección')
    expect(describeHelpStep(pending(), 'en')).toBe('Asked to delete a collection')
    expect(describeHelpStep({ ...pending(), status: 'ok' }, 'es')).toBe('Borró una colección')
    const label = describeHelpStep(pending(), 'es')
    expect(label).not.toContain(UUID)
    expect(label).not.toContain('col_')
  })

  it('marks pending live steps with the target name', () => {
    expect(
      describeHelpStep(
        {
          tool: 'documents_delete',
          args: '{"itemIds":["ite_1"]}',
          status: 'pending',
          target: 'Acta',
        },
        'es'
      )
    ).toBe('Pidió borrar el documento «Acta»')
    expect(
      describeHelpStep(
        {
          tool: 'documents_delete',
          args: '{"itemIds":["ite_1"]}',
          status: 'pending',
          target: 'Acta',
        },
        'en'
      )
    ).toBe('Asked to delete the “Acta” document')
  })
})

describe('help approval outcomes (T26)', () => {
  it('names the outcome event', () => {
    expect(HELP_APPROVAL_OUTCOME_EVENT).toBe('help://approval-outcome')
  })

  it('relays the help://approval-outcome payload', async () => {
    const { listen } = await import('@tauri-apps/api/event')
    const seen: HelpApprovalOutcome[] = []

    await listenHelpApprovalOutcome((outcome) => seen.push(outcome))

    expect(listen).toHaveBeenCalledWith('help://approval-outcome', expect.any(Function))
    const handler = vi
      .mocked(listen)
      .mock.calls.filter(([name]) => name === 'help://approval-outcome')
      .at(-1)?.[1] as ((event: { payload: HelpApprovalOutcome }) => void) | undefined
    handler?.({
      payload: {
        approvalId: 'apr_1',
        status: 'approved',
        message: 'Se eliminó la colección «SIMAPE».',
      },
    })
    expect(seen).toEqual([
      { approvalId: 'apr_1', status: 'approved', message: 'Se eliminó la colección «SIMAPE».' },
    ])
  })
})

describe('help history backend wrappers (S19)', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset()
  })

  it('helpHistoryList calls help_history_list', async () => {
    vi.mocked(invoke).mockResolvedValue([
      { id: 'c-1', title: '¿Cómo importo?', updatedAt: 1700000000000, messageCount: 2 },
    ])

    await expect(helpHistoryList()).resolves.toEqual([
      { id: 'c-1', title: '¿Cómo importo?', updatedAt: 1700000000000, messageCount: 2 },
    ])
    expect(invoke).toHaveBeenCalledWith('help_history_list')
  })

  it('helpHistoryGet calls help_history_get with the id', async () => {
    const detail = {
      id: 'c-1',
      title: '¿Cómo importo?',
      messages: [
        { role: 'user', content: '¿Cómo importo?', meta: {} },
        { role: 'assistant', content: 'Así.', meta: { kind: 'answer', local: true } },
      ],
    }
    vi.mocked(invoke).mockResolvedValue(detail)

    await expect(helpHistoryGet('c-1')).resolves.toEqual(detail)
    expect(invoke).toHaveBeenCalledWith('help_history_get', { id: 'c-1' })
  })

  it('helpHistoryDelete calls help_history_delete with the id', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)

    await helpHistoryDelete('c-1')

    expect(invoke).toHaveBeenCalledWith('help_history_delete', { id: 'c-1' })
  })

  it('helpHistoryClear calls help_history_clear', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)

    await helpHistoryClear()

    expect(invoke).toHaveBeenCalledWith('help_history_clear')
  })

  it('helpHistoryAppend sends conversation, role and content', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)

    await helpHistoryAppend('c-1', 'assistant', 'Aprobaste: Crear la colección «SIMAPE»')

    expect(invoke).toHaveBeenCalledWith('help_history_append', {
      conversationId: 'c-1',
      role: 'assistant',
      content: 'Aprobaste: Crear la colección «SIMAPE»',
    })
  })

  it('helpAsk passes conversationId through when the panel sends one', async () => {
    vi.mocked(invoke).mockResolvedValue({ answer: 'listo', sources: [] })

    await helpAsk('¿Cómo importo?', [], null, { conversationId: 'c-1' })

    expect(invoke).toHaveBeenCalledWith('help_ask', {
      question: '¿Cómo importo?',
      history: [],
      conversationId: 'c-1',
    })
  })

  it('helpAsk leaves conversationId out for older callers', async () => {
    vi.mocked(invoke).mockResolvedValue({ answer: 'listo', sources: [] })

    await helpAsk('¿Cómo importo?', [], null)

    const args = vi.mocked(invoke).mock.calls[0]?.[1] as Record<string, unknown>
    expect('conversationId' in args).toBe(false)
  })

  it('helpAsk passes the stored conversation id back through', async () => {
    vi.mocked(invoke).mockResolvedValue({
      answer: 'listo',
      sources: [],
      conversationId: 'c-9',
    })

    await expect(helpAsk('¿Cómo importo?', [], null, { conversationId: 'c-9' })).resolves.toEqual({
      answer: 'listo',
      sources: [],
      conversationId: 'c-9',
    })
  })

  it('newHelpConversationId answers unique ids', () => {
    const first = newHelpConversationId()
    const second = newHelpConversationId()
    expect(typeof first).toBe('string')
    expect(first.length).toBeGreaterThan(0)
    expect(first).not.toBe(second)
  })
})

describe('formatHistoryDate (S19)', () => {
  // A fixed Monday noon UTC: dates and relative labels stay deterministic.
  const NOW = Date.UTC(2026, 5, 15, 12, 0, 0)

  it('says now under 45 seconds, in both locales', () => {
    expect(formatHistoryDate(NOW - 10_000, 'es', 'ahora mismo', NOW)).toBe('ahora mismo')
    expect(formatHistoryDate(NOW - 10_000, 'en', 'just now', NOW)).toBe('just now')
  })

  it('uses minutes, hours and days with the UI locale', () => {
    const minutes = formatHistoryDate(NOW - 5 * 60_000, 'es', 'ahora mismo', NOW)
    expect(minutes).toContain('5')
    expect(minutes).toContain('hace')
    expect(formatHistoryDate(NOW - 5 * 60_000, 'en', 'just now', NOW)).toContain('ago')

    const hours = formatHistoryDate(NOW - 3 * 3_600_000, 'es', 'ahora mismo', NOW)
    expect(hours).toContain('3')
    expect(hours).toContain('hace')

    const days = formatHistoryDate(NOW - 2 * 86_400_000, 'en', 'just now', NOW)
    expect(days).toContain('2')
    expect(days).toContain('ago')
  })

  it('falls back to a short date past a week', () => {
    const es = formatHistoryDate(NOW - 10 * 86_400_000, 'es', 'ahora mismo', NOW)
    const en = formatHistoryDate(NOW - 10 * 86_400_000, 'en', 'just now', NOW)
    expect(es).toContain('5')
    expect(en).toContain('5')
    expect(es).not.toBe(en)
  })
})
