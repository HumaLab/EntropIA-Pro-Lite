// Design tokens
export { colors, spacing, typography, radius, shadows } from './tokens/index'

// Components — Fase 0
export { Button } from './components/Button/index'
export { ActionIcon } from './components/Button/index'
export type {
  ButtonProps,
  ButtonVariant,
  ButtonSize,
  ActionIconName,
} from './components/Button/index'

export { Input } from './components/Input/index'
export type { InputProps, InputType } from './components/Input/index'

export { Card } from './components/Card/index'
export type { CardProps, CardPadding } from './components/Card/index'

export { Panel } from './components/Panel/index'
export type { PanelPadding, PanelProps, PanelVariant } from './components/Panel/index'

export { TabButton, TabList } from './components/Tabs/index'
export type { TabButtonProps, TabListProps } from './components/Tabs/index'

export { IconButton } from './components/IconButton/index'
export type { IconButtonProps, IconButtonSize, IconButtonVariant } from './components/IconButton/index'
export { SearchClearButton } from './components/SearchClearButton/index'
export type { SearchClearButtonProps } from './components/SearchClearButton/index'

export { StatusBadge } from './components/StatusBadge/index'
export type { StatusBadgeProps, StatusBadgeSize, StatusBadgeVariant } from './components/StatusBadge/index'

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

export { NoteEditor } from './components/NoteEditor/index'
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

export { MapViewer } from './components/MapViewer/index'
export type {
  MapLocationOption,
  MapViewerLabels,
  MapViewerProps,
  MapMarker,
} from './components/MapViewer/index'
