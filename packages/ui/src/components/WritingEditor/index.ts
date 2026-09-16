export { default as WritingEditor } from './WritingEditor.svelte'
export { createWritingExtensions, DocumentCitation } from './extensions'
export { outlineFromDocument, outlineDepth } from './outline'
export {
  citationProjection,
  citationsFromDocument,
  duplicatedCitationIds,
  zoteroCitationProjection,
  zoteroCitationsFromDocument,
} from './citations'
export { newCitationId } from './unique-citation-ids'
export { citeWork, worksOf } from './citation-cluster'
export { findMatches } from './search'
export type { SearchMatch, SearchOptions } from './search'
export type { DocumentCitationRow, ZoteroCitationRow } from './citations'
export type { OutlineEntry } from './outline'
export {
  WRITING_SCHEMA_VERSION,
  emptyDocument,
  parseCanonical,
  repairCanonical,
  needsRepair,
  validateCanonical,
  writingSchema,
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
