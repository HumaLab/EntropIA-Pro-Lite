import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { locale } from '$lib/i18n'
import { HELP_MANUAL_URL } from '$lib/help'
import HelpView from './HelpView.svelte'

const mockInvoke = vi.mocked(invoke)

const { navigationRef } = vi.hoisted(() => ({
  navigationRef: { navigate: vi.fn(), openRootSection: vi.fn() },
}))

vi.mock('$lib/pane-context', () => ({
  getNavigation: () => navigationRef,
  getPaneId: () => 'pane-test',
}))

const PREVIEW_TEXT = 'Producto: EntropIA Lite\n- llaves: configurada / no configurada'

const DEFAULT_FAQ = [
  { group: 'Primeros pasos', question: '¿Cómo importo un PDF?' },
  { group: 'Primeros pasos', question: '¿Cómo creo mi primera colección?' },
  { group: 'Preguntas frecuentes', question: '¿Cómo hago una copia de seguridad?' },
]

function answer(
  text: string,
  sources: Array<{ chapter: string; title: string }> = [],
  local = false
) {
  return { answer: text, sources, local }
}

/** Answers each command on its own, as the real backend does. */
function backend(options: {
  hasKey: boolean
  ask?: unknown
  preview?: string
  faq?: unknown
  askCalls?: unknown[]
  historyCalls?: unknown[]
  list?: unknown
  detail?: unknown
}) {
  const askCalls: unknown[] = options.askCalls ?? []
  const historyCalls: unknown[] = options.historyCalls ?? []
  // A faithful fake of the history store: list serves the live rows, so
  // delete and clear are visible to the panel like against the backend.
  const liveList: Array<{ id: string }> = Array.isArray(options.list)
    ? [...(options.list as Array<{ id: string }>)]
    : []
  mockInvoke.mockImplementation(async (command: string, args?: unknown) => {
    if (command === 'help_status') return { hasOpenrouterKey: options.hasKey }
    if (command === 'help_diagnostics_preview') return options.preview ?? PREVIEW_TEXT
    if (command === 'help_history_list') return liveList
    if (command === 'help_history_get') {
      const detail = options.detail
      if (detail instanceof Error) throw detail
      return detail ?? { id: 'c-1', title: 'Guardada', messages: [] }
    }
    if (command === 'help_history_delete') {
      historyCalls.push({ command, args })
      const id = (args as { id?: string } | undefined)?.id
      const at = liveList.findIndex((item) => item.id === id)
      if (at !== -1) liveList.splice(at, 1)
      return undefined
    }
    if (command === 'help_history_clear' || command === 'help_history_append') {
      historyCalls.push({ command, args })
      if (command === 'help_history_clear') liveList.length = 0
      return undefined
    }
    if (command === 'help_faq_list') {
      const faq = options.faq
      if (faq instanceof Error) throw faq
      return faq ?? DEFAULT_FAQ
    }
    if (command === 'help_ask') {
      askCalls.push(args)
      const asked = options.ask
      if (asked instanceof Error) throw asked
      return asked ?? answer('Respuesta de ayuda', [])
    }
    return undefined
  })
  return askCalls
}

async function composer(): Promise<HTMLElement> {
  return await screen.findByRole('textbox', { name: 'Escribí tu pregunta…' })
}

/** The <pre> keeps the exact preview text, whitespace included. */
async function previewBlock(): Promise<HTMLElement> {
  return await screen.findByText((_content, element) => element?.textContent === PREVIEW_TEXT)
}

/** Opens the collapsed «Fuentes y pasos» disclosure of an answer (S20). */
async function expandMeta(name: string | RegExp): Promise<void> {
  await fireEvent.click(await screen.findByRole('button', { name }))
}

async function sendQuestion(text: string) {
  const input = await composer()
  await fireEvent.input(input, { target: { value: text } })
  await fireEvent.click(screen.getByRole('button', { name: 'Enviar' }))
}

