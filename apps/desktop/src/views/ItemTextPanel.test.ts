import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import type { ComponentProps } from 'svelte'
import ItemTextPanel from './ItemTextPanel.svelte'
import { t, locale } from '$lib/i18n'

afterEach(cleanup)
beforeEach(() => {
  vi.clearAllMocks()
  locale.set('es')
})

type Props = ComponentProps<typeof ItemTextPanel>

function setup(edited = false, pendingEdit = false, overrides: Partial<Props> = {}) {
  vi.mocked(invoke).mockResolvedValue(edited)
  const onTranscribeAudio = vi.fn()
  const props: Props = {
    selectedAsset: {
      id: 'audio-1',
      type: 'audio',
      path: '/audio.wav',
      itemId: 'item-1',
      createdAt: 0,
      sortIndex: 0,
      size: null,
    },
    assetsCount: 1,
    allAssetsAreImages: false,
    selectedAssetIndex: 0,
    ocrState: null,
    ocrEditedText: '',
    transcriptionState: {
      status: 'done',
      progress: 100,
      text: 'original',
      editedByHand: pendingEdit,
    },
    transcriptionEditedText: 'original',
    llmState: { status: 'idle', activeJob: null, result: null, error: null },
    llmAvailable: false,
    ocrCorrectionEngine: null,
    localOcrAvailable: false,
    isOcrCorrected: false,
    ocrEditingDisabled: false,
    currentSummary: null,
    isSummarizing: false,
    translate: t,
    onExtractText: vi.fn(),
    onCorrectOcr: vi.fn(),
    onSummarize: vi.fn(),
    onTranscribeAudio,
    onOcrTextInput: vi.fn(),
    onTranscriptionTextInput: vi.fn(),
    onRestoreTranscriptVersion: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  }
  render(ItemTextPanel, props)
  return onTranscribeAudio
}

const imageAsset: Props['selectedAsset'] = {
  id: 'image-1',
  type: 'image',
  path: '/page.png',
  itemId: 'item-1',
  createdAt: 0,
  sortIndex: 0,
  size: null,
}

function ocrState(status: 'running' | 'done' | 'error', progress = 100): Props['ocrState'] {
  return { status, progress, method: 'tesseract', ...(status === 'error' ? { error: 'boom' } : {}) }
}

it('asks before replacing edited text and cancel does not transcribe', async () => {
  const transcribe = setup(true)
  await fireEvent.click(screen.getByRole('button', { name: 'STT' }))
  expect(await screen.findByRole('dialog')).toHaveTextContent(
    'La transcripción tiene correcciones hechas a mano. Si la volvés a transcribir, se reemplaza; la versión corregida queda en «Versiones anteriores». ¿Continuar?'
  )
  expect(transcribe).not.toHaveBeenCalled()
  await fireEvent.click(screen.getByRole('button', { name: 'Cancelar' }))
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  expect(transcribe).not.toHaveBeenCalled()
})

it('transcribes the edited asset only after confirmation', async () => {
  const transcribe = setup(true)
  await fireEvent.click(screen.getByRole('button', { name: 'STT' }))
  await fireEvent.click(await screen.findByRole('button', { name: 'Transcribir de nuevo' }))
  expect(transcribe).toHaveBeenCalledTimes(1)
  expect(transcribe).toHaveBeenCalledWith(expect.objectContaining({ id: 'audio-1' }))
})

it('asks for pending debounced edits even before history records them', async () => {
  const transcribe = setup(false, true)
  await fireEvent.click(screen.getByRole('button', { name: 'STT' }))
  expect(await screen.findByRole('dialog')).toBeInTheDocument()
  expect(transcribe).not.toHaveBeenCalled()
})

it('transcribes at once when the audio has no transcript yet', async () => {
  const transcribe = setup(false, false, {
    transcriptionState: { status: 'idle', progress: 0 },
    transcriptionEditedText: '',
  })
  await fireEvent.click(screen.getByRole('button', { name: 'STT' }))
  await waitFor(() => expect(transcribe).toHaveBeenCalledTimes(1))
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
})

