import type { DictationLabels, DictationLogLevel } from '../Dictation/dictation.svelte'
import type { CanonicalDocument, ValidationFailure } from './document-contract'
import type { WritingColorLabelKey } from './writing-colors'

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
  /** Fires when a bibliographic citation is clicked, to adjust it (§11.5). */
  onzoterocitation?: (attrs: Record<string, unknown>) => void
  editable?: boolean
  /** Show the formatting toolbar. */
  toolbar?: boolean
  placeholder?: string
  /**
   * Transcribes a dictation. Without it there is no microphone in the toolbar;
   * with it, the transcription goes in at the caret, replacing any selection.
   */
  ondictate?: (audio: Blob) => Promise<string>
  /** Capture diagnostics, for the app log. */
  ondictationlog?: (level: DictationLogLevel, message: string) => void | Promise<void>
  /** Recording stops by itself after this many seconds. */
  dictationMaxSeconds?: number
  labels?: Partial<WritingEditorLabels>
}

/** One name per palette colour, for its swatch: `colorRed`, `colorBlue`… */
type ColorLabels = Record<WritingColorLabelKey, string>

export interface WritingEditorLabels extends DictationLabels, ColorLabels {
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
  fontSizeIncrease: string
  fontSizeDecrease: string
  /** The Aa button, and the name of the menu it opens. */
  changeCase: string
  caseUpper: string
  caseLower: string
  caseSentence: string
  caseWords: string
  subscript: string
  superscript: string
  clearFormatting: string
  /** The two colour buttons, and the names of the menus they open. */
  highlight: string
  textColor: string
  /** The entry in each colour menu that takes the colour off. */
  noColor: string
  /** Indent in and out; in a list, they nest and un-nest the item. */
  indentDecrease: string
  indentIncrease: string
  alignLeft: string
  alignCenter: string
  alignRight: string
  alignJustify: string
  /** The line spacing button, the name of its menu, and its overflow heading. */
  lineHeight: string
  /**
   * The four spacings, written with the locale's decimal separator: the
   * stored values are always `1`, `1.15`, `1.5` and `2`.
   */
  lineHeight1: string
  lineHeight115: string
  lineHeight15: string
  lineHeight2: string
  /** Takes the spacing off, back to the theme's own. */
  lineHeightDefault: string
  find: string
  findPrevious: string
  findNext: string
  replace: string
  replaceOne: string
  replaceAll: string
  closeSearch: string
  noMatches: string
  /** The button that holds the tools the toolbar has no room for. */
  moreTools: string
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
  fontSizeIncrease: 'Aumentar tamaño de fuente',
  fontSizeDecrease: 'Disminuir tamaño de fuente',
  changeCase: 'Cambiar mayúsculas y minúsculas',
  caseUpper: 'MAYÚSCULAS',
  caseLower: 'minúsculas',
  caseSentence: 'Tipo oración',
  caseWords: 'Capitalizar palabras',
  subscript: 'Subíndice',
  superscript: 'Superíndice',
  clearFormatting: 'Borrar formato',
  highlight: 'Color de resaltado',
  textColor: 'Color de texto',
  noColor: 'Sin color',
  indentDecrease: 'Disminuir sangría',
  indentIncrease: 'Aumentar sangría',
  alignLeft: 'Alinear a la izquierda',
  alignCenter: 'Centrar',
  alignRight: 'Alinear a la derecha',
  alignJustify: 'Justificar',
  lineHeight: 'Interlineado',
  lineHeight1: '1',
  lineHeight115: '1,15',
  lineHeight15: '1,5',
  lineHeight2: '2',
  lineHeightDefault: 'Predeterminado',
  colorGray: 'Gris',
  colorRed: 'Rojo',
  colorOrange: 'Naranja',
  colorYellow: 'Amarillo',
  colorGreen: 'Verde',
  colorBlue: 'Azul',
  colorPurple: 'Violeta',
  colorPink: 'Rosa',
  find: 'Buscar',
  findPrevious: 'Coincidencia anterior',
  findNext: 'Coincidencia siguiente',
  replace: 'Reemplazar por',
  replaceOne: 'Reemplazar',
  replaceAll: 'Reemplazar todo',
  closeSearch: 'Cerrar la búsqueda',
  noMatches: 'Sin coincidencias',
  moreTools: 'Más herramientas',
  dictationStart: 'Iniciar dictado',
  dictationStop: 'Detener dictado',
  dictationProcessing: 'Procesando dictado...',
  dictationNoMicrophone: 'No hay micrófono disponible en este dispositivo.',
  dictationNoAudio: 'No se pudo capturar audio del micrófono.',
  dictationAutoStopProcessing: 'Se alcanzó el máximo de {duration}. Procesando audio...',
  dictationTranscribing: 'Transcribiendo audio...',
  dictationAutoStopInserted: 'Se alcanzó el máximo de {duration}. Texto insertado.',
  dictationInserted: 'Texto insertado desde el micrófono.',
  dictationNoText: 'No se detectó texto en el audio.',
  dictationTranscriptionFailed: 'No se pudo transcribir el audio.',
  refusedTitle: 'Este documento no se puede abrir en esta versión',
  refusedUnknownNode:
    'Contiene un elemento que esta versión de EntropIA no conoce. El documento no se modificó.',
  refusedUnknownMark:
    'Contiene un formato que esta versión de EntropIA no conoce. El documento no se modificó.',
  refusedUnsupportedVersion:
    'Fue escrito por una versión más nueva de EntropIA. El documento no se modificó.',
  refusedInvalidStructure: 'Su contenido no se pudo interpretar. El documento no se modificó.',
  buildFailedTitle: 'No se pudo abrir el editor',
  buildFailedBody: 'El documento está intacto y no se modificó. El detalle técnico es este:',
}

export function refusalMessage(failure: ValidationFailure, labels: WritingEditorLabels): string {
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
