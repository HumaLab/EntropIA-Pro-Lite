export { default as WritingEditor } from './WritingEditor.svelte'
export { createWritingExtensions, DocumentCitation } from './extensions'
export { outlineFromDocument, outlineDepth } from './outline'
export { citationProjection, citationsFromDocument, duplicatedCitationIds } from './citations'
export type { DocumentCitationRow } from './citations'
export type { OutlineEntry } from './outline'
export {
  WRITING_SCHEMA_VERSION,
  emptyDocument,
  parseCanonical,
  repairCanonical,
  needsRepair,
  validateCanonical,
} from './document-contract'
export type {
  CanonicalDocument,
  ParseResult,
  ValidationFailure,
  ValidationResult,
  RepairReport,
} from './document-contract'
export {
  DEFAULT_WRITING_EDITOR_LABELS,
  refusalMessage,
} from './WritingEditor.types'
export type { WritingEditorLabels, WritingEditorProps } from './WritingEditor.types'
