import type { Editor } from '@tiptap/core'

import {
  chooseDictationCaptureStrategy,
  encodeWavFromPcm,
  type DictationCaptureStrategy,
} from './dictation'

/**
 * Microphone dictation for a Tiptap editor: capture, the state machine, the
 * duration limit, and putting the transcription into the document.
 *
 * Editor-agnostic on purpose. NoteEditor and WritingEditor both dictate, and
 * the capture half is where the platform bugs live (WebKitGTK's MediaRecorder,
 * a stop event that beats its last chunk); one copy is what keeps a fix for
 * one editor from skipping the other.
 */

export type DictationState = 'idle' | 'recording' | 'transcribing' | 'error'
export type DictationLogLevel = 'info' | 'warn' | 'error'

export interface DictationLabels {
  dictationStart: string
  dictationStop: string
  dictationProcessing: string
  dictationNoMicrophone: string
  dictationNoAudio: string
  dictationAutoStopProcessing: string
  dictationTranscribing: string
  dictationAutoStopInserted: string
  dictationInserted: string
  dictationNoText: string
  dictationTranscriptionFailed: string
}

/**
 * Where the transcription goes.
 *
 * - `append-unless-caret` (NoteEditor): at the caret only when the editor held
 *   the focus at the start; otherwise, and for a caret still at the very start,
 *   appended to the end. A note is usually dictated onto the end of itself.
 * - `caret` (WritingEditor): at the editor's selection, replacing it, wherever
 *   the focus was. A manuscript is dictated into, not onto.
 */
export type DictationInsertion = 'append-unless-caret' | 'caret'

/**
 * Read through getters, so a component can hand over its props and the
 * controller always sees their current value.
 */
export interface DictationOptions {
  readonly editor: Editor | null | undefined
  readonly editorElement: HTMLElement | null | undefined
  readonly isEditorFocused: boolean
  readonly ondictate: ((audio: Blob) => Promise<string>) | undefined
  readonly onlog: ((level: DictationLogLevel, message: string) => void | Promise<void>) | undefined
  readonly maxSeconds: number
  readonly labels: DictationLabels
  /** Prefixes console output, e.g. `[NoteEditor/dictation]`. */
  readonly logPrefix: string
  readonly insertion: DictationInsertion
  /** Called after the text went in. */
  readonly oninserted?: (() => void) | undefined
}

type Range = { from: number; to: number }

export function formatDuration(totalSeconds: number) {
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  return `${minutes}:${seconds.toString().padStart(2, '0')}`
}

function withDuration(template: string, duration: string) {
  return template.replace('{duration}', duration)
}

function describeDictationError(error: unknown) {
  if (error instanceof Error) {
    return `${error.name}: ${error.message}`
  }

  return String(error)
}

function formatTrackDetails(track: MediaStreamTrack, index: number) {
  return `#${index} kind=${track.kind} readyState=${track.readyState} enabled=${track.enabled} muted=${track.muted}`
}

