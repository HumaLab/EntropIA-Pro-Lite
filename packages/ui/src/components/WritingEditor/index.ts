export { default as WritingEditor } from './WritingEditor.svelte'
export { WRITING_EXTENSIONS, DocumentCitation } from './extensions'
export {
  WRITING_SCHEMA_VERSION,
  emptyDocument,
  parseCanonical,
  validateCanonical,
} from './document-contract'
export type {
  CanonicalDocument,
  ParseResult,
  ValidationFailure,
  ValidationResult,
} from './document-contract'
export {
  DEFAULT_WRITING_EDITOR_LABELS,
  refusalMessage,
} from './WritingEditor.types'
export type { WritingEditorLabels, WritingEditorProps } from './WritingEditor.types'
