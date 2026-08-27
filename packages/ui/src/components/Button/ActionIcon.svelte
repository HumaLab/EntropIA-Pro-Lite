<script lang="ts">
  import {
    BrushCleaning,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    ChevronUp,
    CirclePlay,
    Copy,
    Crop,
    Database,
    Download,
    Eraser,
    Expand,
    Eye,
    EyeOff,
    ExternalLink,
    File,
    FileAudio,
    FileBraces,
    FileImage,
    FileText,
    FileUp,
    Folder,
    FolderPlus,
    Hand,
    Languages,
    MessageCircle,
    MessageCirclePlus,
    Moon,
    PanelLeft,
    PanelLeftClose,
    Pause,
    Pencil,
    Play,
    RotateCcw,
    RotateCw,
    Redo2,
    Save,
    Search,
    SearchX,
    Send,
    Settings,
    SkipBack,
    SkipForward,
    Square,
    Trash2,
    Underline,
    Undo2,
    Volume2,
    X,
    ZoomIn,
    ZoomOut,
  } from '@lucide/svelte'
  import type { ActionIconName } from './ActionIcon.types'

  let { name, size = 16 }: { name: ActionIconName; size?: number } = $props()
</script>

{#if name === 'broom'}
  <BrushCleaning {size} aria-hidden="true" />
{:else if name === 'chevron-left'}
  <ChevronLeft {size} aria-hidden="true" />
{:else if name === 'chevron-right'}
  <ChevronRight {size} aria-hidden="true" />
{:else if name === 'chevron-up'}
  <ChevronUp {size} aria-hidden="true" />
{:else if name === 'chevron-down'}
  <ChevronDown {size} aria-hidden="true" />
{:else if name === 'close'}
  <X {size} aria-hidden="true" />
{:else if name === 'circle-play'}
  <CirclePlay {size} aria-hidden="true" />
{:else if name === 'copy'}
  <Copy {size} aria-hidden="true" />
{:else if name === 'crop'}
  <Crop {size} aria-hidden="true" />
{:else if name === 'database'}
  <Database {size} aria-hidden="true" />
{:else if name === 'delete'}
  <Trash2 {size} aria-hidden="true" />
{:else if name === 'download'}
  <Download {size} aria-hidden="true" />
{:else if name === 'edit'}
  <Pencil {size} aria-hidden="true" />
{:else if name === 'eraser'}
  <Eraser {size} aria-hidden="true" />
{:else if name === 'expand'}
  <Expand {size} aria-hidden="true" />
{:else if name === 'eye'}
  <Eye {size} aria-hidden="true" />
{:else if name === 'eye-off'}
  <EyeOff {size} aria-hidden="true" />
{:else if name === 'external-link'}
  <ExternalLink {size} aria-hidden="true" />
{:else if name === 'file'}
  <File {size} aria-hidden="true" />
{:else if name === 'file-audio'}
  <FileAudio {size} aria-hidden="true" />
{:else if name === 'file-braces'}
  <FileBraces {size} aria-hidden="true" />
{:else if name === 'file-image'}
  <FileImage {size} aria-hidden="true" />
{:else if name === 'file-text'}
  <FileText {size} aria-hidden="true" />
{:else if name === 'file-up'}
  <FileUp {size} aria-hidden="true" />
{:else if name === 'folder'}
  <Folder {size} aria-hidden="true" />
{:else if name === 'folder-plus'}
  <FolderPlus {size} aria-hidden="true" />
{:else if name === 'hand'}
  <Hand {size} aria-hidden="true" />
{:else if name === 'languages'}
  <Languages {size} aria-hidden="true" />
{:else if name === 'message-circle'}
  <MessageCircle {size} aria-hidden="true" />
{:else if name === 'message-circle-plus'}
  <MessageCirclePlus {size} aria-hidden="true" />
{:else if name === 'moon'}
  <Moon {size} aria-hidden="true" />
{:else if name === 'panel-left'}
  <PanelLeft {size} aria-hidden="true" />
{:else if name === 'panel-left-close'}
  <PanelLeftClose {size} aria-hidden="true" />
{:else if name === 'pause'}
  <Pause {size} aria-hidden="true" />
{:else if name === 'pencil'}
  <Pencil {size} aria-hidden="true" />
{:else if name === 'play'}
  <Play {size} aria-hidden="true" />
{:else if name === 'rectangle'}
  <Square {size} aria-hidden="true" />
{:else if name === 'redo'}
  <Redo2 {size} aria-hidden="true" />
{:else if name === 'rotate-ccw'}
  <RotateCcw {size} aria-hidden="true" />
{:else if name === 'rotate-cw'}
  <RotateCw {size} aria-hidden="true" />
{:else if name === 'rotate-fine-ccw'}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.7"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    data-action-icon="rotate-fine-ccw"
  >
    <circle cx="12" cy="12" r="7" stroke-dasharray="1 3" />
    <circle cx="12" cy="12" r="2.2" fill="currentColor" stroke="none" />
    <path d="M12 12 8.7 8.7" />
    <path d="M8.8 5.4 6.2 8l2.6 2.6" />
    <text
      x="16.5"
      y="18.2"
      fill="currentColor"
      stroke="none"
      font-size="5.4"
      font-family="monospace"
      font-weight="700"
      text-anchor="middle"
    >-°</text>
  </svg>
{:else if name === 'rotate-fine-cw'}
  <svg
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.7"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    data-action-icon="rotate-fine-cw"
  >
    <circle cx="12" cy="12" r="7" stroke-dasharray="1 3" />
    <circle cx="12" cy="12" r="2.2" fill="currentColor" stroke="none" />
    <path d="M12 12 15.3 8.7" />
    <path d="M15.2 5.4 17.8 8l-2.6 2.6" />
    <text
      x="16.5"
      y="18.2"
      fill="currentColor"
      stroke="none"
      font-size="5.4"
      font-family="monospace"
      font-weight="700"
      text-anchor="middle"
    >+°</text>
  </svg>
{:else if name === 'search'}
  <Search {size} aria-hidden="true" />
{:else if name === 'search-x'}
  <SearchX {size} aria-hidden="true" />
{:else if name === 'save'}
  <Save {size} aria-hidden="true" />
{:else if name === 'send'}
  <Send {size} aria-hidden="true" />
{:else if name === 'settings'}
  <Settings {size} aria-hidden="true" />
{:else if name === 'skip-back'}
  <SkipBack {size} aria-hidden="true" />
{:else if name === 'skip-forward'}
  <SkipForward {size} aria-hidden="true" />
{:else if name === 'underline'}
  <Underline {size} aria-hidden="true" />
{:else if name === 'undo'}
  <Undo2 {size} aria-hidden="true" />
{:else if name === 'volume'}
  <Volume2 {size} aria-hidden="true" />
{:else if name === 'zoom-in'}
  <ZoomIn {size} aria-hidden="true" />
{:else if name === 'zoom-out'}
  <ZoomOut {size} aria-hidden="true" />
{/if}
