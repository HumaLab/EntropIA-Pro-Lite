<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte'
  import { Editor } from '@tiptap/core'
  import StarterKit from '@tiptap/starter-kit'
  import Underline from '@tiptap/extension-underline'
  import Link from '@tiptap/extension-link'
  import Placeholder from '@tiptap/extension-placeholder'
  import ActionIcon from '../Button/ActionIcon.svelte'
  import type { ActionIconName } from '../Button/ActionIcon.types'

  import type {
    NoteEditorLabels,
    NoteEditorProps,
  } from './NoteEditor.types'
  import {
    chooseDictationCaptureStrategy,
    encodeWavFromPcm,
    type DictationCaptureStrategy,
  } from './dictation'
  import {
    normalizeNoteContentForEditor,
    normalizeNoteContentForRender,
    sanitizeNoteHtml,
    shouldDisableNoteEditorSave,
  } from './note-content'

  let {
    content = '',
    placeholder = '',
    onsave,
    oncancel,
    ondictate,
    ondictationlog,
    dictationMaxSeconds = 300,
    clearOnSave = true,
    saveLabel = 'Save',
    cancelLabel = 'Cancel',
    labels: labelsProp = {},
  }: NoteEditorProps = $props()

  const defaultLabels: NoteEditorLabels = {
    toolbarAriaLabel: 'Formatting toolbar',
    textStyleGroup: 'Text style',
    structureGroup: 'Structure',
    insertGroup: 'Insert',
    dictationGroup: 'Dictation',
    bold: 'Bold',
    italic: 'Italic',
    underline: 'Underline',
    inlineCode: 'Inline code',
    heading1: 'Heading 1',
    heading2: 'Heading 2',
    heading3: 'Heading 3',
    bulletList: 'Bullet list',
    orderedList: 'Ordered list',
    quote: 'Quote',
    addLink: 'Add link',
    removeLink: 'Remove link',
    dictationStart: 'Start dictation',
    dictationStop: 'Stop dictation',
    dictationProcessing: 'Processing dictation...',
    helperText: 'Tip: select text to apply formatting or links.',
    dictationNoMicrophone: 'Microphone is not available on this device.',
    dictationNoAudio: 'Could not capture audio from the microphone.',
    dictationAutoStopProcessing: 'Reached the maximum of {duration}. Processing audio...',
    dictationTranscribing: 'Transcribing audio...',
    dictationAutoStopInserted: 'Reached the maximum of {duration}. Text inserted.',
    dictationInserted: 'Text inserted from the microphone.',
    dictationNoText: 'No text was detected in the audio.',
    dictationTranscriptionFailed: 'Could not transcribe the audio.',
    linkInvalidUrl: 'Enter a valid URL.',
    linkInvalidHttp: 'Use a valid http or https URL.',
    linkInvalidExample: 'Enter a valid URL, for example https://entropia.app.',
    linkModalTitle: 'Insert link',
    linkModalDescription: 'Paste a valid URL for the selected text.',
    linkUrlLabel: 'URL',
    linkPlaceholder: 'https://...',
    linkCancel: 'Cancel',
    linkSubmit: 'Insert',
  }

  const labels = $derived({ ...defaultLabels, ...labelsProp })

  let editorElement: HTMLDivElement | undefined = $state(undefined)
  let linkInputElement: HTMLInputElement | undefined = $state(undefined)
  let editor = $state<Editor | null>(null)
  let editorRevision = $state(0)
  let currentHtml = $state('<p></p>')
  let originalHtml = $state('<p></p>')
  let lastExternalHtml = $state('')
  let isFocused = $state(false)
  let dictationState = $state<'idle' | 'recording' | 'transcribing' | 'error'>('idle')
  let dictationSeconds = $state(0)
  let dictationMessage = $state<string | null>(null)
  let dictationAutoStopped = $state(false)
  let dictationStrategy = $state<DictationCaptureStrategy | null>(null)
  let mediaRecorder = $state<MediaRecorder | null>(null)
  let mediaStream = $state<MediaStream | null>(null)
  let dictationAudioContext = $state<AudioContext | null>(null)
  let dictationSourceNode = $state<MediaStreamAudioSourceNode | null>(null)
  let dictationProcessorNode = $state<ScriptProcessorNode | null>(null)
  let dictationTimer = $state<ReturnType<typeof setInterval> | null>(null)
  let dictationChunks = $state<Blob[]>([])
  let dictationPcmChunks = $state<Float32Array[]>([])
  let dictationSampleRate = $state(0)
  let dictationSelection = $state<{ from: number; to: number } | null>(null)
  let isLinkModalOpen = $state(false)
  let linkDraftHref = $state('')
  let linkModalError = $state<string | null>(null)
  let linkSelection = $state<{ from: number; to: number } | null>(null)

  const showCancel = $derived(typeof oncancel === 'function')
  const supportsDictation = $derived(typeof ondictate === 'function')
  const isEditing = $derived(showCancel || !clearOnSave)
  const isSaveDisabled = $derived(
    shouldDisableNoteEditorSave({
      currentContent: currentHtml,
      originalContent: originalHtml,
      isEditing,
    })
  )

  const dictationButtonLabel = $derived.by(() => {
    if (dictationState === 'recording') return labels.dictationStop
    if (dictationState === 'transcribing') return labels.dictationProcessing
    return labels.dictationStart
  })

  const dictationTimerLabel = $derived(formatDuration(dictationSeconds))

  const linkModalTitleId = 'note-editor-link-modal-title'
  const linkModalDescriptionId = 'note-editor-link-modal-description'
  const linkModalErrorId = 'note-editor-link-modal-error'

  type DictationLogLevel = 'info' | 'warn' | 'error'

  type ToolbarButton = {
    label: string
    icon: ActionIconName
    isActive: () => boolean
    action: () => void
  }

  type ToolbarGroup = {
    label: string
    buttons: ToolbarButton[]
  }

  const toolbarGroups = $derived.by<ToolbarGroup[]>(() => [
    {
      label: labels.textStyleGroup,
      buttons: [
        {
          label: labels.bold,
          icon: 'bold',
          isActive: () => editor?.isActive('bold') ?? false,
          action: () => editor?.chain().focus().toggleBold().run(),
        },
        {
          label: labels.italic,
          icon: 'italic',
          isActive: () => editor?.isActive('italic') ?? false,
          action: () => editor?.chain().focus().toggleItalic().run(),
        },
        {
          label: labels.underline,
          icon: 'underline',
          isActive: () => editor?.isActive('underline') ?? false,
          action: () => editor?.chain().focus().toggleUnderline().run(),
        },
        {
          label: labels.inlineCode,
          icon: 'code',
          isActive: () => editor?.isActive('code') ?? false,
          action: () => editor?.chain().focus().toggleCode().run(),
        },
      ],
    },
    {
      label: labels.structureGroup,
      buttons: [
        {
          label: labels.heading1,
          icon: 'heading-1',
          isActive: () => editor?.isActive('heading', { level: 1 }) ?? false,
          action: () => editor?.chain().focus().toggleHeading({ level: 1 }).run(),
        },
        {
          label: labels.heading2,
          icon: 'heading-2',
          isActive: () => editor?.isActive('heading', { level: 2 }) ?? false,
          action: () => editor?.chain().focus().toggleHeading({ level: 2 }).run(),
        },
        {
          label: labels.heading3,
          icon: 'heading-3',
          isActive: () => editor?.isActive('heading', { level: 3 }) ?? false,
          action: () => editor?.chain().focus().toggleHeading({ level: 3 }).run(),
        },
        {
          label: labels.bulletList,
          icon: 'list',
          isActive: () => editor?.isActive('bulletList') ?? false,
          action: () => editor?.chain().focus().toggleBulletList().run(),
        },
        {
          label: labels.orderedList,
          icon: 'list-ordered',
          isActive: () => editor?.isActive('orderedList') ?? false,
          action: () => editor?.chain().focus().toggleOrderedList().run(),
        },
        {
          label: labels.quote,
          icon: 'text-quote',
          isActive: () => editor?.isActive('blockquote') ?? false,
          action: () => editor?.chain().focus().toggleBlockquote().run(),
        },
      ],
    },
    {
      label: labels.insertGroup,
      buttons: [
        {
          label: labels.addLink,
          icon: 'link',
          isActive: () => editor?.isActive('link') ?? false,
          action: () => updateLink(),
        },
        {
          label: labels.removeLink,
          icon: 'unlink',
          isActive: () => false,
          action: () => removeLink(),
        },
      ],
    },
  ])

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

  function logDictation(level: DictationLogLevel, message: string) {
    const formattedMessage = `[NoteEditor/dictation] ${message}`

    if (level === 'error') {
      console.error(formattedMessage)
    } else if (level === 'warn') {
      console.warn(formattedMessage)
    } else {
      console.info(formattedMessage)
    }

    if (!ondictationlog) return

    void Promise.resolve(ondictationlog(level, message)).catch((error) => {
      console.error(
        '[NoteEditor/dictation] Failed to forward dictation diagnostic log:',
        error
      )
    })
  }

  function bumpEditorRevision() {
    editorRevision += 1
  }

  function syncEditorState(nextHtml: string) {
    currentHtml = nextHtml || '<p></p>'
    bumpEditorRevision()
  }

  function buildEditor() {
    if (!editorElement) return

    const instance = new Editor({
      element: editorElement,
      extensions: [
        StarterKit.configure({
          heading: { levels: [1, 2, 3] },
        }),
        Underline,
        Link.configure({
          openOnClick: false,
          autolink: false,
          HTMLAttributes: {
            rel: 'noopener noreferrer nofollow',
            target: '_blank',
          },
        }),
        Placeholder.configure({ placeholder }),
      ],
      content: currentHtml,
      autofocus: false,
      editorProps: {
        attributes: {
          class: 'note-editor__content ProseMirror',
          role: 'textbox',
          'aria-multiline': 'true',
          'aria-placeholder': placeholder,
          'data-testid': 'note-editor-input',
        },
      },
      onCreate: ({ editor }: { editor: Editor }) => {
        dictationSelection = {
          from: editor.state.selection.from,
          to: editor.state.selection.to,
        }
        syncEditorState(sanitizeNoteHtml(editor.getHTML()) || '<p></p>')
      },
      onUpdate: ({ editor }: { editor: Editor }) => {
        syncEditorState(sanitizeNoteHtml(editor.getHTML()) || '<p></p>')
      },
      onSelectionUpdate: ({ editor }: { editor: Editor }) => {
        if (dictationState === 'idle' || dictationState === 'error') {
          dictationSelection = {
            from: editor.state.selection.from,
            to: editor.state.selection.to,
          }
        }
        bumpEditorRevision()
      },
      onFocus: () => {
        isFocused = true
        bumpEditorRevision()
      },
      onBlur: () => {
        isFocused = false
        bumpEditorRevision()
      },
    })

    editor = instance
  }

  function formatDuration(totalSeconds: number) {
    const minutes = Math.floor(totalSeconds / 60)
    const seconds = totalSeconds % 60
    return `${minutes}:${seconds.toString().padStart(2, '0')}`
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

  function getDictationInsertionPlan(text: string) {
    if (!editor) {
      return { text: text.trim(), leadingSpace: false, trailingSpace: false }
    }

    const trimmed = text.trim()
    if (!trimmed) return { text: '', leadingSpace: false, trailingSpace: false }

    const hasExplicitInsertionSelection = Boolean(
      dictationSelection && !(dictationSelection.from === 1 && dictationSelection.to === 1)
    )

    if (!hasExplicitInsertionSelection || !dictationSelection) {
      const currentText = editor.getText()
      const prevChar = currentText.slice(-1)
      return {
        text: trimmed,
        leadingSpace: Boolean(prevChar) && !/\s/.test(prevChar) && !/^[\s,.;:!?)]/.test(trimmed),
        trailingSpace: false,
      }
    }

    const fallbackSelection = {
      from: editor.state.doc.content.size,
      to: editor.state.doc.content.size,
    }
    const { from, to } = dictationSelection ?? fallbackSelection
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
    if (!editor) return

    const insertion = getDictationInsertionPlan(text)
    if (!insertion.text) return
    const hasExplicitInsertionSelection = Boolean(
      dictationSelection && !(dictationSelection.from === 1 && dictationSelection.to === 1)
    )

    const insertionText = `${insertion.leadingSpace ? ' ' : ''}${insertion.text}${insertion.trailingSpace ? ' ' : ''}`

    logDictation(
      'info',
      hasExplicitInsertionSelection && dictationSelection
        ? `inserting transcription at selection from=${dictationSelection.from} to=${dictationSelection.to}`
        : 'inserting transcription at document end'
    )

    if (hasExplicitInsertionSelection && dictationSelection) {
      editor
        .chain()
        .focus()
        .insertContentAt(
          { from: dictationSelection.from, to: dictationSelection.to },
          insertionText
        )
        .run()
    } else {
      const end = Math.max(1, editor.state.doc.content.size - 1)
      editor
        .chain()
        .focus()
        .insertContentAt({ from: end, to: end }, insertionText)
        .run()
    }
    syncEditorState(sanitizeNoteHtml(editor.getHTML()) || '<p></p>')
  }

  async function finalizeCapturedAudio(audioBlob: Blob, details: string) {
    const wasAutoStopped = dictationAutoStopped

    await resetDictationCaptureState()

    logDictation('info', `finalizing recording; ${details}; blobBytes=${audioBlob.size}; blobType=${audioBlob.type || 'unknown'}`)

    if (!ondictate || audioBlob.size === 0) {
      dictationState = 'idle'
      if (audioBlob.size === 0) {
        logDictation('warn', 'recording finished without usable audio data')
        setDictationMessage(labels.dictationNoAudio, 'error')
      }
      return
    }

    dictationState = 'transcribing'
    if (wasAutoStopped) {
      dictationMessage = withDuration(
        labels.dictationAutoStopProcessing,
        formatDuration(dictationMaxSeconds)
      )
    } else {
      dictationMessage = labels.dictationTranscribing
    }

    try {
      logDictation('info', `transcription callback started; blobBytes=${audioBlob.size}`)
      const text = (await ondictate(audioBlob)).trim()
      logDictation('info', `transcription callback resolved; textLength=${text.length}`)
      if (text) {
        insertDictationText(text)
        dictationMessage = wasAutoStopped
          ? withDuration(labels.dictationAutoStopInserted, formatDuration(dictationMaxSeconds))
          : labels.dictationInserted
        dictationState = 'idle'
      } else {
        setDictationMessage(labels.dictationNoText, 'error')
      }
    } catch (error) {
      logDictation('error', `transcription callback failed; error=${describeDictationError(error)}`)
      setDictationMessage(
        error instanceof Error ? error.message : labels.dictationTranscriptionFailed,
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

    const audioBlob = sampleCount > 0
      ? encodeWavFromPcm(dictationPcmChunks, sampleRate)
      : new Blob([], { type: 'audio/wav' })
    await finalizeCapturedAudio(
      audioBlob,
      `strategy=pcm-wav; chunks=${dictationPcmChunks.length}; sampleCount=${sampleCount}; sampleRate=${sampleRate}`
    )
  }

  async function stopDictation(options?: { autoStop?: boolean }) {
    dictationAutoStopped = options?.autoStop ?? false

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

  async function startDictation() {
    if (!ondictate) return

    logDictation('info', 'dictation start requested')

    if (
      typeof window === 'undefined' ||
      typeof navigator === 'undefined' ||
      !navigator.mediaDevices?.getUserMedia
    ) {
      logDictation('warn', 'microphone APIs are unavailable in this runtime')
      setDictationMessage(labels.dictationNoMicrophone, 'error')
      return
    }

    try {
      dictationChunks = []
      dictationMessage = null
      dictationAutoStopped = false
      const currentEditorElement = editorElement
      const hasActiveEditorSelection =
        currentEditorElement != null &&
        typeof document !== 'undefined' &&
        document.activeElement instanceof Node &&
        currentEditorElement.contains(document.activeElement)
      dictationSelection = editor && isFocused && hasActiveEditorSelection
        ? {
            from: editor.state.selection.from,
            to: editor.state.selection.to,
          }
        : null
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
        setDictationMessage(labels.dictationNoMicrophone, 'error')
        return
      }

      dictationStrategy = strategy
      logDictation('info', `selected capture strategy=${strategy}; userAgent=${navigator.userAgent}`)

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
        logDictation('info', `PCM/WAV fallback started; sampleRate=${audioContext.sampleRate}; bufferSize=4096`)
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
              : labels.dictationTranscriptionFailed,
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
        if (dictationSeconds >= dictationMaxSeconds) {
          logDictation('info', `dictation reached max duration=${dictationMaxSeconds}s; auto-stopping`)
          void stopDictation({ autoStop: true })
        }
      }, 1000)
    } catch (error) {
      await failDictationCapture(
        error instanceof Error ? error.message : labels.dictationNoMicrophone,
        `getUserMedia failed; error=${describeDictationError(error)}`
      )
    }
  }

  async function toggleDictation() {
    if (dictationState === 'transcribing') return
    if (dictationState === 'recording') {
      await stopDictation()
      return
    }
    await startDictation()
  }

  function normalizeLinkHref(value: string) {
    const trimmed = value.trim()

    if (!trimmed) {
      return {
        isValid: false,
        normalized: '',
        error: labels.linkInvalidUrl,
      }
    }

    const candidate = /^[a-zA-Z][a-zA-Z\d+.-]*:/.test(trimmed) ? trimmed : `https://${trimmed}`

    try {
      const url = new URL(candidate)

      if (!['http:', 'https:'].includes(url.protocol)) {
        return {
          isValid: false,
          normalized: '',
          error: labels.linkInvalidHttp,
        }
      }

      return {
        isValid: true,
        normalized: url.toString(),
        error: null,
      }
    } catch {
      return {
        isValid: false,
        normalized: '',
        error: labels.linkInvalidExample,
      }
    }
  }

  async function updateLink() {
    if (!editor) return

    const { from, to } = editor.state.selection

    linkSelection = { from, to }
    linkDraftHref = editor.getAttributes('link').href ?? ''
    linkModalError = null
    isLinkModalOpen = true

    await tick()

    linkInputElement?.focus()
    linkInputElement?.select()
  }

  function closeLinkModal() {
    isLinkModalOpen = false
    linkModalError = null
    linkDraftHref = ''
    linkSelection = null

    editor?.commands.focus()
  }

  function handleLinkInput() {
    if (linkModalError) {
      linkModalError = null
    }
  }

  function submitLink() {
    if (!editor) return

    const result = normalizeLinkHref(linkDraftHref)

    if (!result.isValid) {
      linkModalError = result.error
      linkInputElement?.focus()
      return
    }

    let chain = editor.chain().focus()

    if (linkSelection) {
      chain = chain.setTextSelection(linkSelection)
    }

    chain.extendMarkRange('link').setLink({ href: result.normalized }).run()

    closeLinkModal()
    bumpEditorRevision()
  }

  function handleLinkModalKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault()
      closeLinkModal()
    }
  }

  function removeLink() {
    if (!editor) return

    editor.chain().focus().extendMarkRange('link').unsetLink().run()
    bumpEditorRevision()
  }

  function clearEditor() {
    editor?.commands.setContent('<p></p>', false)
    currentHtml = '<p></p>'
    originalHtml = '<p></p>'
    lastExternalHtml = normalizeNoteContentForEditor(content)
    bumpEditorRevision()
  }

  async function handleSave() {
    if (!editor || isSaveDisabled) return

    const html = normalizeNoteContentForRender(editor.getHTML())
    if (!html) return

    try {
      await onsave?.(html)

      if (clearOnSave) {
        clearEditor()
        editor.commands.focus('end')
        return
      }

      originalHtml = html
      lastExternalHtml = html
      currentHtml = html || '<p></p>'
      editor.commands.setContent(html, false)
      editor.commands.focus('end')
      bumpEditorRevision()
    } catch {
      // Save failed — keep content so the user can retry
    }
  }

  onMount(() => {
    const normalizedInitial = normalizeNoteContentForEditor(content)
    currentHtml = normalizedInitial
    originalHtml = normalizedInitial
    lastExternalHtml = normalizedInitial
    buildEditor()
  })

  onDestroy(() => {
    resetDictationTimer()
    if (mediaRecorder?.state === 'recording') {
      mediaRecorder.stop()
    }
    void teardownPcmDictation()
    stopMediaStreamTracks()
    editor?.destroy()
    editor = null
  })

  $effect(() => {
    const normalizedExternal = normalizeNoteContentForEditor(content)

    if (normalizedExternal === lastExternalHtml) {
      return
    }

    lastExternalHtml = normalizedExternal
    originalHtml = normalizedExternal
    currentHtml = normalizedExternal

    if (!editor) {
      return
    }

    const currentEditorHtml = normalizeNoteContentForRender(editor.getHTML())
    const nextEditorHtml = normalizeNoteContentForRender(normalizedExternal)

    if (currentEditorHtml !== nextEditorHtml) {
      editor.commands.setContent(normalizedExternal, false)
      bumpEditorRevision()
    }
  })
