// Design tokens
export { colors, spacing, typography, radius, shadows } from './tokens/index'

// Components — Fase 0
export { Button } from './components/Button/index'
export { ActionIcon } from './components/Button/index'
export { ACTION_ICON_NAMES, ACTION_ICON_SIZES } from './components/Button/index'
export type {
  ButtonProps,
  ButtonVariant,
  ButtonSize,
  ActionIconName,
  ActionIconSize,
} from './components/Button/index'

export { Input } from './components/Input/index'
export type { InputProps, InputType } from './components/Input/index'

export { Checkbox } from './components/Checkbox/index'
export type { CheckboxProps } from './components/Checkbox/index'

export { Card } from './components/Card/index'
export type { CardProps, CardPadding } from './components/Card/index'

export { Panel } from './components/Panel/index'
export type { PanelPadding, PanelProps, PanelVariant } from './components/Panel/index'

export { TabButton, TabList } from './components/Tabs/index'
export type { TabButtonProps, TabListProps } from './components/Tabs/index'

export { TooltipLayer, tooltip } from './components/Tooltip/index'
export type { TooltipAnchor, TooltipState } from './components/Tooltip/index'

export { IconButton } from './components/IconButton/index'
export type {
  IconButtonProps,
  IconButtonSize,
  IconButtonVariant,
} from './components/IconButton/index'
export { SearchClearButton } from './components/SearchClearButton/index'
export type { SearchClearButtonProps } from './components/SearchClearButton/index'

export { StatusBadge } from './components/StatusBadge/index'
export type {
  StatusBadgeProps,
  StatusBadgeSize,
  StatusBadgeVariant,
} from './components/StatusBadge/index'

export { ConfirmDialog } from './components/ConfirmDialog/index'
export type { ConfirmDialogProps, ConfirmDialogVariant } from './components/ConfirmDialog/index'

// Components — Fase 1
export { CollectionCard } from './components/CollectionCard/index'
export type { CollectionCardProps } from './components/CollectionCard/index'

export { ItemCard } from './components/ItemCard/index'
export type { ItemCardProps } from './components/ItemCard/index'

export { DocumentViewer } from './components/DocumentViewer/index'
export type {
  DocumentViewerProps,
  ViewerType,
  ViewerAnnotation,
  ViewerLayoutRegion,
  AnnotationKind,
  DocumentEditKind,
  ViewerAnnotationKind,
  AnnotationTool,
  EditTool,
  ImageEditResult,
} from './components/DocumentViewer/index'

export { AudioPlayer } from './components/AudioPlayer/index'

export { SearchBar } from './components/SearchBar/index'
export type { SearchBarProps } from './components/SearchBar/index'

export { MetadataEditor } from './components/MetadataEditor/index'
export type { MetadataEditorProps } from './components/MetadataEditor/index'

export { TopicEditor } from './components/TopicEditor/index'
export type { TopicEditorProps } from './components/TopicEditor/index'

export type { NoteEditorProps } from './components/NoteEditor/index'
export {
  convertLegacyNoteTextToHtml,
  hasNoteEditorMeaningfulChanges,
  isLegacyPlainTextNoteContent,
  isNoteHtmlEffectivelyEmpty,
  normalizeNoteLinkHref,
  normalizeNoteContentForEditor,
  normalizeNoteContentForRender,
  sanitizeNoteHtml,
  shouldDisableNoteEditorSave,
} from './components/NoteEditor/index'

// Components — Fase 3
export { EntityViewer } from './components/EntityViewer/index'
export type { Entity, EntityType, EntityViewerProps } from './components/EntityViewer/index'

export type {
  MapLocationOption,
  MapViewerLabels,
  MapViewerProps,
  MapMarker,
} from './components/MapViewer/index'

// Virtualized collection grid
export { VirtualGrid } from './components/VirtualGrid/index'
export {
  computeVirtualGridWindow,
  resolveColumnCount,
  resolveFocusTarget,
} from './components/VirtualGrid/index'
export type {
  FocusTarget,
  VirtualGridWindow,
  VirtualGridWindowInput,
} from './components/VirtualGrid/index'

export { ResizeHandle } from './components/ResizeHandle/index'
export {
  EDITOR_MIN_WIDTH,
  OUTLINE_BOUNDS,
  RESEARCH_BOUNDS,
  clampPanel,
  readPanelWidth,
} from './components/ResizeHandle/index'
export type { PanelBounds, PanelSide } from './components/ResizeHandle/index'
export { WritingEditor } from './components/WritingEditor/index'
export {
  createWritingExtensions,
  DocumentCitation,
  outlineFromDocument,
  outlineDepth,
  citationProjection,
  zoteroCitationProjection,
  zoteroCitationsFromDocument,
  newCitationId,
  worksOf,
  findMatches,
  citationsFromDocument,
  duplicatedCitationIds,
  WRITING_SCHEMA_VERSION,
  emptyDocument,
  parseCanonical,
  repairCanonical,
  needsRepair,
  validateCanonical,
  writingSchema,
  DEFAULT_WRITING_EDITOR_LABELS,
  refusalMessage,
} from './components/WritingEditor/index'
export type {
  CanonicalDocument,
  ParseResult,
  ValidationFailure,
  ValidationResult,
  RepairReport,
  SearchMatch,
  SearchOptions,
  OutlineEntry,
  DocumentCitationRow,
  ZoteroCitationRow,
  WritingEditorLabels,
  WritingEditorProps,
} from './components/WritingEditor/index'