describe('HelpView', () => {
  beforeEach(() => {
    locale.set('es')
    navigationRef.navigate.mockReset()
    navigationRef.openRootSection.mockReset()
    mockInvoke.mockReset()
  })

  it('checks for an OpenRouter key and loads the faq starters on mount', async () => {
    backend({ hasKey: true })
    render(HelpView)

    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith('help_status'))
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith('help_faq_list'))
    await composer()
  })

  it('the no-key page keeps the key explanation and still offers starters and input', async () => {
    backend({ hasKey: false })
    render(HelpView)

    expect(await screen.findByText('La ayuda usa tu cuenta de OpenRouter')).toBeInTheDocument()
    expect(
      screen.getByText(/Configuración › APIs remotas › OpenRouter/, { exact: false })
    ).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Abrir Configuración' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Ver el manual en línea' })).toBeInTheDocument()
    // Local answers still work without a key (S6, S10): the input stays.
    expect(await composer()).toBeInTheDocument()
    expect(await screen.findByRole('button', { name: '¿Cómo importo un PDF?' })).toBeInTheDocument()
  })

  it('the no-key page opens Configuración and the online manual', async () => {
    backend({ hasKey: false })
    render(HelpView)

    await fireEvent.click(await screen.findByRole('button', { name: 'Abrir Configuración' }))
    expect(navigationRef.openRootSection).toHaveBeenCalledWith({ name: 'settings' })

    await fireEvent.click(screen.getByRole('button', { name: 'Ver el manual en línea' }))
    expect(mockInvoke).toHaveBeenCalledWith('open_external_url', { url: HELP_MANUAL_URL })
  })

  it('sends the question with history and no diagnostics when the checkbox is off', async () => {
    const askCalls = backend({ hasKey: true })
    render(HelpView)

    await sendQuestion('¿Cómo importo un PDF?')

    await waitFor(() => expect(askCalls).toHaveLength(1))
    expect(askCalls[0]).toEqual({
      question: '¿Cómo importo un PDF?',
      history: [],
      requestId: expect.any(String),
      conversationId: expect.any(String),
    })
    expect('diagnostics' in (askCalls[0] as Record<string, unknown>)).toBe(false)
    expect(await screen.findByText('Respuesta de ayuda')).toBeInTheDocument()
  })

  it('the documents checkbox is off by default', async () => {
    backend({ hasKey: true })
    render(HelpView)
    await composer()

    expect(screen.getByRole('checkbox', { name: 'Buscar en mis documentos' })).not.toBeChecked()
  })

  it('hints at the document opt-in through its info icon tooltip, not a paragraph', async () => {
    backend({ hasKey: true })
    render(HelpView)
    await composer()

    expect(
      screen.getByRole('button', { name: 'Más información sobre Buscar en mis documentos' })
    ).toHaveAttribute(
      'data-tooltip',
      'Fragmentos de tus documentos viajarán a OpenRouter solo por esta pregunta.'
    )
  })

  it('checkbox on: the send passes documents:true, then resets to off', async () => {
    const askCalls = backend({ hasKey: true })
    render(HelpView)
    await composer()

    await fireEvent.click(screen.getByRole('checkbox', { name: 'Buscar en mis documentos' }))
    await sendQuestion('¿Qué dice mi acta?')

    await waitFor(() => expect(askCalls).toHaveLength(1))
    expect(askCalls[0]).toEqual({
      question: '¿Qué dice mi acta?',
      history: [],
      documents: true,
      requestId: expect.any(String),
      conversationId: expect.any(String),
    })
    expect(screen.getByRole('checkbox', { name: 'Buscar en mis documentos' })).not.toBeChecked()
  })

  it('lists the used document titles under the answer', async () => {
    backend({
      hasKey: true,
      ask: {
        answer: 'Dice esto',
        sources: [],
        local: false,
        documents: [{ title: 'Acta del cabildo' }, { title: 'Crónica del puerto' }],
      },
    })
    render(HelpView)

    await sendQuestion('¿Qué dice mi acta?')

    expect(await screen.findByText('Dice esto')).toBeInTheDocument()
    await expandMeta('Fuentes y pasos (2)')
    expect(
      screen.getByText('De tus documentos: Acta del cabildo; Crónica del puerto')
    ).toBeInTheDocument()
  })

  it('no document titles when the box is off', async () => {
    backend({ hasKey: true, ask: answer('Respuesta de ayuda', []) })
    render(HelpView)

    await sendQuestion('¿Cómo importo?')

    expect(await screen.findByText('Respuesta de ayuda')).toBeInTheDocument()
    expect(screen.queryByText(/De tus documentos/)).not.toBeInTheDocument()
  })

  it('renders the assistant answer as markdown and lists the manual sections used', async () => {
    backend({
      hasKey: true,
      ask: answer('**Importá** así', [
        { chapter: 'Capítulo 1. Inicio rápido', title: '1.1 Importar archivos' },
      ]),
    })
    render(HelpView)

    await sendQuestion('¿Cómo importo?')

    const strong = await screen.findByText('Importá')
    expect(strong.tagName).toBe('STRONG')
    await expandMeta('Fuentes y pasos (1)')
    expect(
      screen.getByText('Del manual: Capítulo 1. Inicio rápido › 1.1 Importar archivos')
    ).toBeInTheDocument()
  })

  it('labels a screen-map source as the map and a manual one as the manual', async () => {
    // S17/D15: screen-map hits are labelled «Del mapa de pantallas», not
    // «Del manual»; a `manual` kind and a missing kind both render manual.
    backend({
      hasKey: true,
      ask: {
        answer: 'Listo',
        sources: [
          { chapter: 'Sincronización', title: 'card', kind: 'screenMap' },
          { chapter: 'Capítulo 1. Inicio rápido', title: '1.1 Importar archivos', kind: 'manual' },
          { chapter: 'Capítulo 4', title: '4.2' },
        ],
        local: false,
      },
    })
    render(HelpView)

    await sendQuestion('¿Dónde está el botón Guardar cambios?')

    await expandMeta('Fuentes y pasos (3)')
    expect(screen.getByText('Del mapa de pantallas: Sincronización › card')).toBeInTheDocument()
    expect(
      screen.getByText('Del manual: Capítulo 1. Inicio rápido › 1.1 Importar archivos')
    ).toBeInTheDocument()
    expect(screen.getByText('Del manual: Capítulo 4 › 4.2')).toBeInTheDocument()
  })

  it('marks local answers with the no-OpenRouter badge', async () => {
    backend({
      hasKey: true,
      ask: answer('Pasos locales', [{ chapter: 'Capítulo 4', title: '4.2' }], true),
    })
    render(HelpView)

    await sendQuestion('¿Cómo importo un PDF?')

    expect(await screen.findByText('Pasos locales')).toBeInTheDocument()
    expect(screen.getByText('Respuesta local, sin consultar a OpenRouter')).toBeInTheDocument()
  })

  it('a model answer carries no local badge', async () => {
    backend({
      hasKey: true,
      ask: answer('Respuesta del modelo', [], false),
    })
    render(HelpView)

    await sendQuestion('¿Cómo importo un PDF?')

    expect(await screen.findByText('Respuesta del modelo')).toBeInTheDocument()
    expect(
      screen.queryByText('Respuesta local, sin consultar a OpenRouter')
    ).not.toBeInTheDocument()
  })

  it('a local answer works with no key', async () => {
    backend({
      hasKey: false,
      ask: answer('Pasos locales', [{ chapter: 'Capítulo 4', title: '4.2' }], true),
    })
    render(HelpView)

    await sendQuestion('¿Cómo importo un PDF?')

    expect(await screen.findByText('Pasos locales')).toBeInTheDocument()
    expect(screen.getByText('Respuesta local, sin consultar a OpenRouter')).toBeInTheDocument()
  })

  it('a non-local question with no key shows the key message instead of a generic error', async () => {
    backend({ hasKey: false, ask: new Error('help_no_openrouter_key') })
    render(HelpView)

    await sendQuestion('¿Quién escribió el Facundo?')

    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('Cargá tu clave en Configuración')
    // The input stays so a local question can still be asked.
    expect(screen.getByRole('textbox', { name: 'Escribí tu pregunta…' })).toHaveValue(
      '¿Quién escribió el Facundo?'
    )
  })

  it('marks local answers in english too', async () => {
    locale.set('en')
    backend({
      hasKey: true,
      ask: answer('Local steps', [], true),
    })
    render(HelpView)

    const input = await screen.findByRole('textbox', { name: 'Type your question…' })
    await fireEvent.input(input, { target: { value: 'How do I import a PDF?' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Send' }))

    expect(await screen.findByText('Local steps')).toBeInTheDocument()
    expect(screen.getByText('Local answer, without consulting OpenRouter')).toBeInTheDocument()
  })

  it('starter buttons come from the Primeros pasos faq list', async () => {
    backend({ hasKey: true })
    render(HelpView)
    await composer()

    expect(
      await screen.findByRole('button', { name: '¿Cómo creo mi primera colección?' })
    ).toBeInTheDocument()
    expect(
      screen.queryByRole('button', { name: '¿Cómo hago una copia de seguridad?' })
    ).not.toBeInTheDocument()
  })

  it('S18: a starter button asks its question at once, without copying it into the composer', async () => {
    const calls = backend({ hasKey: true, ask: answer('Pasos locales', [], true) })
    render(HelpView)
    await composer()

    await fireEvent.click(
      await screen.findByRole('button', { name: '¿Cómo creo mi primera colección?' })
    )

    await waitFor(() => expect(calls).toHaveLength(1))
    expect((calls[0] as { question: string }).question).toBe('¿Cómo creo mi primera colección?')
    expect(await screen.findByText('Pasos locales')).toBeInTheDocument()
    expect(
      screen.getByText('¿Cómo creo mi primera colección?', { selector: '.help-view__bubble--user' })
    ).toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: 'Escribí tu pregunta…' })).toHaveValue('')
  })

  it('S18: a starter button keeps a draft the user was typing', async () => {
    const calls = backend({ hasKey: true, ask: answer('Pasos locales', [], true) })
    render(HelpView)
    const input = await composer()
    await fireEvent.input(input, { target: { value: 'mi borrador' } })

    await fireEvent.click(
      await screen.findByRole('button', { name: '¿Cómo creo mi primera colección?' })
    )

    await waitFor(() => expect(calls).toHaveLength(1))
    expect((calls[0] as { question: string }).question).toBe('¿Cómo creo mi primera colección?')
    expect(input).toHaveValue('mi borrador')
  })

  it('falls back to built-in suggestions when the faq list fails', async () => {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'help_status') return { hasOpenrouterKey: true }
      if (command === 'help_faq_list') throw new Error('denied')
      return undefined
    })
    render(HelpView)
    await composer()

    expect(await screen.findByRole('button', { name: '¿Cómo importo un PDF?' })).toBeInTheDocument()
  })

  it('preview shows the exact diagnostic text', async () => {
    backend({ hasKey: true })
    render(HelpView)
    await composer()

    await fireEvent.click(screen.getByRole('button', { name: 'Ver qué se envía' }))

    expect(await previewBlock()).toBeInTheDocument()
    expect(mockInvoke).toHaveBeenCalledWith('help_diagnostics_preview')
  })

  it('checkbox on: the send passes exactly the previewed text, then resets to off', async () => {
    const askCalls = backend({ hasKey: true })
    render(HelpView)
    await composer()

    await fireEvent.click(screen.getByRole('checkbox', { name: 'Incluir diagnóstico' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Ver qué se envía' }))
    await previewBlock()

    const previewCalls = mockInvoke.mock.calls.filter(
      ([command]) => command === 'help_diagnostics_preview'
    ).length

    await sendQuestion('Falla la transcripción')

    await waitFor(() => expect(askCalls).toHaveLength(1))
    expect(askCalls[0]).toEqual({
      question: 'Falla la transcripción',
      history: [],
      diagnostics: PREVIEW_TEXT,
      requestId: expect.any(String),
      conversationId: expect.any(String),
    })
    // The preview is reused, not fetched again for the send.
    expect(
      mockInvoke.mock.calls.filter(([command]) => command === 'help_diagnostics_preview')
    ).toHaveLength(previewCalls)
    expect(screen.getByRole('checkbox', { name: 'Incluir diagnóstico' })).not.toBeChecked()
  })

  it('checkbox on without a preview fetches it for the send', async () => {
    const askCalls = backend({ hasKey: true })
    render(HelpView)
    await composer()

    await fireEvent.click(screen.getByRole('checkbox', { name: 'Incluir diagnóstico' }))
    await sendQuestion('Falla algo')

    await waitFor(() => expect(askCalls).toHaveLength(1))
    expect(askCalls[0]).toEqual({
      question: 'Falla algo',
      history: [],
      diagnostics: PREVIEW_TEXT,
      requestId: expect.any(String),
      conversationId: expect.any(String),
    })
  })

  it('leaves diagnostics out when the checkbox is off even after previewing', async () => {
    const askCalls = backend({ hasKey: true })
    render(HelpView)
    await composer()

    await fireEvent.click(screen.getByRole('button', { name: 'Ver qué se envía' }))
    await previewBlock()
    await sendQuestion('Otra pregunta')

    await waitFor(() => expect(askCalls).toHaveLength(1))
    expect('diagnostics' in (askCalls[0] as Record<string, unknown>)).toBe(false)
  })

  it('a missing-key rejection mid-chat shows the key message and keeps the draft', async () => {
    backend({ hasKey: true, ask: new Error('help_no_openrouter_key') })
    render(HelpView)

    await sendQuestion('¿Cómo importo?')

    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent('Cargá tu clave en Configuración')
    expect(screen.getByRole('textbox', { name: 'Escribí tu pregunta…' })).toHaveValue(
      '¿Cómo importo?'
    )
  })

  it('a generic failure shows a notice and keeps the typed question', async () => {
    backend({ hasKey: true, ask: new Error('boom') })
    render(HelpView)

    await sendQuestion('¿Cómo importo?')

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'No se pudo responder. Probá de nuevo.'
    )
    expect(screen.getByRole('textbox', { name: 'Escribí tu pregunta…' })).toHaveValue(
      '¿Cómo importo?'
    )
  })

  it('a failed status check is not reported as a missing key', async () => {
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'help_status') throw new Error('Command help_status not allowed by ACL')
      if (command === 'help_faq_list') return DEFAULT_FAQ
      return undefined
    })
    render(HelpView)

    expect(await screen.findByRole('textbox')).toBeInTheDocument()
    expect(screen.queryByText('La ayuda usa tu cuenta de OpenRouter')).not.toBeInTheDocument()
  })

  it('shows a loading state while waiting for the answer', async () => {
    let release!: (value: unknown) => void
    mockInvoke.mockImplementation(async (command: string) => {
      if (command === 'help_status') return { hasOpenrouterKey: true }
      if (command === 'help_faq_list') return DEFAULT_FAQ
      if (command === 'help_ask') return new Promise((resolve) => (release = resolve))
      return undefined
    })
    render(HelpView)

    await sendQuestion('¿Cómo importo?')

    expect(await screen.findByText('Buscando en el manual…')).toBeInTheDocument()
    release(answer('listo', []))
    await waitFor(() =>
      expect(screen.queryByText('Buscando en el manual…')).not.toBeInTheDocument()
    )
  })

  it('S12: does not resend document-derived answers without renewed consent', async () => {
    const secret = 'El cabildo aprobó el presupuesto reservado.'
    const calls = backend({
      hasKey: true,
      ask: { answer: secret, sources: [], local: false, documents: [{ title: 'Acta privada' }] },
    })
    render(HelpView)
    await composer()
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Buscar en mis documentos' }))
    await sendQuestion('¿Qué dice mi acta?')
    await screen.findByText(secret)

    await sendQuestion('Explicame las opciones de ayuda')
    await waitFor(() => expect(calls).toHaveLength(2))
    const next = calls[1] as { documents?: boolean; history: Array<{ content: string }> }
    expect(next.documents).not.toBe(true)
    expect(next.history.map((turn) => turn.content).join('\n')).not.toContain(secret)
    expect(next.history.map((turn) => turn.content)).toContain('¿Qué dice mi acta?')
  })

  it('caps the history sent at 12 turns, the most the settings allow', async () => {
    let n = 0
    const askCalls = backend({ hasKey: true })
    mockInvoke.mockImplementation(async (command: string, args?: unknown) => {
      if (command === 'help_status') return { hasOpenrouterKey: true }
      if (command === 'help_faq_list') return DEFAULT_FAQ
      if (command === 'help_diagnostics_preview') return PREVIEW_TEXT
      if (command === 'help_ask') {
        askCalls.push(args)
        n += 1
        return answer(`respuesta ${n}`, [])
      }
      return undefined
    })
    render(HelpView)
    await composer()

    for (let i = 0; i < 8; i += 1) {
      const input = screen.getByRole('textbox', { name: 'Escribí tu pregunta…' })
      await fireEvent.input(input, { target: { value: `pregunta ${i}` } })
      await fireEvent.click(screen.getByRole('button', { name: 'Enviar' }))
      await screen.findByText(`respuesta ${i + 1}`)
    }

    expect(askCalls).toHaveLength(8)
    const last = askCalls[7] as { history: Array<{ content: string }> }
    expect(last.history).toHaveLength(12)
    expect(last.history[0]?.content).toBe('pregunta 1')
  })

  it('hints at the diagnostics opt-in through its info icon tooltip, not a paragraph', async () => {
    backend({ hasKey: true })
    render(HelpView)
    await composer()

    expect(
      screen.getByRole('button', { name: 'Más información sobre Incluir diagnóstico' })
    ).toHaveAttribute(
      'data-tooltip',
      'Viaja a OpenRouter junto con tu pregunta. No incluye claves ni el contenido de tus documentos.'
    )
  })

  describe('embedded in the floating panel (T15)', () => {
    it('hides the big header card but keeps the chat usable', async () => {
      backend({ hasKey: true })
      render(HelpView, { props: { embedded: true } })
      await composer()

      expect(document.querySelector('.help-view--embedded')).not.toBeNull()
      expect(document.querySelector('.help-view__header')).toBeNull()
      expect(
        screen.queryByText('Preguntá cómo usar EntropIA o qué hacer cuando algo falla')
      ).not.toBeInTheDocument()
    })

    it('without the prop the full-page header card stays', async () => {
      backend({ hasKey: true })
      render(HelpView)
      await composer()

      expect(document.querySelector('.help-view__header')).not.toBeNull()
      expect(
        screen.getByText('Preguntá cómo usar EntropIA o qué hacer cuando algo falla')
      ).toBeInTheDocument()
    })

    it('the composer lives in the fixed footer region', async () => {
      backend({ hasKey: true })
      render(HelpView)
      const input = await composer()

      expect(input.closest('.help-view__composer')).not.toBeNull()
      expect(
        screen.getByRole('button', { name: 'Enviar' }).closest('.help-view__composer')
      ).not.toBeNull()
    })

    it('the diagnostics preview renders inside the scrolling messages region', async () => {
      backend({ hasKey: true })
      render(HelpView)
      await composer()

      await fireEvent.click(screen.getByRole('button', { name: 'Ver qué se envía' }))

      const pre = await previewBlock()
      expect(pre.closest('.help-view__messages')).not.toBeNull()
      expect(pre.closest('.help-view__composer')).toBeNull()
    })
  })

  describe('agent approvals and steps (T22)', () => {
    it('lists the tool steps under the answer', async () => {
      backend({
        hasKey: true,
        ask: {
          answer: 'Listo',
          sources: [],
          local: false,
          documents: [],
          steps: [
            { tool: 'collections_list', args: '{}', status: 'ok' },
            { tool: 'collections_create', args: '{"name":"SIMAPE"}', status: 'ok' },
          ],
          roundLimitReached: false,
          toolsUnavailable: false,
        },
      })
      render(HelpView)

      await sendQuestion('Creá una colección SIMAPE')

      await screen.findByText('Listo')
      await expandMeta('Fuentes y pasos (2)')
      expect(
        screen.getByText('Usó: Listó colecciones, Creó la colección «SIMAPE»')
      ).toBeInTheDocument()
    })

    it('no steps line when the answer ran no tools', async () => {
      backend({ hasKey: true, ask: answer('Respuesta de ayuda', []) })
      render(HelpView)

      await sendQuestion('¿Cómo importo?')

      expect(await screen.findByText('Respuesta de ayuda')).toBeInTheDocument()
      expect(screen.queryByText(/Usó:/)).not.toBeInTheDocument()
      // No metadata at all: not even the disclosure toggle renders.
      expect(screen.queryByRole('button', { name: /Fuentes y pasos/ })).not.toBeInTheDocument()
    })

    it('notes the round limit and unavailable tools', async () => {
      backend({
        hasKey: true,
        ask: {
          answer: 'A medias',
          sources: [],
          local: false,
          documents: [],
          steps: [{ tool: 'search_items', args: '{"query":"x"}', status: 'ok' }],
          roundLimitReached: true,
          toolsUnavailable: true,
        },
      })
      render(HelpView)

      await sendQuestion('Hacé muchas cosas')

      expect(await screen.findByText('A medias')).toBeInTheDocument()
      expect(screen.getByText(/límite de rondas/)).toBeInTheDocument()
      expect(screen.getByText(/sin usar herramientas/)).toBeInTheDocument()
    })
  })

  describe('live steps and Detener (T24)', () => {
    /** The last help://step handler, the way the approval tests read theirs. */
    async function fireStep(step: {
      requestId: string
      tool: string
      args: string
      status: string
    }) {
      const { listen } = await import('@tauri-apps/api/event')
      const handlers = vi
        .mocked(listen)
        .mock.calls.filter(([name]) => name === 'help://step')
        .map(([, handler]) => handler) as Array<(event: { payload: typeof step }) => void>
      const handler = handlers.at(-1)
      if (!handler) throw new Error('HelpView is not listening to help://step')
      handler({ payload: step })
    }

    /** An ask that stays in flight until the test releases it. */
    function deferredBackend(askCalls: unknown[]) {
      let release!: (value: unknown) => void
      mockInvoke.mockImplementation(async (command: string, args?: unknown) => {
        if (command === 'help_status') return { hasOpenrouterKey: true }
        if (command === 'help_faq_list') return DEFAULT_FAQ
        if (command === 'help_ask') {
          askCalls.push(args)
          return new Promise((resolve) => (release = resolve))
        }
        return undefined
      })
      return (value: unknown) => release(value)
    }

    it('sends a requestId generated per question', async () => {
      const askCalls: unknown[] = []
      backend({ hasKey: true })
      const calls = askCalls
      mockInvoke.mockImplementation(async (command: string, args?: unknown) => {
        if (command === 'help_status') return { hasOpenrouterKey: true }
        if (command === 'help_faq_list') return DEFAULT_FAQ
        if (command === 'help_ask') {
          calls.push(args)
          return answer('listo', [])
        }
        return undefined
      })
      render(HelpView)

      await sendQuestion('¿Cómo importo?')

      await waitFor(() => expect(calls).toHaveLength(1))
      const first = calls[0] as { requestId?: unknown }
      expect(typeof first.requestId).toBe('string')
      expect((first.requestId as string).length).toBeGreaterThan(0)
    })

    it('shows live steps in order under Pensando as they arrive, then the final list', async () => {
      const askCalls: unknown[] = []
      const release = deferredBackend(askCalls)
      render(HelpView)

      await sendQuestion('Creá una colección SIMAPE')

      expect(await screen.findByText('Buscando en el manual…')).toBeInTheDocument()
      const requestId = (askCalls[0] as { requestId: string }).requestId
      expect(requestId).toBeTruthy()

      await fireStep({ requestId, tool: 'collections_list', args: '{}', status: 'ok' })
      await fireStep({
        requestId,
        tool: 'collections_create',
        args: '{"name":"SIMAPE"}',
        status: 'ok',
      })

      expect(await screen.findByText('Listó colecciones')).toBeInTheDocument()
      expect(screen.getByText('Creó la colección «SIMAPE»')).toBeInTheDocument()

      // Steps from another run never leak into this question.
      await fireStep({ requestId: 'other-run', tool: 'items_list', args: '{}', status: 'ok' })
      expect(screen.queryByText('Listó documentos')).not.toBeInTheDocument()

      release({
        answer: 'Listo',
        sources: [],
        local: false,
        documents: [],
        steps: [
          { tool: 'collections_list', args: '{}', status: 'ok' },
          { tool: 'collections_create', args: '{"name":"SIMAPE"}', status: 'ok' },
        ],
        roundLimitReached: false,
        toolsUnavailable: false,
      })

      expect(await screen.findByText('Listo')).toBeInTheDocument()
      // The final list stays under the answer; the live list is gone, so no
      // label shows twice. The final list lives behind the disclosure.
      await expandMeta('Fuentes y pasos (2)')
      expect(screen.queryByText('Buscando en el manual…')).not.toBeInTheDocument()
      expect(document.querySelectorAll('.help-view__live-steps li')).toHaveLength(0)
      expect(
        screen.getByText('Usó: Listó colecciones, Creó la colección «SIMAPE»')
      ).toBeInTheDocument()
    })

    it('marks a denied live step', async () => {
      const askCalls: unknown[] = []
      const release = deferredBackend(askCalls)
      render(HelpView)

      await sendQuestion('Creá una colección SIMAPE')
      await screen.findByText('Buscando en el manual…')
      const requestId = (askCalls[0] as { requestId: string }).requestId

      await fireStep({
        requestId,
        tool: 'collections_create',
        args: '{"name":"SIMAPE"}',
        status: 'denied',
      })

      expect(await screen.findByText('Creó la colección «SIMAPE» (denegado)')).toBeInTheDocument()
      release({
        answer: 'No puedo sin tu aprobación.',
        sources: [],
        local: false,
        documents: [],
        steps: [{ tool: 'collections_create', args: '{"name":"SIMAPE"}', status: 'denied' }],
        roundLimitReached: false,
        toolsUnavailable: false,
      })
      expect(await screen.findByText('No puedo sin tu aprobación.')).toBeInTheDocument()
    })

    it('Detener swaps the send button and invokes help_stop with the run id', async () => {
      const askCalls: unknown[] = []
      const release = deferredBackend(askCalls)
      render(HelpView)

      await sendQuestion('Hacé muchas cosas')

      const stop = await screen.findByRole('button', { name: 'Detener' })
      expect(screen.queryByRole('button', { name: 'Enviar' })).not.toBeInTheDocument()
      const requestId = (askCalls[0] as { requestId: string }).requestId

      await fireEvent.click(stop)

      await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith('help_stop', { requestId }))
      release({
        answer: 'Detenido.',
        sources: [],
        local: false,
        documents: [],
        steps: [{ tool: 'collections_list', args: '{}', status: 'ok' }],
        roundLimitReached: false,
        toolsUnavailable: false,
      })
      expect(await screen.findByText('Detenido.')).toBeInTheDocument()
      // The send button is back once the run settles.
      expect(await screen.findByRole('button', { name: 'Enviar' })).toBeInTheDocument()
    })
  })

  describe('MCP approval outcomes (T26)', () => {
    const UUID = '29189757-1545-411c-b1cc-93132f7aaab7'

    /** Fires a help://approval-outcome event the way the backend emits it. */
    async function fireOutcome(outcome: { approvalId: string; status: string; message: string }) {
      const { listen } = await import('@tauri-apps/api/event')
      const handlers = vi
        .mocked(listen)
        .mock.calls.filter(([name]) => name === 'help://approval-outcome')
        .map(([, handler]) => handler) as Array<(event: { payload: typeof outcome }) => void>
      const handler = handlers.at(-1)
      if (!handler) throw new Error('HelpView is not listening to help://approval-outcome')
      handler({ payload: outcome })
    }

    it('shows the pending request label while the owner decides, with the name', async () => {
      backend({
        hasKey: true,
        ask: {
          answer: 'Pedí borrarla; aprobala en el diálogo.',
          sources: [],
          local: false,
          documents: [],
          steps: [
            {
              tool: 'collections_delete',
              args: `{"collectionId":"${UUID}"}`,
              status: 'pending',
              target: 'SIMAPE',
              approvalId: 'apr_1',
            },
          ],
          roundLimitReached: false,
          toolsUnavailable: false,
        },
      })
      render(HelpView)

      await sendQuestion('Borrá la colección SIMAPE')

      expect(await screen.findByText('Pedí borrarla; aprobala en el diálogo.')).toBeInTheDocument()
      await expandMeta('Fuentes y pasos (1)')
      const label = await screen.findByText('Usó: Pidió borrar la colección «SIMAPE»')
      expect(label).toBeInTheDocument()
      expect(screen.queryByText(new RegExp(UUID))).not.toBeInTheDocument()
    })

    it('appends the outcome notice after the right answer and flips the step label', async () => {
      backend({
        hasKey: true,
        ask: {
          answer: 'Pedí borrarla; aprobala en el diálogo.',
          sources: [],
          local: false,
          documents: [],
          steps: [
            {
              tool: 'collections_delete',
              args: `{"collectionId":"${UUID}"}`,
              status: 'pending',
              target: 'SIMAPE',
              approvalId: 'apr_1',
            },
          ],
          roundLimitReached: false,
          toolsUnavailable: false,
        },
      })
      render(HelpView)

      await sendQuestion('Borrá la colección SIMAPE')
      await expandMeta('Fuentes y pasos (1)')
      await screen.findByText('Usó: Pidió borrar la colección «SIMAPE»')

      await fireOutcome({
        approvalId: 'apr_1',
        status: 'approved',
        message: 'Se eliminó la colección «SIMAPE».',
      })

      expect(await screen.findByText('Se eliminó la colección «SIMAPE».')).toBeInTheDocument()
      expect(await screen.findByText('Usó: Borró la colección «SIMAPE»')).toBeInTheDocument()
      expect(screen.queryByText('Usó: Pidió borrar la colección «SIMAPE»')).not.toBeInTheDocument()
      expect(screen.queryByText(new RegExp(UUID))).not.toBeInTheDocument()
      // The original answer stays: it was true when written.
      expect(screen.getByText('Pedí borrarla; aprobala en el diálogo.')).toBeInTheDocument()
    })

    it('marks a rejected outcome and keeps the raw id out of the label', async () => {
      backend({
        hasKey: true,
        ask: {
          answer: 'Pedí borrarla.',
          sources: [],
          local: false,
          documents: [],
          steps: [
            {
              tool: 'collections_delete',
              args: `{"collectionId":"${UUID}"}`,
              status: 'pending',
              target: 'SIMAPE',
              approvalId: 'apr_2',
            },
          ],
          roundLimitReached: false,
          toolsUnavailable: false,
        },
      })
      render(HelpView)

      await sendQuestion('Borrá la colección SIMAPE')
      await expandMeta('Fuentes y pasos (1)')
      await screen.findByText('Usó: Pidió borrar la colección «SIMAPE»')

      await fireOutcome({
        approvalId: 'apr_2',
        status: 'rejected',
        message: 'Rechazaste borrar la colección «SIMAPE»; no se borró nada.',
      })

      expect(
        await screen.findByText('Rechazaste borrar la colección «SIMAPE»; no se borró nada.')
      ).toBeInTheDocument()
      expect(
        await screen.findByText('Usó: Borrado rechazado: la colección «SIMAPE»')
      ).toBeInTheDocument()
      expect(screen.queryByText(new RegExp(UUID))).not.toBeInTheDocument()
    })

    it('an outcome for another answer does not touch this one', async () => {
      backend({
        hasKey: true,
        ask: {
          answer: 'Pedí borrarla.',
          sources: [],
          local: false,
          documents: [],
          steps: [
            {
              tool: 'collections_delete',
              args: `{"collectionId":"${UUID}"}`,
              status: 'pending',
              target: 'SIMAPE',
              approvalId: 'apr_3',
            },
          ],
          roundLimitReached: false,
          toolsUnavailable: false,
        },
      })
      render(HelpView)

      await sendQuestion('Borrá la colección SIMAPE')
      await expandMeta('Fuentes y pasos (1)')
      await screen.findByText('Usó: Pidió borrar la colección «SIMAPE»')

      await fireOutcome({
        approvalId: 'apr-other',
        status: 'approved',
        message: 'Se eliminó la colección «Otra».',
      })

      expect(screen.queryByText('Se eliminó la colección «Otra».')).not.toBeInTheDocument()
      expect(screen.getByText('Usó: Pidió borrar la colección «SIMAPE»')).toBeInTheDocument()
    })
  })

  describe('inline approvals as a dialog (T27)', () => {
    /** Fires a help://approval request the way the backend emits it. */
    async function approvalRequest(request: { id: string; tool: string; summary: string }) {
      const { listen } = await import('@tauri-apps/api/event')
      const handlers = vi
        .mocked(listen)
        .mock.calls.filter(([name]) => name === 'help://approval')
        .map(([, handler]) => handler) as Array<(event: { payload: typeof request }) => void>
      const handler = handlers.at(-1)
      if (!handler) throw new Error('HelpView is not listening to help://approval')
      handler({ payload: request })
    }

    it('the approval arrives as a modal dialog, not a card in the conversation', async () => {
      backend({ hasKey: true })
      render(HelpView)
      await composer()

      await approvalRequest({
        id: 'help-apr-1',
        tool: 'collections_create',
        summary: 'Crear la colección «SIMAPE»',
      })

      const dialog = await screen.findByRole('dialog')
      expect(dialog).toHaveTextContent('La ayuda pide aprobación')
      expect(dialog).toHaveTextContent('Crear la colección «SIMAPE»')
      expect(screen.getByRole('button', { name: 'Aprobar' })).toBeInTheDocument()
      // No card stays pinned in the conversation.
      expect(document.querySelector('.help-view__bubble--approval')).toBeNull()
      await waitFor(() => expect(screen.getByRole('button', { name: 'Rechazar' })).toHaveFocus())
    })

    it('Aprobar answers, closes the dialog and adds an approved line', async () => {
      backend({ hasKey: true })
      render(HelpView)
      await composer()

      await approvalRequest({
        id: 'help-apr-2',
        tool: 'collections_create',
        summary: 'Crear la colección «SIMAPE»',
      })
      await fireEvent.click(await screen.findByRole('button', { name: 'Aprobar' }))

      await waitFor(() =>
        expect(mockInvoke).toHaveBeenCalledWith('help_approval_answer', {
          id: 'help-apr-2',
          approved: true,
        })
      )
      await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
      expect(await screen.findByText('Aprobaste: Crear la colección «SIMAPE»')).toBeInTheDocument()
      expect(document.querySelector('.help-view__bubble--approval')).toBeNull()
    })

    it('Rechazar answers negatively and adds a rejected line', async () => {
      backend({ hasKey: true })
      render(HelpView)
      await composer()

      await approvalRequest({
        id: 'help-apr-3',
        tool: 'collections_create',
        summary: 'Crear la colección «SIMAPE»',
      })
      await fireEvent.click(await screen.findByRole('button', { name: 'Rechazar' }))

      await waitFor(() =>
        expect(mockInvoke).toHaveBeenCalledWith('help_approval_answer', {
          id: 'help-apr-3',
          approved: false,
        })
      )
      await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
      expect(await screen.findByText('Rechazaste: Crear la colección «SIMAPE»')).toBeInTheDocument()
      expect(document.querySelector('.help-view__bubble--approval')).toBeNull()
    })

    it('Escape rejects, closes the dialog and adds a rejected line', async () => {
      backend({ hasKey: true })
      render(HelpView)
      await composer()

      await approvalRequest({
        id: 'help-apr-4',
        tool: 'collections_create',
        summary: 'Crear la colección «SIMAPE»',
      })
      await screen.findByRole('dialog')
      await fireEvent.keyDown(window, { key: 'Escape' })

      await waitFor(() =>
        expect(mockInvoke).toHaveBeenCalledWith('help_approval_answer', {
          id: 'help-apr-4',
          approved: false,
        })
      )
      await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
      expect(await screen.findByText('Rechazaste: Crear la colección «SIMAPE»')).toBeInTheDocument()
    })
  })

  describe('responsive opt-ins (T28)', () => {
    const DIAG_HINT =
      'Viaja a OpenRouter junto con tu pregunta. No incluye claves ni el contenido de tus documentos.'
    const DOCS_HINT = 'Fragmentos de tus documentos viajarán a OpenRouter solo por esta pregunta.'

    it('maximized: both opt-ins share one single-line row', async () => {
      backend({ hasKey: true })
      render(HelpView, { props: { embedded: true, maximized: true } })
      await composer()

      const row = document.querySelector('.help-view__optins--single')
      expect(row).not.toBeNull()
      expect(
        within(row as HTMLElement).getByRole('checkbox', { name: 'Incluir diagnóstico' })
      ).toBeInTheDocument()
      expect(
        within(row as HTMLElement).getByRole('checkbox', { name: 'Buscar en mis documentos' })
      ).toBeInTheDocument()
    })

    it('normal size: the opt-ins wrap instead of forcing one line', async () => {
      backend({ hasKey: true })
      render(HelpView)
      await composer()

      const row = document.querySelector('.help-view__optins')
      expect(row).not.toBeNull()
      expect(row?.classList.contains('help-view__optins--single')).toBe(false)
    })

    it('hints live on superscript info icons as tooltips, not in paragraphs', async () => {
      backend({ hasKey: true })
      render(HelpView)
      await composer()

      expect(document.querySelector('.help-view__hint')).toBeNull()

      const diagIcon = screen.getByRole('button', {
        name: 'Más información sobre Incluir diagnóstico',
      })
      const docsIcon = screen.getByRole('button', {
        name: 'Más información sobre Buscar en mis documentos',
      })
      expect(diagIcon).toHaveAttribute('data-tooltip', DIAG_HINT)
      expect(docsIcon).toHaveAttribute('data-tooltip', DOCS_HINT)
      for (const [icon, hint] of [
        [diagIcon, DIAG_HINT],
        [docsIcon, DOCS_HINT],
      ] as const) {
        const describedBy = icon.getAttribute('aria-describedby')
        expect(describedBy).toBeTruthy()
        expect(document.getElementById(describedBy!)?.textContent).toBe(hint)
      }
      expect(diagIcon.querySelector('svg[data-action-icon="info"]')).not.toBeNull()
      expect(docsIcon.querySelector('svg[data-action-icon="info"]')).not.toBeNull()
    })

    it('the maximized panel drives the single-line row (stylesheet contract)', async () => {
      const { readFileSync } = await import('node:fs')
      const { resolve } = await import('node:path')
      const styles = readFileSync(resolve(import.meta.dirname, 'HelpView.svelte'), 'utf-8')
        .split('<style>')[1]
        ?.replace(/\/\*[\s\S]*?\*\//g, '')
      const rule = (selector: string): string => {
        const match = new RegExp(
          `(^|\\n)\\s*${selector.replace(/[.]/g, '\\.')}\\s*\\{([^}]*)\\}`
        ).exec(styles ?? '')
        expect(match, `${selector} is no longer in the stylesheet`).not.toBeNull()
        return match?.[2] ?? ''
      }
      // The row wraps by default (the normal floating size) and only the
      // maximized contract forces one line.
      expect(rule('.help-view__optins')).toMatch(/flex-wrap:\s*wrap/)
      expect(rule('.help-view__optins--single')).toMatch(/flex-wrap:\s*nowrap/)
      // The hint icon reads as a superscript, not a paragraph.
      expect(rule('.help-view__info')).toMatch(/vertical-align:\s*super/)
    })
  })
})