</script>

<div class="note-editor">
  <div
    class="note-editor__toolbar"
    aria-label={labels.toolbarAriaLabel}
    data-editor-revision={editorRevision}
  >
    {#each toolbarGroups as group (group.label)}
      <div class="note-editor__tool-group" role="group" aria-label={group.label}>
        {#each group.buttons as button (button.label)}
          <button
            type="button"
            class="note-editor__tool"
            class:note-editor__tool--active={button.isActive()}
            aria-pressed={button.isActive()}
            aria-label={button.label}
            title={button.label}
            onmousedown={(event) => event.preventDefault()}
            onclick={button.action}
          >
            <ActionIcon name={button.icon} size={16} />
          </button>
        {/each}
      </div>
    {/each}

    {#if supportsDictation}
      <div
        class="note-editor__tool-group note-editor__tool-group--dictation"
        role="group"
        aria-label={labels.dictationGroup}
      >
        <button
          type="button"
          class="note-editor__tool"
          class:note-editor__tool--recording={dictationState === 'recording'}
          aria-label={dictationButtonLabel}
          title={dictationButtonLabel}
          disabled={dictationState === 'transcribing'}
          onmousedown={(event) => event.preventDefault()}
          onclick={toggleDictation}
        >
          <ActionIcon name="mic" size={16} />
        </button>
        {#if dictationState === 'recording' || dictationState === 'transcribing'}
          <span
            class="note-editor__dictation-status"
            class:note-editor__dictation-status--recording={dictationState === 'recording'}
            data-testid="note-editor-dictation-timer"
          >
            {#if dictationState === 'recording'}
              {dictationTimerLabel} / {formatDuration(dictationMaxSeconds)}
            {:else}
              {labels.dictationProcessing}
            {/if}
          </span>
        {/if}
      </div>
    {/if}
  </div>

  <p class="note-editor__helper">{labels.helperText}</p>

  {#if dictationMessage}
    <p
      class="note-editor__dictation-message"
      class:note-editor__dictation-message--error={dictationState === 'error'}
      data-testid="note-editor-dictation-message"
    >
      {dictationMessage}
    </p>
  {/if}

  <div class="note-editor__surface" class:note-editor__surface--focused={isFocused}>
    <div bind:this={editorElement}></div>
  </div>

  <div class="note-editor__actions">
    {#if showCancel}
      <button
        class="note-editor__btn note-editor__btn--ghost"
        type="button"
        data-testid="note-cancel"
        onclick={() => oncancel?.()}
      >
        {cancelLabel}
      </button>
    {/if}

    <button
      class="note-editor__btn note-editor__btn--save note-editor__btn--icon-only"
      type="button"
      data-testid="note-save"
      disabled={isSaveDisabled}
      aria-disabled={isSaveDisabled}
      aria-label={saveLabel}
      title={saveLabel}
      onclick={handleSave}
    >
      <ActionIcon name="save" size={16} />
    </button>
  </div>

  {#if isLinkModalOpen}
    <div
      class="note-editor__modal-backdrop"
      role="presentation"
      onclick={(event) => {
        if (event.currentTarget === event.target) {
          closeLinkModal()
        }
      }}
    >
      <div
        class="note-editor__modal"
        role="dialog"
        tabindex="-1"
        aria-modal="true"
        aria-labelledby={linkModalTitleId}
        aria-describedby={linkModalError ? linkModalErrorId : linkModalDescriptionId}
        onkeydown={handleLinkModalKeydown}
      >
        <div class="note-editor__modal-header">
          <div class="note-editor__modal-icon">
            <ActionIcon name="link" size={16} />
          </div>
          <div class="note-editor__modal-copy">
            <h3 id={linkModalTitleId}>{labels.linkModalTitle}</h3>
            <p id={linkModalDescriptionId}>{labels.linkModalDescription}</p>
          </div>
        </div>

        <form
          class="note-editor__modal-form"
          novalidate
          onsubmit={(event) => {
            event.preventDefault()
            submitLink()
          }}
        >
          <label class="note-editor__modal-label" for="note-editor-link-input"
            >{labels.linkUrlLabel}</label
          >
          <input
            id="note-editor-link-input"
            bind:this={linkInputElement}
            class="note-editor__modal-input"
            type="text"
            inputmode="url"
            autocapitalize="off"
            autocomplete="off"
            autocorrect="off"
            spellcheck="false"
            placeholder={labels.linkPlaceholder}
            bind:value={linkDraftHref}
            aria-invalid={linkModalError ? 'true' : 'false'}
            aria-describedby={linkModalError ? linkModalErrorId : linkModalDescriptionId}
            data-testid="note-editor-link-input"
            oninput={handleLinkInput}
          />

          {#if linkModalError}
            <p
              id={linkModalErrorId}
              class="note-editor__modal-error"
              data-testid="note-editor-link-error"
            >
              {linkModalError}
            </p>
          {/if}

          <div class="note-editor__modal-actions">
            <button
              type="button"
              class="note-editor__btn note-editor__btn--ghost"
              data-testid="note-editor-link-cancel"
              onclick={closeLinkModal}
            >
              {labels.linkCancel}
            </button>
            <button
              type="submit"
              class="note-editor__btn note-editor__btn--save"
              data-testid="note-editor-link-submit"
            >
              {labels.linkSubmit}
            </button>
          </div>
        </form>
      </div>
    </div>
  {/if}
</div>

<style>
  .note-editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .note-editor__toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: 2px;
    align-items: center;
    padding: 2px;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    background: var(--surface-toolbar);
  }

  .note-editor__tool-group {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px;
  }

  .note-editor__tool-group + .note-editor__tool-group::before {
    content: '';
    align-self: center;
    width: 1px;
    height: 16px;
    margin: 0 var(--space-1);
    background: var(--color-hairline);
  }

  .note-editor__tool-group--dictation {
    margin-left: auto;
  }

  .note-editor__btn {
    padding: var(--space-2) var(--space-3);
    font-family: var(--font-sans);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    border-radius: var(--radius-control);
    cursor: pointer;
    border: 1px solid var(--color-border);
    transition:
      background-color var(--transition-base),
      border-color var(--transition-base),
      color var(--transition-base),
      box-shadow var(--transition-base);
  }

  .note-editor__btn--icon-only {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--control-height-sm);
    height: var(--control-height-sm);
    padding: 0;
  }

  .note-editor__btn:hover:not(:disabled) {
    border-color: var(--color-border-strong);
    color: var(--color-text-primary);
    background: color-mix(in srgb, var(--color-surface) 72%, black 28%);
  }

  .note-editor__tool {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: none;
    border-radius: var(--radius-xs);
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
    transition:
      background-color var(--transition-base),
      color var(--transition-base),
      box-shadow var(--transition-base);
  }

  .note-editor__tool :global(svg) {
    flex-shrink: 0;
    pointer-events: none;
  }

  .note-editor__tool:hover:not(:disabled) {
    background: var(--color-accent-faint);
    color: var(--color-text-primary);
  }

  .note-editor__tool:disabled {
    cursor: not-allowed;
    opacity: 0.48;
  }

  .note-editor__tool:focus-visible,
  .note-editor__btn:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .note-editor__tool--active {
    background: var(--color-accent-soft);
    color: var(--color-text-primary);
  }

  .note-editor__tool--recording {
    background: var(--color-danger-soft);
    color: var(--color-danger);
  }

  .note-editor__tool--recording:hover:not(:disabled) {
    background: var(--color-danger-soft);
    color: var(--color-danger-hover);
  }

  .note-editor__helper {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .note-editor__dictation-status {
    display: inline-flex;
    align-items: center;
    padding: 0 var(--space-2);
    font-size: var(--font-size-2xs);
    font-variant-numeric: tabular-nums;
    color: var(--color-text-muted);
  }

  .note-editor__dictation-status--recording {
    color: var(--color-danger);
  }

  .note-editor__dictation-message {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  .note-editor__dictation-message--error {
    color: var(--color-danger);
  }

  .note-editor__surface {
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    background: var(--color-surface-raised);
    overflow: hidden;
    transition:
      border-color var(--transition-base),
      box-shadow var(--transition-base);
  }

  .note-editor__surface--focused {
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
  }

  .note-editor__surface :global(.ProseMirror) {
    min-height: 88px;
    padding: var(--space-3);
    color: var(--color-text-primary);
    outline: none;
    font-family: var(--font-sans);
    font-size: var(--font-size-md);
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .note-editor__surface :global(.ProseMirror p.is-editor-empty:first-child::before) {
    content: attr(data-placeholder);
    color: var(--color-text-muted);
    float: left;
    height: 0;
    pointer-events: none;
  }

  .note-editor__surface :global(.ProseMirror p:first-child) {
    margin-top: 0;
  }

  .note-editor__surface :global(.ProseMirror p:last-child) {
    margin-bottom: 0;
  }

  .note-editor__surface :global(.ProseMirror a) {
    color: var(--color-accent-hover);
    text-decoration: underline;
  }

  .note-editor__surface :global(.ProseMirror blockquote) {
    margin: var(--space-3) 0;
    padding-left: var(--space-3);
    border-left: 3px solid color-mix(in srgb, var(--color-accent) 45%, var(--color-border));
    color: var(--color-text-secondary);
  }

  .note-editor__surface :global(.ProseMirror code) {
    background: color-mix(in srgb, var(--color-border) 65%, transparent);
    border-radius: var(--radius-sm);
    padding: 0.1rem 0.3rem;
    font-size: 0.95em;
  }

  .note-editor__surface :global(.ProseMirror pre) {
    background: color-mix(in srgb, var(--color-surface) 76%, black 24%);
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    overflow-x: auto;
  }

  .note-editor__surface :global(.ProseMirror ul),
  .note-editor__surface :global(.ProseMirror ol) {
    padding-left: 1.25rem;
  }

  .note-editor__surface :global(.ProseMirror:focus) {
    box-shadow: none;
  }

  .note-editor__surface :global(.ProseMirror ::selection) {
    background: color-mix(in srgb, var(--color-accent) 35%, transparent);
  }

  .note-editor__actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }

  .note-editor__modal-backdrop {
    position: fixed;
    inset: 0;
    z-index: 1200;
    display: grid;
    place-items: center;
    padding: var(--space-4);
    background: var(--color-overlay);
    backdrop-filter: blur(10px);
  }

  .note-editor__modal {
    width: min(100%, 28rem);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-4);
    border: 1px solid color-mix(in srgb, var(--color-border) 88%, transparent);
    border-radius: var(--radius-dialog);
    background: color-mix(in srgb, var(--color-surface) 92%, black 8%);
    box-shadow: var(--shadow-lg);
  }

  .note-editor__modal-header {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
  }

  .note-editor__modal-icon {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 2.5rem;
    height: 2.5rem;
    border-radius: var(--radius-surface);
    border: 1px solid color-mix(in srgb, var(--color-border) 82%, transparent);
    background: color-mix(in srgb, var(--color-accent) 12%, var(--color-surface));
    color: var(--color-accent-hover);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.04);
  }

  /* Size and paint come from ActionIcon (stroke 2, fill none); the icon inherits
     `color` from the tile above. The old rule forced `fill: currentColor`, which
     would flood a stroke-based glyph into a solid blob. */

  .note-editor__modal-copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .note-editor__modal-copy h3 {
    margin: 0;
    font-size: var(--font-size-md);
    color: var(--color-text-primary);
  }

  .note-editor__modal-copy p {
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
  }

  .note-editor__modal-form {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .note-editor__modal-label {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.02em;
    color: var(--color-text-secondary);
  }

  .note-editor__modal-input {
    width: 100%;
    padding: var(--space-3) var(--space-4);
    border: 1px solid color-mix(in srgb, var(--color-border) 90%, transparent);
    border-radius: var(--radius-input);
    background: color-mix(in srgb, var(--color-surface) 82%, black 18%);
    color: var(--color-text-primary);
    font-family: var(--font-sans);
    font-size: var(--font-size-sm);
    transition:
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth),
      background-color var(--transition-smooth);
  }

  .note-editor__modal-input::placeholder {
    color: var(--color-text-muted);
  }

  .note-editor__modal-input:focus {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
    background: color-mix(in srgb, var(--color-surface) 88%, black 12%);
  }

  .note-editor__modal-input[aria-invalid='true'] {
    border-color: var(--color-danger);
    box-shadow: var(--focus-ring-danger);
  }

  .note-editor__modal-error {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-danger);
  }

  .note-editor__modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }

  .note-editor__btn:disabled {
    cursor: not-allowed;
    opacity: 0.48;
  }

  .note-editor__btn--ghost {
    background: transparent;
    color: var(--color-text-secondary);
  }

  .note-editor__btn--save {
    background-color: var(--color-accent);
    color: var(--color-bg);
    border-color: var(--color-accent);
  }

  .note-editor__btn--save:disabled {
    opacity: 1;
    background-color: var(--color-surface-elevated);
    color: var(--color-text-secondary);
    border-color: var(--color-border-strong);
  }

  .note-editor__btn--save:hover:not(:disabled) {
    background-color: var(--color-accent-hover);
    border-color: var(--color-accent-hover);
  }
</style>