export function createDictation(options: DictationOptions) {
  let dictationState = $state<DictationState>('idle')
  let dictationSeconds = $state(0)
  let dictationMessage = $state<string | null>(null)
  let dictationAutoStopped = false
  let dictationStrategy: DictationCaptureStrategy | null = null
  let mediaRecorder: MediaRecorder | null = null
  let mediaStream: MediaStream | null = null
  let dictationAudioContext: AudioContext | null = null
  let dictationSourceNode: MediaStreamAudioSourceNode | null = null
  let dictationProcessorNode: ScriptProcessorNode | null = null
  let dictationTimer: ReturnType<typeof setInterval> | null = null
  let dictationChunks: Blob[] = []
  let dictationPcmChunks: Float32Array[] = []
  let dictationSampleRate = 0
  let dictationSelection: Range | null = null

  function logDictation(level: DictationLogLevel, message: string) {
    const formattedMessage = `${options.logPrefix} ${message}`

    if (level === 'error') {
      console.error(formattedMessage)
    } else if (level === 'warn') {
      console.warn(formattedMessage)
    } else {
      console.info(formattedMessage)
    }

    const onlog = options.onlog
    if (!onlog) return

    void Promise.resolve(onlog(level, message)).catch((error) => {
      console.error(`${options.logPrefix} Failed to forward dictation diagnostic log:`, error)
    })
  }

  function resetDictationTimer() {
    if (dictationTimer) {
      clearInterval(dictationTimer)
      dictationTimer = null
    }
    dictationSeconds = 0
  }

  function stopMediaStreamTracks() {
    mediaStream?.getTracks().forEach((track) => track.stop())
    mediaStream = null
  }

  async function teardownPcmDictation() {
    dictationProcessorNode?.disconnect()
    dictationSourceNode?.disconnect()
    dictationProcessorNode = null
    dictationSourceNode = null

    if (dictationAudioContext) {
      try {
        await dictationAudioContext.close()
      } catch (error) {
        logDictation('warn', `AudioContext close failed; error=${describeDictationError(error)}`)
      }
      dictationAudioContext = null
    }
  }

  function setDictationMessage(message: string | null, tone: 'idle' | 'error' = 'idle') {
    dictationMessage = message
    if (tone === 'error') {
      dictationState = 'error'
    }
  }

  async function resetDictationCaptureState() {
    await teardownPcmDictation()
    stopMediaStreamTracks()
    resetDictationTimer()
    dictationStrategy = null
    mediaRecorder = null
    dictationChunks = []
    dictationPcmChunks = []
    dictationSampleRate = 0
  }

  async function failDictationCapture(message: string, details: string) {
    logDictation('error', details)
    await resetDictationCaptureState()
    dictationAutoStopped = false
    setDictationMessage(message, 'error')
  }

  /** The range the text replaces, or null to append at the end of the document. */
  function insertionTarget(editor: Editor): Range | null {
    if (options.insertion === 'caret') {
      if (!dictationSelection) return null
      // The writer may have kept typing while the recording ran; a range that
      // now points past the document would make the insertion throw.
      const size = editor.state.doc.content.size
      const from = Math.min(dictationSelection.from, size)
      const to = Math.min(Math.max(dictationSelection.to, from), size)
      return { from, to }
    }

    const hasExplicitInsertionSelection = Boolean(
      dictationSelection && !(dictationSelection.from === 1 && dictationSelection.to === 1)
    )
    return hasExplicitInsertionSelection ? dictationSelection : null
  }

  function getDictationInsertionPlan(editor: Editor, target: Range | null, text: string) {
    const trimmed = text.trim()
    if (!trimmed) return { text: '', leadingSpace: false, trailingSpace: false }

    if (!target) {
      const currentText = editor.getText()
      const prevChar = currentText.slice(-1)
      return {
        text: trimmed,
        leadingSpace: Boolean(prevChar) && !/\s/.test(prevChar) && !/^[\s,.;:!?)]/.test(trimmed),
        trailingSpace: false,
      }
    }

    const { from, to } = target
    const prevChar = editor.state.doc.textBetween(Math.max(0, from - 1), from, '', '')
    const nextChar = editor.state.doc.textBetween(
      to,
      Math.min(editor.state.doc.content.size, to + 1),
      '',
      ''
    )

    const needsLeadingSpace =
      from > 1 && prevChar && !/\s/.test(prevChar) && !/^[\s,.;:!?)]/.test(trimmed)
    const needsTrailingSpace = nextChar && !/\s/.test(nextChar) && !/[\s([{]$/.test(trimmed)

    return {
      text: trimmed,
      leadingSpace: Boolean(needsLeadingSpace),
      trailingSpace: Boolean(needsTrailingSpace),
    }
  }

  function insertDictationText(text: string) {
    const editor = options.editor
    if (!editor) return

    const target = insertionTarget(editor)
    const insertion = getDictationInsertionPlan(editor, target, text)
    if (!insertion.text) return

    const insertionText = `${insertion.leadingSpace ? ' ' : ''}${insertion.text}${insertion.trailingSpace ? ' ' : ''}`

    logDictation(
      'info',
      target
        ? `inserting transcription at selection from=${target.from} to=${target.to}`
        : 'inserting transcription at document end'
    )

    // A text node rather than a string in the manuscript: a string is parsed as
    // HTML, and a transcription that says "a < b" is words, not markup.
    const content =
      options.insertion === 'caret' ? { type: 'text', text: insertionText } : insertionText

    if (target) {
      editor.chain().focus().insertContentAt({ from: target.from, to: target.to }, content).run()
    } else {
      const end = Math.max(1, editor.state.doc.content.size - 1)
      editor.chain().focus().insertContentAt({ from: end, to: end }, content).run()
    }
    options.oninserted?.()
  }

  async function finalizeCapturedAudio(audioBlob: Blob, details: string) {
    const wasAutoStopped = dictationAutoStopped

    await resetDictationCaptureState()

    logDictation(
      'info',
      `finalizing recording; ${details}; blobBytes=${audioBlob.size}; blobType=${audioBlob.type || 'unknown'}`
    )

    const ondictate = options.ondictate
    if (!ondictate || audioBlob.size === 0) {
      dictationState = 'idle'
      if (audioBlob.size === 0) {
        logDictation('warn', 'recording finished without usable audio data')
        setDictationMessage(options.labels.dictationNoAudio, 'error')
      }
      return
    }

    dictationState = 'transcribing'
    if (wasAutoStopped) {
      dictationMessage = withDuration(
        options.labels.dictationAutoStopProcessing,
        formatDuration(options.maxSeconds)
      )
    } else {
      dictationMessage = options.labels.dictationTranscribing
    }

    try {
      logDictation('info', `transcription callback started; blobBytes=${audioBlob.size}`)
      const text = (await ondictate(audioBlob)).trim()
      logDictation('info', `transcription callback resolved; textLength=${text.length}`)
      if (text) {
        insertDictationText(text)
        dictationMessage = wasAutoStopped
          ? withDuration(
              options.labels.dictationAutoStopInserted,
              formatDuration(options.maxSeconds)
            )
          : options.labels.dictationInserted
        dictationState = 'idle'
      } else {
        setDictationMessage(options.labels.dictationNoText, 'error')
      }
    } catch (error) {
      logDictation('error', `transcription callback failed; error=${describeDictationError(error)}`)
      setDictationMessage(
        error instanceof Error ? error.message : options.labels.dictationTranscriptionFailed,
        'error'
      )
    } finally {
      dictationAutoStopped = false
    }
  }

  async function finalizeMediaRecorderDictation() {
    const recorder = mediaRecorder
    const chunkCount = dictationChunks.length
    const accumulatedChunkBytes = dictationChunks.reduce((total, chunk) => total + chunk.size, 0)
    const audioBlob = new Blob(dictationChunks, {
      type: recorder?.mimeType || 'audio/webm',
    })

    await finalizeCapturedAudio(
      audioBlob,
      `strategy=media-recorder; chunks=${chunkCount}; accumulatedBytes=${accumulatedChunkBytes}`
    )
  }

  async function finalizePcmDictation() {
    const sampleCount = dictationPcmChunks.reduce((total, chunk) => total + chunk.length, 0)
    const sampleRate = dictationSampleRate || dictationAudioContext?.sampleRate || 44100

    await teardownPcmDictation()

    const audioBlob =
      sampleCount > 0
        ? encodeWavFromPcm(dictationPcmChunks, sampleRate)
        : new Blob([], { type: 'audio/wav' })
    await finalizeCapturedAudio(
      audioBlob,
      `strategy=pcm-wav; chunks=${dictationPcmChunks.length}; sampleCount=${sampleCount}; sampleRate=${sampleRate}`
    )
  }

  async function stopDictation(stopOptions?: { autoStop?: boolean }) {
    dictationAutoStopped = stopOptions?.autoStop ?? false

    if (dictationStrategy === 'pcm-wav') {
      logDictation(
        'info',
        `stop requested; autoStop=${dictationAutoStopped}; strategy=pcm-wav; currentChunks=${dictationPcmChunks.length}`
      )
      await finalizePcmDictation()
      return
    }

    const recorder = mediaRecorder
    if (!recorder || recorder.state !== 'recording') return

    logDictation(
      'info',
      `stop requested; autoStop=${dictationAutoStopped}; strategy=media-recorder; state=${recorder.state}; currentChunks=${dictationChunks.length}`
    )
    const processing = new Promise<void>((resolve) => {
      const previousOnStop = recorder.onstop
      recorder.onstop = (event) => {
        previousOnStop?.call(recorder, event)
        setTimeout(() => {
          void finalizeMediaRecorderDictation().finally(resolve)
        }, 0)
      }
    })
    recorder.requestData()
    recorder.stop()
    await processing
  }

  function captureSelection(): Range | null {
    const editor = options.editor
    if (!editor) return null

    if (options.insertion === 'caret') {
      return { from: editor.state.selection.from, to: editor.state.selection.to }
    }

    const currentEditorElement = options.editorElement
    const hasActiveEditorSelection =
      currentEditorElement != null &&
      typeof document !== 'undefined' &&
      document.activeElement instanceof Node &&
      currentEditorElement.contains(document.activeElement)
    return options.isEditorFocused && hasActiveEditorSelection
      ? { from: editor.state.selection.from, to: editor.state.selection.to }
      : null
  }

  async function startDictation() {
    if (!options.ondictate) return

    logDictation('info', 'dictation start requested')

    if (
      typeof window === 'undefined' ||
      typeof navigator === 'undefined' ||
      !navigator.mediaDevices?.getUserMedia
    ) {
      logDictation('warn', 'microphone APIs are unavailable in this runtime')
      setDictationMessage(options.labels.dictationNoMicrophone, 'error')
      return
    }

    try {
      dictationChunks = []
      dictationMessage = null
      dictationAutoStopped = false
      dictationSelection = captureSelection()
      logDictation('info', 'requesting microphone access via getUserMedia')
      const stream = await navigator.mediaDevices.getUserMedia({ audio: true })
      mediaStream = stream
      const tracks = stream.getTracks()
      const trackDetails = tracks.map((track, index) => formatTrackDetails(track, index)).join('; ')
      logDictation(
        'info',
        `getUserMedia succeeded; tracks=${tracks.length}${trackDetails ? `; ${trackDetails}` : ''}`
      )

      const audioContextConstructor =
        window.AudioContext ??
        (window as Window & { webkitAudioContext?: typeof AudioContext }).webkitAudioContext
      const strategy = chooseDictationCaptureStrategy({
        userAgent: navigator.userAgent,
        platform: navigator.platform,
        hasMediaRecorder: typeof MediaRecorder !== 'undefined',
        hasAudioContext: typeof audioContextConstructor === 'function',
        hasTauriRuntime: '__TAURI_INTERNALS__' in window,
      })

      if (!strategy) {
        logDictation('warn', 'microphone capture APIs are unavailable after getUserMedia')
        stopMediaStreamTracks()
        setDictationMessage(options.labels.dictationNoMicrophone, 'error')
        return
      }

      dictationStrategy = strategy
      logDictation(
        'info',
        `selected capture strategy=${strategy}; userAgent=${navigator.userAgent}`
      )

      if (strategy === 'pcm-wav') {
        if (!audioContextConstructor) {
          throw new Error('AudioContext is not available for WAV dictation fallback.')
        }

        dictationPcmChunks = []
        const audioContext = new audioContextConstructor()
        const source = audioContext.createMediaStreamSource(stream)
        const processor = audioContext.createScriptProcessor(4096, 1, 1)
        const silentOutput = audioContext.createGain()
        silentOutput.gain.value = 0

        processor.onaudioprocess = (event) => {
          const input = event.inputBuffer.getChannelData(0)
          const chunk = new Float32Array(input.length)
          chunk.set(input)
          dictationPcmChunks = [...dictationPcmChunks, chunk]
        }

        source.connect(processor)
        processor.connect(silentOutput)
        silentOutput.connect(audioContext.destination)

        if (audioContext.state === 'suspended') {
          await audioContext.resume()
        }

        dictationAudioContext = audioContext
        dictationSourceNode = source
        dictationProcessorNode = processor
        dictationSampleRate = audioContext.sampleRate
        logDictation(
          'info',
          `PCM/WAV fallback started; sampleRate=${audioContext.sampleRate}; bufferSize=4096`
        )
        dictationState = 'recording'
        dictationSeconds = 0
      } else {
        const preferredTypes = [
          'audio/webm;codecs=opus',
          'audio/webm',
          'audio/ogg;codecs=opus',
          'audio/ogg',
        ]
        const mimeType =
          typeof MediaRecorder.isTypeSupported === 'function'
            ? preferredTypes.find((t) => MediaRecorder.isTypeSupported(t))
            : undefined

        logDictation(
          'info',
          `selected recorder mimeType=${mimeType ?? 'browser-default'}; preferredTypes=${preferredTypes.join(',')}`
        )

        const recorder = mimeType
          ? new MediaRecorder(stream, { mimeType })
          : new MediaRecorder(stream)
        let accumulatedChunkBytes = 0

        recorder.onstart = () => {
          logDictation(
            'info',
            `MediaRecorder start event; state=${recorder.state}; mimeType=${recorder.mimeType || mimeType || 'unknown'}`
          )
        }
        recorder.ondataavailable = (event) => {
          accumulatedChunkBytes += event.data.size
          logDictation(
            event.data.size > 0 ? 'info' : 'warn',
            `MediaRecorder dataavailable; chunkBytes=${event.data.size}; accumulatedBytes=${accumulatedChunkBytes}`
          )
          if (event.data.size > 0) {
            dictationChunks = [...dictationChunks, event.data]
          }
        }
        recorder.onstop = () => {
          logDictation(
            'info',
            `MediaRecorder stop event; state=${recorder.state}; accumulatedBytes=${accumulatedChunkBytes}`
          )
        }
        recorder.onerror = (event) => {
          const error = 'error' in event ? event.error : undefined
          void failDictationCapture(
            error instanceof Error && error.message.trim()
              ? error.message
              : options.labels.dictationTranscriptionFailed,
            `MediaRecorder error event; error=${error ? describeDictationError(error) : 'unknown'}`
          )
        }
        mediaRecorder = recorder
        dictationState = 'recording'
        dictationSeconds = 0
        recorder.start(1000)
      }

      dictationTimer = setInterval(() => {
        dictationSeconds += 1
        if (dictationSeconds >= options.maxSeconds) {
          logDictation(
            'info',
            `dictation reached max duration=${options.maxSeconds}s; auto-stopping`
          )
          void stopDictation({ autoStop: true })
        }
      }, 1000)
    } catch (error) {
      await failDictationCapture(
        error instanceof Error ? error.message : options.labels.dictationNoMicrophone,
        `getUserMedia failed; error=${describeDictationError(error)}`
      )
    }
  }

  async function toggle() {
    if (dictationState === 'transcribing') return
    if (dictationState === 'recording') {
      await stopDictation()
      return
    }
    await startDictation()
  }

  /** Releases the microphone; call from the component's onDestroy. */
  function destroy() {
    resetDictationTimer()
    if (mediaRecorder?.state === 'recording') {
      mediaRecorder.stop()
    }
    void teardownPcmDictation()
    stopMediaStreamTracks()
  }

  return {
    get state() {
      return dictationState
    },
    get message() {
      return dictationMessage
    },
    /** What the microphone button is called in the current state. */
    get buttonLabel() {
      if (dictationState === 'recording') return options.labels.dictationStop
      if (dictationState === 'transcribing') return options.labels.dictationProcessing
      return options.labels.dictationStart
    },
    /** Elapsed over the limit, e.g. `0:12 / 5:00`. */
    get timerLabel() {
      return `${formatDuration(dictationSeconds)} / ${formatDuration(options.maxSeconds)}`
    },
    toggle,
    destroy,
  }
}

export type DictationController = ReturnType<typeof createDictation>
