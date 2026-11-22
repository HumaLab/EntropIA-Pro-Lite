export interface NoteEditorProps {
  content?: string
  placeholder?: string
  onsave?: (content: string) => void | Promise<void>
  oncancel?: () => void
  ondictate?: (audio: Blob) => Promise<string>
  ondictationlog?: (level: 'info' | 'warn' | 'error', message: string) => void | Promise<void>
  /** Shown as a button next to a failed dictation, to open the app's fix. */
  ondictationhelp?: () => void
  dictationMaxSeconds?: number
  clearOnSave?: boolean
  saveLabel?: string
  cancelLabel?: string
  labels?: Partial<NoteEditorLabels>
}

export interface NoteEditorLabels {
  toolbarAriaLabel: string
  textStyleGroup: string
  structureGroup: string
  insertGroup: string
  dictationGroup: string
  bold: string
  italic: string
  underline: string
  inlineCode: string
  heading1: string
  heading2: string
  heading3: string
  bulletList: string
  orderedList: string
  quote: string
  addLink: string
  removeLink: string
  dictationStart: string
  dictationStop: string
  dictationProcessing: string
  helperText: string
  dictationNoMicrophone: string
  dictationNoAudio: string
  dictationSilentMicrophone: string
  dictationAutoStopProcessing: string
  dictationTranscribing: string
  dictationAutoStopInserted: string
  dictationInserted: string
  dictationNoText: string
  dictationTranscriptionFailed: string
  dictationOpenDependencies: string
  linkInvalidUrl: string
  linkInvalidHttp: string
  linkInvalidExample: string
  linkModalTitle: string
  linkModalDescription: string
  linkUrlLabel: string
  linkPlaceholder: string
  linkCancel: string
  linkSubmit: string
}
