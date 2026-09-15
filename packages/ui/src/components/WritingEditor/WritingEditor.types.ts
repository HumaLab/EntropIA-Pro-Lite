import type { CanonicalDocument, ValidationFailure } from './document-contract'

export interface WritingEditorProps {
  /**
   * The manuscript to mount. Must already have gone through `parseCanonical`:
   * the component will not mount a document it was not handed as valid, and
   * says so rather than rendering a blank page.
   */
  document: CanonicalDocument
  /** Fires on every editor transaction that changed the document. */
  onchange?: (next: CanonicalDocument) => void
  /** Fires when the editor is ready, so a caller can focus it. */
  onready?: () => void
  editable?: boolean
  placeholder?: string
  labels?: Partial<WritingEditorLabels>
}

export interface WritingEditorLabels {
  /** Accessible name for the editing surface. */
  editorLabel: string
  /** Shown instead of the editor when the document cannot be mounted. */
  refusedTitle: string
  refusedUnknownNode: string
  refusedUnknownMark: string
  refusedUnsupportedVersion: string
  refusedInvalidStructure: string
}

export const DEFAULT_WRITING_EDITOR_LABELS: WritingEditorLabels = {
  editorLabel: 'Manuscrito',
  refusedTitle: 'Este documento no se puede abrir en esta versión',
  refusedUnknownNode:
    'Contiene un elemento que esta versión de EntropIA no conoce. El documento no se modificó.',
  refusedUnknownMark:
    'Contiene un formato que esta versión de EntropIA no conoce. El documento no se modificó.',
  refusedUnsupportedVersion:
    'Fue escrito por una versión más nueva de EntropIA. El documento no se modificó.',
  refusedInvalidStructure:
    'Su contenido no se pudo interpretar. El documento no se modificó.',
}

export function refusalMessage(
  failure: ValidationFailure,
  labels: WritingEditorLabels
): string {
  switch (failure.code) {
    case 'unknown-node':
      return labels.refusedUnknownNode
    case 'unknown-mark':
      return labels.refusedUnknownMark
    case 'unsupported-schema-version':
      return labels.refusedUnsupportedVersion
    default:
      return labels.refusedInvalidStructure
  }
}
