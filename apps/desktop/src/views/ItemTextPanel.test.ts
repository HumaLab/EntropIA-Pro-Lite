import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
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

function setup(edited = false, pendingEdit = false) {
  vi.mocked(invoke).mockResolvedValue(edited)
  const onTranscribeAudio = vi.fn()
  const props: ComponentProps<typeof ItemTextPanel> = {
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
    ocrCorrectionAvailable: false,
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
  }
  render(ItemTextPanel, props)
  return onTranscribeAudio
}

it('asks before replacing edited text and cancel does not transcribe', async () => {
  const transcribe = setup(true)
  await fireEvent.click(screen.getByRole('button', { name: 'STT' }))
  expect(await screen.findByRole('dialog')).toHaveTextContent(
    'La transcripción tiene correcciones hechas a mano. Si la volvés a transcribir, se reemplazan. ¿Continuar?'
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

it('preserves direct transcription when not edited', async () => {
  const transcribe = setup(false)
  await fireEvent.click(screen.getByRole('button', { name: 'STT' }))
  await waitFor(() => expect(transcribe).toHaveBeenCalledTimes(1))
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
})