it('asks before replacing an existing transcript, even without hand edits', async () => {
  // A transcript with roles from AssemblyAI was replaced without a word (finding 14).
  const transcribe = setup(false)
  await fireEvent.click(screen.getByRole('button', { name: 'STT' }))
  expect(await screen.findByRole('dialog')).toHaveTextContent(
    'Este audio ya tiene una transcripción. Si lo volvés a transcribir, se reemplaza; la actual, con sus nombres de hablante, queda en «Versiones anteriores» y la podés restaurar. ¿Continuar?'
  )
  expect(transcribe).not.toHaveBeenCalled()
  await fireEvent.click(screen.getByRole('button', { name: 'Transcribir de nuevo' }))
  expect(transcribe).toHaveBeenCalledTimes(1)
})

it('shows OCR progress as a progressbar carrying its value', () => {
  setup(false, false, { selectedAsset: imageAsset, ocrState: ocrState('running', 40) })
  const bar = screen.getByRole('progressbar', { name: 'Extracción de texto' })
  expect(bar).toHaveAttribute('aria-valuenow', '40')
  expect(document.querySelector('progress')).toBeNull()
})

it('shows transcription progress as a progressbar carrying its value', () => {
  setup(false, false, {
    transcriptionState: { status: 'running', progress: 25, text: '', editedByHand: false },
  })
  const bar = screen.getByRole('progressbar', { name: 'Transcripción de audio' })
  expect(bar).toHaveAttribute('aria-valuenow', '25')
})

it('keeps the extracted text collapsed behind a disclosure and reports edits', async () => {
  const onOcrTextInput = vi.fn()
  setup(false, false, {
    selectedAsset: imageAsset,
    ocrState: ocrState('done'),
    ocrEditedText: 'texto',
    onOcrTextInput,
  })
  const header = screen.getByRole('button', { name: /Texto extraído/ })
  expect(header).toHaveAttribute('aria-expanded', 'false')
  expect(screen.queryByRole('textbox')).toBeNull()
  await fireEvent.click(header)
  expect(header).toHaveAttribute('aria-expanded', 'true')
  const field = screen.getByRole('textbox')
  expect(field).toHaveValue('texto')
  await fireEvent.input(field, { target: { value: 'texto corregido' } })
  expect(onOcrTextInput).toHaveBeenCalledWith('image-1', 'texto corregido')
})

it('opens the recovered text of a failed OCR run and honours the editing lock', () => {
  setup(false, false, {
    selectedAsset: imageAsset,
    ocrState: ocrState('error'),
    ocrEditedText: 'parcial',
    ocrEditingDisabled: true,
  })
  expect(screen.getByRole('button', { name: /Texto extraído/ })).toHaveAttribute(
    'aria-expanded',
    'true'
  )
  expect(screen.getByRole('textbox')).toBeDisabled()
})

// R21: the OCRC button says where the text goes before it is sent.
it.each([
  [
    { available: true, local: true, tool: 'Qwen3-VL local (llama-server)', model: 'qwen.gguf' },
    'Corrección OCR · corre en este equipo con qwen.gguf',
  ],
  [
    { available: true, local: false, tool: 'OpenRouter API', model: 'google/gemini' },
    'Corrección OCR · envía el texto a OpenRouter API (google/gemini)',
  ],
])('names the OCRC engine in the button tooltip', (engine, title) => {
  setup(false, false, {
    selectedAsset: imageAsset,
    ocrState: ocrState('done'),
    ocrCorrectionEngine: engine,
  })
  expect(screen.getByRole('button', { name: 'OCRC' })).toHaveAttribute('data-tooltip', title)
})

