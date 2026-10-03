import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import { locale } from '$lib/i18n'
import type { RagAnswer, RagSource } from '$lib/rag'
import { ragChat } from '$lib/rag-chat'
import RagChatView from './RagChatView.svelte'

const { navigateMock } = vi.hoisted(() => ({ navigateMock: vi.fn() }))

vi.mock('$lib/pane-context', () => ({
  getNavigation: () => ({ navigate: navigateMock, openRootSection: vi.fn() }),
  getPaneId: () => 'pane-test',
}))

vi.mock('$lib/rag-chat-export', () => ({ downloadRagConversationPdf: vi.fn() }))

const mockInvoke = vi.mocked(invoke)

interface Backend {
  status: unknown
  ask: (args: Record<string, unknown>) => RagAnswer
  context: () => unknown
}

function setupBackend(overrides: Partial<Backend> = {}): Backend {
  const state: Backend = {
    status: {
      libraries: [
        { libraryType: 'user', libraryId: '0', name: 'Mi biblioteca', works: 12, passages: 340 },
        { libraryType: 'group', libraryId: '77', name: 'Grupo Anales', works: 4, passages: 90 },
      ],
      vectorReady: true,
    },
    ask: () => {
      throw new Error('unexpected rag_ask')
    },
    context: () => {
      throw new Error('unexpected bibliography_passage_context')
    },
    ...overrides,
  }
  mockInvoke.mockImplementation((async (command: string, args?: Record<string, unknown>) => {
    switch (command) {
      case 'settings_get':
        return null
      case 'settings_set':
      case 'settings_delete':
        return undefined
      case 'rag_list_conversations':
        return []
      case 'rag_generate_conversation_title':
        return null
      case 'rag_ask':
        return state.ask(args as Record<string, unknown>)
      case 'bibliography_library_status':
        return state.status
      case 'bibliography_passage_context':
        return state.context()
      case 'bibliography_open_passage':
        return { openedPath: 'C:/Zotero/storage/x.pdf', openError: null }
      default:
        throw new Error(`unexpected command: ${command}`)
    }
  }) as typeof invoke)
  return state
}

function callsFor(command: string): unknown[][] {
  return mockInvoke.mock.calls.filter(([cmd]) => cmd === command)
}

const corpusSource: RagSource = {
  index: 1,
  assetId: 'asset-1',
  itemId: 'item-1',
  itemTitle: 'Entrevista 12',
  collectionId: 'col-1',
  collectionName: 'Historia oral',
  snippet: 'la huelga comenzó cuando los obreros...',
  score: 0.9,
  startSeconds: null,
  endSeconds: null,
  provenance: null,
}

function bibliographySource(
  index: number,
  title: string,
  location: { kind: 'pages' | 'paragraphs'; from: number; to: number }
): RagSource {
  return {
    index,
    assetId: '',
    itemId: `item-${index}`,
    itemTitle: title,
    collectionId: '',
    collectionName: 'Mi biblioteca',
    snippet: `fragmento de ${title}`,
    score: 0.8,
    startSeconds: null,
    endSeconds: null,
    provenance: null,
    bibliography: {
      chunkId: `chunk-${index}`,
      itemKey: 'ABCD1234',
      libraryName: 'Mi biblioteca',
      libraryType: 'user',
      libraryNativeId: '0',
      authors: 'Bloch, Febvre',
      year: 1949,
      location,
    },
  }
}

const mixedAnswer: RagAnswer = {
  answer: 'Una huelga [1], un libro [2] y un sitio [3].',
  sources: [
    corpusSource,
    bibliographySource(2, 'Apología para la historia', { kind: 'pages', from: 3, to: 3 }),
    bibliographySource(3, 'Nota web', { kind: 'paragraphs', from: 2, to: 3 }),
  ],
  model: 'test-model',
  conversationId: 'conv-new',
}

async function ask(question: string) {
  const composer = screen.getByRole('textbox', { name: 'Escribí tu pregunta…' })
  await fireEvent.input(composer, { target: { value: question } })
  await fireEvent.keyDown(composer, { key: 'Enter' })
}

beforeEach(() => {
  locale.set('es')
  navigateMock.mockReset()
  mockInvoke.mockReset()
  ragChat.reset()
})