describe('answer metadata disclosure (S20)', () => {
  // Like the `HelpView` block above: this top-level describe needs its own
  // reset, otherwise the English test below leaks `en` into the history
  // block that runs after it.
  beforeEach(() => {
    locale.set('es')
    navigationRef.navigate.mockReset()
    navigationRef.openRootSection.mockReset()
    mockInvoke.mockReset()
  })
  const MIXED = {
    answer: 'Pasos locales',
    sources: [
      { chapter: 'Capítulo 4', title: '4.2' },
      { chapter: 'Sincronización', title: 'card', kind: 'screenMap' },
    ],
    local: true,
    documents: [{ title: 'Acta del cabildo' }],
    steps: [
      { tool: 'collections_list', args: '{}', status: 'ok' },
      { tool: 'collections_create', args: '{"name":"SIMAPE"}', status: 'ok' },
    ],
    roundLimitReached: false,
    toolsUnavailable: false,
  }

  it('is collapsed by default: one quiet line with the count, metadata hidden', async () => {
    backend({ hasKey: true, ask: MIXED })
    render(HelpView)

    await sendQuestion('¿Cómo importo un PDF?')

    expect(await screen.findByText('Pasos locales')).toBeInTheDocument()
    // 2 sources + 1 document + 2 steps.
    const toggle = screen.getByRole('button', { name: 'Fuentes y pasos (5)' })
    expect(toggle).toHaveAttribute('aria-expanded', 'false')
    // The local badge stays visible above the disclosure; the details do not.
    expect(screen.getByText('Respuesta local, sin consultar a OpenRouter')).toBeInTheDocument()
    expect(screen.queryByText(/Del manual/)).not.toBeInTheDocument()
    expect(screen.queryByText(/De tus documentos/)).not.toBeInTheDocument()
    expect(screen.queryByText(/Usó:/)).not.toBeInTheDocument()
  })

  it('expands on click and collapses again, with an accessible group label', async () => {
    backend({ hasKey: true, ask: MIXED })
    render(HelpView)

    await sendQuestion('¿Cómo importo un PDF?')

    const toggle = await screen.findByRole('button', { name: 'Fuentes y pasos (5)' })
    await fireEvent.click(toggle)

    expect(toggle).toHaveAttribute('aria-expanded', 'true')
    const details = screen.getByRole('group', { name: 'Detalle de fuentes y pasos' })
    expect(details).toBeInTheDocument()
    expect(within(details).getByText('Del manual: Capítulo 4 › 4.2')).toBeInTheDocument()
    expect(
      within(details).getByText('Del mapa de pantallas: Sincronización › card')
    ).toBeInTheDocument()
    expect(within(details).getByText('De tus documentos: Acta del cabildo')).toBeInTheDocument()
    expect(
      within(details).getByText('Usó: Listó colecciones, Creó la colección «SIMAPE»')
    ).toBeInTheDocument()

    await fireEvent.click(toggle)
    expect(toggle).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByText(/Del manual/)).not.toBeInTheDocument()
  })

  it('labels the toggle in English too', async () => {
    locale.set('en')
    backend({
      hasKey: true,
      ask: {
        answer: 'Local steps',
        sources: [{ chapter: 'Chapter 4', title: '4.2' }],
        local: true,
      },
    })
    render(HelpView)

    const input = await screen.findByRole('textbox', { name: 'Type your question…' })
    await fireEvent.input(input, { target: { value: 'How do I import a PDF?' } })
    await fireEvent.click(screen.getByRole('button', { name: 'Send' }))

    expect(await screen.findByText('Local steps')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Sources and steps (1)' })).toHaveAttribute(
      'aria-expanded',
      'false'
    )
  })

  it('the maximized panel caps the measure and centres the column (stylesheet contract)', async () => {
    const { readFileSync } = await import('node:fs')
    const { resolve } = await import('node:path')
    const styles = readFileSync(resolve(import.meta.dirname, 'HelpView.svelte'), 'utf-8')
      .split('<style>')[1]
      ?.replace(/\/\*[\s\S]*?\*\//g, '')
    const rule = (selector: string): string => {
      const match = new RegExp(
        `(^|\\n)\\s*${selector.replace(/[.]/g, '\\.')}\\s*\\{([^}]*)\\}`
      ).exec(styles ?? '')
      expect(match, `${selector} is no longer in the stylesheet`).not.toBeNull()
      return match?.[2] ?? ''
    }
    expect(rule('.help-view--maximized .help-view__bubble')).toMatch(/max-width:\s*72ch/)
    expect(rule('.help-view--maximized .help-view__bubble--assistant')).toMatch(
      /align-self:\s*center/
    )
    expect(rule('.help-view--maximized .help-view__bubble--assistant')).toMatch(/width:\s*100%/)
    expect(rule('.help-view--maximized .help-view__empty')).toMatch(/max-width:\s*72ch/)
  })

  it('the starters read as a list and the quiet controls keep the shared focus ring', async () => {
    const { readFileSync } = await import('node:fs')
    const { resolve } = await import('node:path')
    const styles = readFileSync(resolve(import.meta.dirname, 'HelpView.svelte'), 'utf-8')
      .split('<style>')[1]
      ?.replace(/\/\*[\s\S]*?\*\//g, '')
    const rule = (selector: string): string => {
      const match = new RegExp(
        `(^|\\n)\\s*${selector.replace(/[.]/g, '\\.')}\\s*\\{([^}]*)\\}`
      ).exec(styles ?? '')
      expect(match, `${selector} is no longer in the stylesheet`).not.toBeNull()
      return match?.[2] ?? ''
    }
    // A tidy column of full-width rows, not wrapping pills.
    expect(rule('.help-view__suggestions-list')).toMatch(/flex-direction:\s*column/)
    expect(rule('.help-view__suggestions-list')).toMatch(/align-items:\s*stretch/)
    // Every keyboard-reachable quiet control draws the shared ring.
    expect(rule('.help-view__meta-toggle:focus-visible')).toMatch(/box-shadow:\s*var\(--focus-ring/)
    expect(rule('.help-view__preview-toggle:focus-visible')).toMatch(
      /box-shadow:\s*var\(--focus-ring/
    )
    // The superscript hint is a 12px glyph on a 24px target.
    expect(rule('.help-view__info')).toMatch(/min-width:\s*24px/)
    expect(rule('.help-view__info')).toMatch(/min-height:\s*24px/)
  })
})

describe('conversation history (S19)', () => {
  const NOW = Date.UTC(2026, 5, 15, 12, 0, 0)
  const SAVED = [
    { id: 'c-1', title: '¿Cómo importo un PDF?', updatedAt: NOW - 5 * 60_000, messageCount: 2 },
    { id: 'c-2', title: 'Falló la transcripción', updatedAt: NOW - 3 * 3_600_000, messageCount: 4 },
  ]

  beforeEach(() => {
    vi.spyOn(Date, 'now').mockReturnValue(NOW)
  })

  async function toggleHistory() {
    await composer()
    await fireEvent.click(screen.getByRole('button', { name: /Historial/ }))
  }

  it('is collapsed by default, with the count on the toggle', async () => {
    backend({ hasKey: true, list: SAVED })
    render(HelpView)
    await composer()

    expect(screen.getByRole('button', { name: 'Historial (2)' })).toBeInTheDocument()
    expect(document.querySelector('.help-view__history-list')).toBeNull()
    // The starter shares the title; the collapsed list shows no date.
    expect(screen.queryByText(/hace 5/)).not.toBeInTheDocument()
  })

  it('the toggle shows the list with title and relative date', async () => {
    backend({ hasKey: true, list: SAVED })
    render(HelpView)
    await toggleHistory()

    const list = document.querySelector('.help-view__history-list') as HTMLElement
    expect(list).not.toBeNull()
    // Scoped to the list: the starters share these titles.
    expect(within(list).getByText('¿Cómo importo un PDF?')).toBeInTheDocument()
    expect(within(list).getByText('Falló la transcripción')).toBeInTheDocument()
    // A relative date next to the title, in the UI language.
    expect(within(list).getByText(/hace 5/)).toBeInTheDocument()
  })

  it('opening a conversation loads its messages and the next question continues it', async () => {
    const askCalls: unknown[] = []
    backend({
      hasKey: true,
      list: SAVED,
      detail: {
        id: 'c-1',
        title: '¿Cómo importo un PDF?',
        messages: [
          { role: 'user', content: '¿Cómo importo un PDF?', meta: {} },
          {
            role: 'assistant',
            content: 'Arrastrá el PDF.',
            meta: {
              kind: 'answer',
              sources: [{ chapter: 'Capítulo 1', title: '1.1', kind: 'manual' }],
              documents: [],
              steps: [],
              local: true,
            },
          },
        ],
      },
      ask: answer('Otra respuesta', []),
      askCalls,
    })
    render(HelpView)
    await toggleHistory()

    // Scoped to the list: «¿Cómo importo un PDF?» is also a starter button.
    const list = document.querySelector('.help-view__history-list') as HTMLElement
    await fireEvent.click(within(list).getByRole('button', { name: /¿Cómo importo un PDF\?/ }))

    expect(await screen.findByText('Arrastrá el PDF.')).toBeInTheDocument()
    expect(screen.getByText('Respuesta local, sin consultar a OpenRouter')).toBeInTheDocument()
    // The list collapses once the conversation is open.
    expect(document.querySelector('.help-view__history-list')).toBeNull()

    await sendQuestion('¿Y después?')

    await waitFor(() => expect(askCalls).toHaveLength(1))
    expect((askCalls[0] as { conversationId?: unknown }).conversationId).toBe('c-1')
  })

  it('S12: a reopened document answer is not resent as history', async () => {
    // A stored answer built from the user's documents (S12) carries its
    // titles in `meta.documents`. Reopening it and asking again must not
    // resend that answer: its consent covered one question only. The
    // user's own question travels.
    const askCalls: unknown[] = []
    backend({
      hasKey: true,
      list: [
        {
          id: 'c-1',
          title: '¿Qué dice mi acta?',
          updatedAt: NOW - 5 * 60_000,
          messageCount: 2,
        },
      ],
      detail: {
        id: 'c-1',
        title: '¿Qué dice mi acta?',
        messages: [
          { role: 'user', content: '¿Qué dice mi acta?', meta: {} },
          {
            role: 'assistant',
            content: 'El acta dice esto.',
            meta: {
              kind: 'answer',
              sources: [],
              documents: [{ title: 'Acta' }],
              steps: [],
              local: false,
            },
          },
        ],
      },
      ask: answer('Otra respuesta', []),
      askCalls,
    })
    render(HelpView)
    await toggleHistory()

    const list = document.querySelector('.help-view__history-list') as HTMLElement
    await fireEvent.click(within(list).getByRole('button', { name: /¿Qué dice mi acta/ }))

    expect(await screen.findByText('El acta dice esto.')).toBeInTheDocument()

    await sendQuestion('¿Y después?')

    await waitFor(() => expect(askCalls).toHaveLength(1))
    const sent = askCalls[0] as { history: Array<{ role: string; content: string }> }
    expect(sent.history.map((turn) => turn.content)).toContain('¿Qué dice mi acta?')
    expect(sent.history.map((turn) => turn.content).join('\n')).not.toContain('El acta dice esto.')
  })

  it('the first answer brings the conversation id and later questions continue it', async () => {
    const askCalls: unknown[] = []
    backend({
      hasKey: true,
      ask: { answer: 'Listo', sources: [], conversationId: 'c-9' },
      askCalls,
    })
    render(HelpView)
    await composer()

    await sendQuestion('Primera pregunta')
    await screen.findByText('Listo')
    await sendQuestion('Segunda pregunta')

    await waitFor(() => expect(askCalls).toHaveLength(2))
    // The first question generated the id it sent: a non-empty string.
    const first = (askCalls[0] as { conversationId?: unknown }).conversationId
    expect(typeof first).toBe('string')
    expect((first as string).length).toBeGreaterThan(0)
    // The answer echoed the stored id; the second question continues it.
    expect((askCalls[1] as { conversationId?: unknown }).conversationId).toBe('c-9')
  })

  it('a new conversation starts empty and drops the stored id', async () => {
    const askCalls: unknown[] = []
    backend({
      hasKey: true,
      ask: { answer: 'Listo', sources: [], conversationId: 'c-9' },
      askCalls,
    })
    render(HelpView)
    await composer()

    await sendQuestion('Primera pregunta')
    await screen.findByText('Listo')

    await fireEvent.click(screen.getByRole('button', { name: 'Nueva conversación' }))

    expect(screen.queryByText('Listo')).not.toBeInTheDocument()
    expect(screen.getByText('Probá con:')).toBeInTheDocument()

    await sendQuestion('Otra cosa')

    await waitFor(() => expect(askCalls).toHaveLength(2))
    const second = (askCalls[1] as { conversationId?: unknown }).conversationId
    expect(typeof second).toBe('string')
    expect(second).not.toBe('c-9')
  })

  it('deleting the open conversation clears the panel', async () => {
    const historyCalls: unknown[] = []
    backend({
      hasKey: true,
      list: SAVED,
      detail: {
        id: 'c-1',
        title: '¿Cómo importo un PDF?',
        messages: [{ role: 'user', content: '¿Cómo importo un PDF?', meta: {} }],
      },
      historyCalls,
    })
    render(HelpView)
    await toggleHistory()
    const list = document.querySelector('.help-view__history-list') as HTMLElement
    await fireEvent.click(within(list).getByRole('button', { name: /¿Cómo importo un PDF\?/ }))
    await screen.findByText('¿Cómo importo un PDF?', { selector: '.help-view__bubble--user' })

    await toggleHistory()
    const deletes = screen.getAllByRole('button', { name: 'Eliminar conversación' })
    await fireEvent.click(deletes[0] as HTMLElement)

    await waitFor(() =>
      expect(historyCalls).toContainEqual({
        command: 'help_history_delete',
        args: { id: 'c-1' },
      })
    )
    expect(
      screen.queryByText('¿Cómo importo un PDF?', { selector: '.help-view__bubble--user' })
    ).not.toBeInTheDocument()
  })

  it('clear asks for confirmation and then deletes everything', async () => {
    const historyCalls: unknown[] = []
    backend({ hasKey: true, list: SAVED, historyCalls })
    render(HelpView)
    await toggleHistory()

    await fireEvent.click(screen.getByRole('button', { name: 'Borrar historial' }))

    const dialog = await screen.findByRole('dialog')
    expect(dialog).toHaveTextContent('¿Borrar el historial?')
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Borrar' }))

    await waitFor(() =>
      expect(historyCalls).toContainEqual({ command: 'help_history_clear', args: undefined })
    )
    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Historial (0)' })).toBeInTheDocument()
    )
  })

  it('an inline approval decision is appended to the stored conversation', async () => {
    const historyCalls: unknown[] = []
    backend({
      hasKey: true,
      ask: { answer: 'Listo', sources: [], conversationId: 'c-7' },
      historyCalls,
    })
    render(HelpView)
    await composer()

    await sendQuestion('Creá una colección SIMAPE')
    await screen.findByText('Listo')

    const { listen } = await import('@tauri-apps/api/event')
    const handlers = vi
      .mocked(listen)
      .mock.calls.filter(([name]) => name === 'help://approval')
      .map(([, handler]) => handler) as Array<(event: { payload: unknown }) => void>
    handlers.at(-1)?.({
      payload: { id: 'help-apr-9', tool: 'collections_create', summary: 'Crear «SIMAPE»' },
    })
    await fireEvent.click(await screen.findByRole('button', { name: 'Aprobar' }))

    await waitFor(() =>
      expect(historyCalls).toContainEqual({
        command: 'help_history_append',
        args: { conversationId: 'c-7', role: 'assistant', content: 'Aprobaste: Crear «SIMAPE»' },
      })
    )
  })
})