it('reports transcription edits from the disclosed field', async () => {
  const onTranscriptionTextInput = vi.fn()
  setup(false, false, { onTranscriptionTextInput })
  await fireEvent.click(screen.getByRole('button', { name: /Transcripción/ }))
  const field = screen.getByRole('textbox')
  expect(field).toHaveValue('original')
  await fireEvent.input(field, { target: { value: 'corregido' } })
  expect(onTranscriptionTextInput).toHaveBeenCalledWith('audio-1', 'corregido')
})

it('offers the previous versions of a done transcript', async () => {
  // Finding 14: a replaced transcript stays recoverable from the panel.
  setup(false)
  vi.mocked(invoke).mockResolvedValue([])
  await fireEvent.click(screen.getByRole('button', { name: 'Versiones anteriores' }))
  expect(invoke).toHaveBeenCalledWith('list_transcription_versions', { assetId: 'audio-1' })
})

it('offers OCR of every page of a multi-page document, with the chosen engine', async () => {
  const onExtractAllPages = vi.fn()
  setup(false, false, {
    selectedAsset: imageAsset,
    assetsCount: 3,
    ocrState: ocrState('done'),
    localOcrAvailable: true,
    onExtractAllPages,
  })

  const all = screen.getByRole('group', { name: 'Todas las páginas' })
  await fireEvent.click(within(all).getByRole('button', { name: 'OCRH' }))
  expect(onExtractAllPages).toHaveBeenCalledWith('high')
  await fireEvent.click(within(all).getByRole('button', { name: 'OCRL' }))
  expect(onExtractAllPages).toHaveBeenLastCalledWith('light')
})

it('offers only OCRH for every page when local OCR is not available', () => {
  setup(false, false, {
    selectedAsset: imageAsset,
    assetsCount: 3,
    ocrState: ocrState('done'),
    onExtractAllPages: vi.fn(),
  })

  const all = screen.getByRole('group', { name: 'Todas las páginas' })
  expect(within(all).queryByRole('button', { name: 'OCRL' })).not.toBeInTheDocument()
  expect(within(all).getByRole('button', { name: 'OCRH' })).toBeInTheDocument()
})

it('says how many pages went to the queue', () => {
  setup(false, false, {
    selectedAsset: imageAsset,
    assetsCount: 3,
    ocrState: ocrState('done'),
    onExtractAllPages: vi.fn(),
    ocrAllPagesNotice: { queued: 2, skipped: 1 },
  })

  expect(screen.getByRole('status')).toHaveTextContent(
    'Páginas en cola: 2. Salteadas: 1 (ya tenían texto, estaban corregidas a mano o ya estaban en cola).'
  )
})

it('says when there was no page left to read', () => {
  setup(false, false, {
    selectedAsset: imageAsset,
    assetsCount: 3,
    ocrState: ocrState('done'),
    onExtractAllPages: vi.fn(),
    ocrAllPagesNotice: { queued: 0, skipped: 3 },
  })

  expect(screen.getByRole('status')).toHaveTextContent(
    'Todas las páginas ya tienen texto o están en cola.'
  )
})

it('does not offer every page for a single-page document', () => {
  setup(false, false, {
    selectedAsset: imageAsset,
    assetsCount: 1,
    ocrState: ocrState('done'),
    onExtractAllPages: vi.fn(),
  })

  expect(screen.queryByRole('group', { name: 'Todas las páginas' })).not.toBeInTheDocument()
})

it('closes the every-page notice', async () => {
  const onDismissOcrAllPagesNotice = vi.fn()
  setup(false, false, {
    selectedAsset: imageAsset,
    assetsCount: 3,
    ocrState: ocrState('done'),
    onExtractAllPages: vi.fn(),
    ocrAllPagesNotice: { queued: 2, skipped: 1 },
    onDismissOcrAllPagesNotice,
  })

  await fireEvent.click(screen.getByRole('button', { name: 'Cerrar aviso' }))
  expect(onDismissOcrAllPagesNotice).toHaveBeenCalledOnce()
})