describe('RagChatView scope control', () => {
  it('offers Corpus, Biblioteca and Ambos, on Corpus, without touching the libraries', async () => {
    setupBackend()
    render(RagChatView)

    const corpus = screen.getByRole('tab', { name: 'Corpus' })
    expect(screen.getByRole('tab', { name: 'Biblioteca' })).toBeInTheDocument()
    expect(screen.getByRole('tab', { name: 'Ambos' })).toBeInTheDocument()
    expect(corpus).toHaveAttribute('aria-selected', 'true')
    expect(screen.queryByRole('button', { name: /bibliotecas/i })).toBeNull()
    expect(callsFor('bibliography_library_status')).toHaveLength(0)
  })

  it('asks for the library state when Biblioteca is chosen and lists "all" by default', async () => {
    setupBackend()
    render(RagChatView)

    await fireEvent.click(screen.getByRole('tab', { name: 'Biblioteca' }))

    expect(screen.getByRole('tab', { name: 'Biblioteca' })).toHaveAttribute('aria-selected', 'true')
    await waitFor(() =>
      expect(screen.getByRole('button', { name: /Todas las bibliotecas/ })).toBeInTheDocument()
    )
    expect(callsFor('bibliography_library_status')).toHaveLength(1)
  })

  it('says honestly that no library is synced, and does not pretend to search', async () => {
    setupBackend({ status: { libraries: [], vectorReady: false } })
    render(RagChatView)

    await fireEvent.click(screen.getByRole('tab', { name: 'Ambos' }))

    await waitFor(() =>
      expect(
        screen.getByText(/Todavía no hay ninguna biblioteca de Zotero sincronizada/)
      ).toBeVisible()
    )
    expect(screen.queryByRole('button', { name: /Todas las bibliotecas/ })).toBeNull()
  })

  it('says honestly when libraries are synced but have no vectors yet', async () => {
    setupBackend({
      status: {
        libraries: [
          { libraryType: 'user', libraryId: '0', name: 'Mi biblioteca', works: 3, passages: 0 },
        ],
        vectorReady: false,
      },
    })
    render(RagChatView)

    await fireEvent.click(screen.getByRole('tab', { name: 'Biblioteca' }))

    await waitFor(() =>
      expect(
        screen.getByText(/todavía no tienen vectores para buscar por significado/)
      ).toBeVisible()
    )
  })

  it('sends the scope with the question and no libraries while "all" is chosen', async () => {
    const state = setupBackend({ ask: vi.fn(() => mixedAnswer) })
    render(RagChatView)
    await fireEvent.click(screen.getByRole('tab', { name: 'Ambos' }))
    await waitFor(() => screen.getByRole('button', { name: /Todas las bibliotecas/ }))

    await ask('¿Qué pasó?')

    await waitFor(() => expect(state.ask).toHaveBeenCalled())
    const payload = (state.ask as ReturnType<typeof vi.fn>).mock.calls[0]![0]
    expect(payload.scope).toBe('both')
    expect(payload.libraries).toBeUndefined()
  })

  it('narrows to the libraries left checked in the ToolbarMenu', async () => {
    const state = setupBackend({ ask: vi.fn(() => mixedAnswer) })
    render(RagChatView)
    await fireEvent.click(screen.getByRole('tab', { name: 'Biblioteca' }))
    const trigger = await screen.findByRole('button', { name: /Todas las bibliotecas/ })

    await fireEvent.click(trigger)
    const mine = await screen.findByRole('menuitemcheckbox', { name: /Mi biblioteca/ })
    expect(mine).toHaveAttribute('aria-checked', 'true')
    await fireEvent.click(mine)

    await waitFor(() =>
      expect(screen.getByRole('button', { name: /1 de 2 bibliotecas/ })).toBeInTheDocument()
    )
    await ask('¿Qué pasó?')

    await waitFor(() => expect(state.ask).toHaveBeenCalled())
    const payload = (state.ask as ReturnType<typeof vi.fn>).mock.calls[0]![0]
    expect(payload.scope).toBe('biblioteca')
    expect(payload.libraries).toEqual([{ libraryType: 'group', libraryId: '77' }])
  })
})

