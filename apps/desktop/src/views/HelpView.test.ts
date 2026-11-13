import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
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
}) {
  const askCalls: unknown[] = options.askCalls ?? []
  mockInvoke.mockImplementation(async (command: string, args?: unknown) => {
    if (command === 'help_status') return { hasOpenrouterKey: options.hasKey }
    if (command === 'help_diagnostics_preview') return options.preview ?? PREVIEW_TEXT
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

  it('explains that document fragments go to OpenRouter for this question only', async () => {
    backend({ hasKey: true })
    render(HelpView)
    await composer()

    expect(
      screen.getByText('Fragmentos de tus documentos viajarán a OpenRouter solo por esta pregunta.')
    ).toBeInTheDocument()
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
    expect(
      screen.getByText('Del manual: Capítulo 1. Inicio rápido › 1.1 Importar archivos')
    ).toBeInTheDocument()
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

    await fireEvent.click(screen.getByRole('button', { name: '¿Cómo creo mi primera colección?' }))

    expect(screen.getByRole('textbox', { name: 'Escribí tu pregunta…' })).toHaveValue(
      '¿Cómo creo mi primera colección?'
    )
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

  it('caps the history sent at 6 turns', async () => {
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

    for (let i = 0; i < 5; i += 1) {
      const input = screen.getByRole('textbox', { name: 'Escribí tu pregunta…' })
      await fireEvent.input(input, { target: { value: `pregunta ${i}` } })
      await fireEvent.click(screen.getByRole('button', { name: 'Enviar' }))
      await screen.findByText(`respuesta ${i + 1}`)
    }

    expect(askCalls).toHaveLength(5)
    const last = askCalls[4] as { history: Array<{ content: string }> }
    expect(last.history).toHaveLength(6)
    expect(last.history[0]?.content).toBe('pregunta 1')
  })

  it('explains that diagnostics travel to OpenRouter without keys or document content', async () => {
    backend({ hasKey: true })
    render(HelpView)
    await composer()

    expect(
      screen.getByText(
        'Viaja a OpenRouter junto con tu pregunta. No incluye claves ni el contenido de tus documentos.'
      )
    ).toBeInTheDocument()
  })
})
