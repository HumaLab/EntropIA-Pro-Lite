import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import NoteEditor from '../NoteEditor.svelte'

class FakeMediaRecorder {
  static instances: FakeMediaRecorder[] = []
  static finalChunkParts: BlobPart[] = ['audio']
  static isTypeSupported = vi.fn((mimeType: string) => mimeType === 'audio/webm;codecs=opus')
  static emitStopBeforeData = false

  public state: 'inactive' | 'recording' = 'inactive'
  public mimeType = 'audio/webm'
  public onstart: (() => void) | null = null
  public ondataavailable: ((event: { data: Blob }) => void) | null = null
  public onstop: (() => void) | null = null
  public onerror: ((event: Event & { error?: DOMException }) => void) | null = null
  public stream: MediaStream

  constructor(stream: MediaStream, options?: { mimeType?: string }) {
    this.stream = stream
    this.mimeType = options?.mimeType ?? 'audio/webm'
    FakeMediaRecorder.instances.push(this)
  }

  start(_timeslice?: number) {
    this.state = 'recording'
    this.onstart?.()
  }

  stop() {
    this.state = 'inactive'
    const emitData = () => {
      this.ondataavailable?.({
        data: new Blob(FakeMediaRecorder.finalChunkParts, { type: this.mimeType }),
      })
    }

    if (FakeMediaRecorder.emitStopBeforeData) {
      this.onstop?.()
      queueMicrotask(emitData)
      return
    }

    emitData()
    this.onstop?.()
  }

  requestData() {
    // no-op: test data is delivered on stop()
  }

  emitError(error?: DOMException) {
    this.state = 'inactive'
    this.onerror?.(Object.assign(new Event('error'), { error }) as Event & { error?: DOMException })
  }
}

class FakeScriptProcessorNode {
  public onaudioprocess:
    | ((event: { inputBuffer: { getChannelData: (channel: number) => Float32Array } }) => void)
    | null = null

  public connect = vi.fn()
  public disconnect = vi.fn()

  emit(samples: number[]) {
    this.onaudioprocess?.({
      inputBuffer: {
        getChannelData: () => Float32Array.from(samples),
      },
    })
  }
}

class FakeAudioContext {
  static instances: FakeAudioContext[] = []

  public state: 'running' | 'suspended' = 'running'
  public sampleRate = 16000
  public destination = {} as AudioDestinationNode
  public lastProcessor: FakeScriptProcessorNode | null = null
  public lastSource = {
    connect: vi.fn(),
    disconnect: vi.fn(),
  }
  public lastGain = {
    gain: { value: 1 },
    connect: vi.fn(),
    disconnect: vi.fn(),
  }

  constructor() {
    FakeAudioContext.instances.push(this)
  }

  createMediaStreamSource() {
    return this.lastSource as unknown as MediaStreamAudioSourceNode
  }

  createScriptProcessor() {
    const processor = new FakeScriptProcessorNode()
    this.lastProcessor = processor
    return processor as unknown as ScriptProcessorNode
  }

  createGain() {
    return this.lastGain as unknown as GainNode
  }

  resume = vi.fn(async () => {
    this.state = 'running'
  })

  close = vi.fn(async () => {})
}