describe('RagChatView sources of both scopes', () => {
  async function renderMixedAnswer() {
    setupBackend({ ask: () => mixedAnswer })
    render(RagChatView)
    await ask('¿Qué pasó?')
    await waitFor(() => expect(screen.getByText('Fuentes')).toBeInTheDocument())
  }

  it('labels each source with its scope and keeps one continuous numbering', async () => {
    await renderMixedAnswer()

    const section = screen.getByText('Fuentes').closest('section') as HTMLElement
    const [first, second, third] = within(section).getAllByRole('listitem') as [
      HTMLElement,
      HTMLElement,
      HTMLElement,
    ]
    expect(within(section).getAllByRole('listitem')).toHaveLength(3)
    expect(within(first).getByText('[1]')).toBeInTheDocument()
    expect(within(first).getByText('Corpus')).toBeInTheDocument()
    expect(within(second).getByText('[2]')).toBeInTheDocument()
    expect(within(second).getByText('Biblioteca')).toBeInTheDocument()
    expect(within(third).getByText('[3]')).toBeInTheDocument()
  })

  it('names the work and the page for a PDF passage', async () => {
    await renderMixedAnswer()

    const [, pdf] = within(
      screen.getByText('Fuentes').closest('section') as HTMLElement
    ).getAllByRole('listitem') as [HTMLElement, HTMLElement]
    expect(within(pdf).getByText('Apología para la historia')).toBeInTheDocument()
    expect(within(pdf).getByText(/Bloch, Febvre · 1949/)).toBeInTheDocument()
    expect(within(pdf).getByText(/Mi biblioteca/)).toBeInTheDocument()
    expect(within(pdf).getByText('p. 3')).toBeInTheDocument()
  })

  it('cites an HTML snapshot by paragraph range and never as page 1', async () => {
    await renderMixedAnswer()

    const [, , html] = within(
      screen.getByText('Fuentes').closest('section') as HTMLElement
    ).getAllByRole('listitem') as [HTMLElement, HTMLElement, HTMLElement]
    expect(within(html).getByText('párr. 2–3')).toBeInTheDocument()
    expect(html.textContent).not.toMatch(/\bp\. 1\b/)
  })

  it('still opens a corpus source as before', async () => {
    await renderMixedAnswer()

    await fireEvent.click(screen.getByRole('button', { name: /Abrir fuente: \[1\]/ }))

    expect(navigateMock).toHaveBeenCalledWith(
      expect.objectContaining({ name: 'item', assetId: 'asset-1' })
    )
  })

  it('shows the Biblioteca notice under an answer the library could not feed', async () => {
    setupBackend({
      ask: () => ({
        answer: 'Solo del corpus [1].',
        sources: [corpusSource],
        model: 'm',
        conversationId: 'c',
        bibliographyNotice: 'no_embeddings',
      }),
    })
    render(RagChatView)
    await ask('¿Qué pasó?')

    await waitFor(() =>
      expect(
        screen.getByText(/No se buscó en la Biblioteca: todavía no tiene vectores/)
      ).toBeVisible()
    )
  })
})

describe('RagChatView passage reader', () => {
  const pageText = 'Antes del pasaje. LO CITADO AQUÍ. Después del pasaje.'
  const start = pageText.indexOf('LO CITADO')
  const end = start + 'LO CITADO AQUÍ.'.length

  function readable() {
    return {
      chunkId: 'chunk-2',
      itemId: 'item-2',
      itemKey: 'ABCD1234',
      title: 'Apología para la historia',
      text: 'LO CITADO AQUÍ.',
      spans: [[3, start, end]],
      pages: [{ pageNumber: 3, text: pageText, highlights: [[start, end]] }],
      openedPath: null,
      openError: null,
    }
  }

  async function openPassage(context: () => unknown) {
    setupBackend({ ask: () => mixedAnswer, context })
    render(RagChatView)
    await ask('¿Qué pasó?')
    await waitFor(() => expect(screen.getByText('Fuentes')).toBeInTheDocument())
    await fireEvent.click(screen.getByRole('button', { name: /Abrir pasaje: \[2\]/ }))
  }

  it('opens the passage in the app with the cited range highlighted, not through navigation', async () => {
    await openPassage(readable)

    const dialog = await screen.findByRole('dialog')
    await waitFor(() => expect(within(dialog).getByText('LO CITADO AQUÍ.').tagName).toBe('MARK'))
    expect(within(dialog).getByText('Apología para la historia')).toBeInTheDocument()
    expect(within(dialog).getByText(/Antes del pasaje\./)).toBeInTheDocument()
    expect(callsFor('bibliography_passage_context')).toEqual([
      ['bibliography_passage_context', { chunkId: 'chunk-2' }],
    ])
    expect(callsFor('bibliography_open_passage')).toHaveLength(0)
    expect(navigateMock).not.toHaveBeenCalled()
  })

  it('opens the original in the OS viewer only when asked', async () => {
    await openPassage(readable)
    const dialog = await screen.findByRole('dialog')

    await fireEvent.click(within(dialog).getByRole('button', { name: 'Abrir original' }))

    await waitFor(() =>
      expect(callsFor('bibliography_open_passage')).toEqual([
        ['bibliography_open_passage', { chunkId: 'chunk-2' }],
      ])
    )
  })

  it('falls back to the saved snippet when the passage is not on this device', async () => {
    await openPassage(() => {
      throw 'unknown_chunk: chunk chunk-2 does not exist'
    })

    const dialog = await screen.findByRole('dialog')
    await waitFor(() =>
      expect(within(dialog).getByText(/no está en la biblioteca de este equipo/)).toBeVisible()
    )
    expect(within(dialog).getByText('fragmento de Apología para la historia')).toBeInTheDocument()
  })

  it('closes with the Close button', async () => {
    await openPassage(readable)
    const dialog = await screen.findByRole('dialog')

    await fireEvent.click(within(dialog).getByRole('button', { name: 'Cerrar' }))

    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
  })
})
