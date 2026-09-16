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
  /** Fires when a corpus citation is clicked, with its whole anchor (§10.2). */
  oncitation?: (attrs: Record<string, unknown>) => void
  /** Fires when a note link is clicked, with its snapshot and hash (§13). */
  onnotelink?: (attrs: Record<string, unknown>) => void
  editable?: boolean
  /** Show the formatting toolbar. */
  toolbar?: boolean
  placeholder?: string
  labels?: Partial<WritingEditorLabels>
}

export interface WritingEditorLabels {
  /** Accessible name for the editing surface. */
  editorLabel: string
  toolbarLabel: string
  undo: string
  redo: string
  bold: string
  italic: string
  underline: string
  strike: string
  code: string
  heading1: string
  heading2: string
  heading3: string
  bulletList: string
  orderedList: string
  blockquote: string
  link: string
  unlink: string
  table: string
  tableControls: string
  addRow: string
  addColumn: string
  deleteRow: string
  deleteColumn: string
  deleteTable: string
  footnote: string
  find: string
  findPrevious: string
  findNext: string
  replace: string
  replaceOne: string
  replaceAll: string
  closeSearch: string
  noMatches: string
  /** Shown instead of the editor when the document cannot be mounted. */
  refusedTitle: string
  refusedUnknownNode: string
  refusedUnknownMark: string
  refusedUnsupportedVersion: string
  refusedInvalidStructure: string
  buildFailedTitle: string
  buildFailedBody: string
}

export const DEFAULT_WRITING_EDITOR_LABELS: WritingEditorLabels = {
  editorLabel: 'Manuscrito',
  toolbarLabel: 'Formato',
  undo: 'Deshacer',
  redo: 'Rehacer',
  bold: 'Negrita',
  italic: 'Cursiva',
  underline: 'Subrayado',
  strike: 'Tachado',
  code: 'Código',
  heading1: 'Título 1',
  heading2: 'Título 2',
  heading3: 'Título 3',
  bulletList: 'Lista',
  orderedList: 'Lista ordenada',
  blockquote: 'Cita en bloque',
  link: 'Enlace',
  unlink: 'Quitar enlace',
  table: 'Insertar tabla',
  tableControls: 'Controles de tabla',
  addRow: '+ Fila',
  addColumn: '+ Columna',
  deleteRow: '− Fila',
  deleteColumn: '− Columna',
  deleteTable: 'Eliminar tabla',
  footnote: 'Nota al pie',
  find: 'Buscar',
  findPrevious: 'Coincidencia anterior',
  findNext: 'Coincidencia siguiente',
  replace: 'Reemplazar por',
  replaceOne: 'Reemplazar',
  replaceAll: 'Reemplazar todo',
  closeSearch: 'Cerrar la búsqueda',
  noMatches: 'Sin coincidencias',
  refusedTitle: 'Este documento no se puede abrir en esta versión',
  refusedUnknownNode:
    'Contiene un elemento que esta versión de EntropIA no conoce. El documento no se modificó.',
  refusedUnknownMark:
    'Contiene un formato que esta versión de EntropIA no conoce. El documento no se modificó.',
  refusedUnsupportedVersion:
    'Fue escrito por una versión más nueva de EntropIA. El documento no se modificó.',
  refusedInvalidStructure:
    'Su contenido no se pudo interpretar. El documento no se modificó.',
  buildFailedTitle: 'No se pudo abrir el editor',
  buildFailedBody:
    'El documento está intacto y no se modificó. El detalle técnico es este:',
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