describe('NoteEditor dictation', () => {
  const getUserMediaMock = vi.fn<() => Promise<MediaStream>>()
  const stopTrackMock = vi.fn()

  beforeEach(() => {
    FakeMediaRecorder.instances = []
    FakeMediaRecorder.finalChunkParts = ['audio']
    FakeMediaRecorder.isTypeSupported.mockClear()
    FakeMediaRecorder.emitStopBeforeData = false
    FakeAudioContext.instances = []
    stopTrackMock.mockReset()
    getUserMediaMock.mockReset()

    Object.defineProperty(globalThis, 'MediaRecorder', {
      configurable: true,
      value: FakeMediaRecorder,
    })

    Object.defineProperty(globalThis.navigator, 'mediaDevices', {
      configurable: true,
      value: {
        getUserMedia: getUserMediaMock,
      },
    })

    getUserMediaMock.mockResolvedValue({
      getTracks: () => [{ stop: stopTrackMock }],
    } as unknown as MediaStream)

    Object.defineProperty(window, 'AudioContext', {
      configurable: true,
      value: undefined,
    })

    Object.defineProperty(globalThis.navigator, 'userAgent', {
      configurable: true,
      value: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/126.0.0.0 Safari/537.36',
    })
    Object.defineProperty(globalThis.navigator, 'platform', {
      configurable: true,
      value: 'MacIntel',
    })

    Reflect.deleteProperty(window, '__TAURI_INTERNALS__')
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('renders the microphone button only when dictation is enabled', () => {
    const { rerender } = render(NoteEditor, { props: {} })
    expect(screen.queryByRole('button', { name: 'Start dictation' })).not.toBeInTheDocument()

    rerender({ ondictate: vi.fn() })
    expect(screen.getByRole('button', { name: 'Start dictation' })).toBeInTheDocument()
  })

  it('shows a non intrusive message when microphone APIs are unavailable', async () => {
    Object.defineProperty(globalThis, 'MediaRecorder', {
      configurable: true,
      value: undefined,
    })

    render(NoteEditor, { props: { ondictate: vi.fn() } })

    await fireEvent.click(screen.getByRole('button', { name: 'Start dictation' }))

    expect(screen.getByTestId('note-editor-dictation-message')).toHaveTextContent(
      'Microphone is not available on this device.'
    )
  })

  it('records, transcribes, and appends the text when no cursor selection is active', async () => {
    const ondictate = vi.fn().mockResolvedValue('texto dictado')
    render(NoteEditor, { props: { ondictate, content: '<p>Hola </p>' } })

    await fireEvent.click(screen.getByRole('button', { name: 'Start dictation' }))

    expect(screen.getByRole('button', { name: 'Stop dictation' })).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Stop dictation' }))

    await waitFor(() => {
      expect(ondictate).toHaveBeenCalledOnce()
      expect(screen.getByRole('textbox')).toHaveTextContent('Hola texto dictado')
    })

    expect(stopTrackMock).toHaveBeenCalledOnce()
  })

  it('waits for the final audio chunk when stop fires before dataavailable', async () => {
    FakeMediaRecorder.emitStopBeforeData = true
    const ondictate = vi.fn().mockResolvedValue('late chunk text')
    render(NoteEditor, { props: { ondictate, content: '<p>Hello </p>' } })

    await fireEvent.click(screen.getByRole('button', { name: 'Start dictation' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Stop dictation' }))

    await waitFor(() => {
      expect(ondictate).toHaveBeenCalledOnce()
    })

    expect(ondictate.mock.calls[0]?.[0]).toBeInstanceOf(Blob)
    expect((ondictate.mock.calls[0]?.[0] as Blob).size).toBeGreaterThan(0)

    expect(screen.queryByTestId('note-editor-dictation-message')).not.toHaveTextContent(
      'Could not capture audio from the microphone.'
    )
  })

  it('emits granular dictation diagnostics through the optional callback', async () => {
    const ondictationlog = vi.fn()
    render(NoteEditor, { props: { ondictate: vi.fn().mockResolvedValue('logged text'), ondictationlog } })

    await fireEvent.click(screen.getByRole('button', { name: 'Start dictation' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Stop dictation' }))

    await waitFor(() => {
      expect(ondictationlog).toHaveBeenCalledWith(
        'info',
        expect.stringContaining('getUserMedia succeeded; tracks=1')
      )
      expect(ondictationlog).toHaveBeenCalledWith(
        'info',
        expect.stringContaining('selected recorder mimeType=audio/webm;codecs=opus')
      )
      expect(ondictationlog).toHaveBeenCalledWith(
        'info',
        expect.stringContaining('MediaRecorder start event; state=recording')
      )
      expect(ondictationlog).toHaveBeenCalledWith(
        'info',
        expect.stringContaining('MediaRecorder dataavailable; chunkBytes=5; accumulatedBytes=5')
      )
      expect(ondictationlog).toHaveBeenCalledWith(
        'info',
        expect.stringContaining(
          'finalizing recording; strategy=media-recorder; chunks=1; accumulatedBytes=5; blobBytes=5'
        )
      )
    })
  })

  it('uses the PCM/WAV fallback on Linux WebKitGTK/Tauri runtimes', async () => {
    Object.defineProperty(window, 'AudioContext', {
      configurable: true,
      value: FakeAudioContext,
    })
    Object.defineProperty(window, '__TAURI_INTERNALS__', {
      configurable: true,
      value: {},
    })
    Object.defineProperty(globalThis.navigator, 'userAgent', {
      configurable: true,
      value: 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 Safari/605.1.15',
    })
    Object.defineProperty(globalThis.navigator, 'platform', {
      configurable: true,
      value: 'Linux x86_64',
    })

    const ondictate = vi.fn().mockResolvedValue('wav fallback text')
    const ondictationlog = vi.fn()
    render(NoteEditor, { props: { ondictate, ondictationlog } })

    await fireEvent.click(screen.getByRole('button', { name: 'Start dictation' }))

    FakeAudioContext.instances[0]?.lastProcessor?.emit([0.2, -0.2, 0.1, 0])

    await fireEvent.click(screen.getByRole('button', { name: 'Stop dictation' }))

    await waitFor(() => {
      expect(ondictate).toHaveBeenCalledOnce()
      expect(screen.getByRole('textbox')).toHaveTextContent('wav fallback text')
    })

    const audioBlob = ondictate.mock.calls[0]?.[0] as Blob
    expect(audioBlob.type).toBe('audio/wav')
    expect(audioBlob.size).toBeGreaterThan(44)
    expect(FakeMediaRecorder.instances).toHaveLength(0)
    expect(ondictationlog).toHaveBeenCalledWith(
      'info',
      expect.stringContaining('selected capture strategy=pcm-wav')
    )
    expect(ondictationlog).toHaveBeenCalledWith(
      'info',
      expect.stringContaining('PCM/WAV fallback started; sampleRate=16000')
    )
  })

  it('shows a generic no-audio message when recording stops without usable audio', async () => {
    FakeMediaRecorder.finalChunkParts = []

    render(NoteEditor, { props: { ondictate: vi.fn() } })

    await fireEvent.click(screen.getByRole('button', { name: 'Start dictation' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Stop dictation' }))

    await waitFor(() => {
      expect(screen.getByTestId('note-editor-dictation-message')).toHaveTextContent(
        'Could not capture audio from the microphone.'
      )
    })

    expect(screen.getByTestId('note-editor-dictation-message')).not.toHaveTextContent('GStreamer')
    expect(screen.getByTestId('note-editor-dictation-message')).not.toHaveTextContent('Linux')
  })

  it('does not transcribe a WAV fallback recording when no PCM samples were captured', async () => {
    Object.defineProperty(window, 'AudioContext', {
      configurable: true,
      value: FakeAudioContext,
    })
    Object.defineProperty(window, '__TAURI_INTERNALS__', {
      configurable: true,
      value: {},
    })
    Object.defineProperty(globalThis.navigator, 'userAgent', {
      configurable: true,
      value: 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 Safari/605.1.15',
    })
    Object.defineProperty(globalThis.navigator, 'platform', {
      configurable: true,
      value: 'Linux x86_64',
    })

    const ondictate = vi.fn().mockResolvedValue('should not be used')
    render(NoteEditor, { props: { ondictate } })

    await fireEvent.click(screen.getByRole('button', { name: 'Start dictation' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Stop dictation' }))

    await waitFor(() => {
      expect(screen.getByTestId('note-editor-dictation-message')).toHaveTextContent(
        'Could not capture audio from the microphone.'
      )
    })

    expect(ondictate).not.toHaveBeenCalled()
  })

  it('cleans up recorder state when MediaRecorder errors at runtime', async () => {
    render(NoteEditor, { props: { ondictate: vi.fn() } })

    await fireEvent.click(screen.getByRole('button', { name: 'Start dictation' }))

    FakeMediaRecorder.instances[0]?.emitError(new DOMException('Recorder failed', 'InvalidStateError'))

    await waitFor(() => {
      expect(screen.getByTestId('note-editor-dictation-message')).toHaveTextContent('Recorder failed')
    })

    expect(stopTrackMock).toHaveBeenCalledOnce()
    expect(screen.getByRole('button', { name: 'Start dictation' })).toBeInTheDocument()
  })

  it('adds spacing when dictation is inserted after a word without trailing whitespace', async () => {
    const ondictate = vi.fn().mockResolvedValue('texto dictado')
    render(NoteEditor, { props: { ondictate, content: '<p>Hola</p>' } })

    await fireEvent.click(screen.getByRole('button', { name: 'Start dictation' }))
    await fireEvent.click(screen.getByRole('button', { name: 'Stop dictation' }))

    await waitFor(() => {
      expect(screen.getByRole('textbox')).toHaveTextContent('Hola texto dictado')
    })
  })

  it('auto stops at the configured limit and shows a brief message', async () => {
    vi.useFakeTimers()
    const ondictate = vi.fn().mockResolvedValue('texto automático')
    render(NoteEditor, { props: { ondictate, dictationMaxSeconds: 2 } })

    await fireEvent.click(screen.getByRole('button', { name: 'Start dictation' }))

    await vi.advanceTimersByTimeAsync(2100)

    await waitFor(() => {
      expect(ondictate).toHaveBeenCalledOnce()
      expect(screen.getByTestId('note-editor-dictation-message')).toHaveTextContent(
        'Reached the maximum of 0:02. Text inserted.'
      )
    })
  })
})
