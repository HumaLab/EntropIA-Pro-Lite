<script lang="ts">
  /**
   * The single gate every icon in the app passes through.
   *
   * No component imports from '@tabler/icons-svelte-runes' or hand-writes an
   * <svg> — the ESLint rule in eslint.config.js enforces that. Adding an icon
   * means one entry in ACTION_ICON_NAMES and one entry in the map below.
   *
   * See ActionIcon.types.ts for the contract this component implements.
   *
   * Each glyph is imported from its OWN module, never from the package barrel.
   * Do not "tidy" these back into one `import { IconX, IconY } from '...'`:
   * Tabler ships every icon as raw .svelte with no precompiled .js, so in dev
   * the barrel is 7.9 MB of JavaScript that statically imports 6184 .svelte
   * files, each one compiled on demand. Rollup drops them for a production
   * build, but the dev server has to serve them all, and Tauri's WebView never
   * finishes booting — the splash screen gives up after 20s on a black window.
   */
  import IconAlertTriangle from '@tabler/icons-svelte-runes/icons/alert-triangle'
  import IconArrowBackUp from '@tabler/icons-svelte-runes/icons/arrow-back-up'
  import IconArrowForwardUp from '@tabler/icons-svelte-runes/icons/arrow-forward-up'
  import IconArrowsMaximize from '@tabler/icons-svelte-runes/icons/arrows-maximize'
  import IconBell from '@tabler/icons-svelte-runes/icons/bell'
  import IconBold from '@tabler/icons-svelte-runes/icons/bold'
  import IconBrush from '@tabler/icons-svelte-runes/icons/brush'
  import IconCheck from '@tabler/icons-svelte-runes/icons/check'
  import IconChecks from '@tabler/icons-svelte-runes/icons/checks'
  import IconChevronDown from '@tabler/icons-svelte-runes/icons/chevron-down'
  import IconChevronLeft from '@tabler/icons-svelte-runes/icons/chevron-left'
  import IconChevronRight from '@tabler/icons-svelte-runes/icons/chevron-right'
  import IconChevronUp from '@tabler/icons-svelte-runes/icons/chevron-up'
  import IconChevronsLeft from '@tabler/icons-svelte-runes/icons/chevrons-left'
  import IconChevronsRight from '@tabler/icons-svelte-runes/icons/chevrons-right'
  import IconCircleCaretRight from '@tabler/icons-svelte-runes/icons/circle-caret-right'
  import IconCircleCheck from '@tabler/icons-svelte-runes/icons/circle-check'
  import IconCircleX from '@tabler/icons-svelte-runes/icons/circle-x'
  import IconCode from '@tabler/icons-svelte-runes/icons/code'
  import IconContrast from '@tabler/icons-svelte-runes/icons/contrast'
  import IconCopy from '@tabler/icons-svelte-runes/icons/copy'
  import IconCrop from '@tabler/icons-svelte-runes/icons/crop'
  import IconDatabase from '@tabler/icons-svelte-runes/icons/database'
  import IconDeviceFloppy from '@tabler/icons-svelte-runes/icons/device-floppy'
  import IconDownload from '@tabler/icons-svelte-runes/icons/download'
  import IconEraser from '@tabler/icons-svelte-runes/icons/eraser'
  import IconExternalLink from '@tabler/icons-svelte-runes/icons/external-link'
  import IconEye from '@tabler/icons-svelte-runes/icons/eye'
  import IconEyeOff from '@tabler/icons-svelte-runes/icons/eye-off'
  import IconFile from '@tabler/icons-svelte-runes/icons/file'
  import IconFileDownload from '@tabler/icons-svelte-runes/icons/file-download'
  import IconFileMusic from '@tabler/icons-svelte-runes/icons/file-music'
  import IconFilePlus from '@tabler/icons-svelte-runes/icons/file-plus'
  import IconFileSpreadsheet from '@tabler/icons-svelte-runes/icons/file-spreadsheet'
  import IconFileText from '@tabler/icons-svelte-runes/icons/file-text'
  import IconFileUpload from '@tabler/icons-svelte-runes/icons/file-upload'
  import IconFolder from '@tabler/icons-svelte-runes/icons/folder'
  import IconFolderPlus from '@tabler/icons-svelte-runes/icons/folder-plus'
  import IconH1 from '@tabler/icons-svelte-runes/icons/h-1'
  import IconH2 from '@tabler/icons-svelte-runes/icons/h-2'
  import IconH3 from '@tabler/icons-svelte-runes/icons/h-3'
  import IconHandStop from '@tabler/icons-svelte-runes/icons/hand-stop'
  import IconHelpCircle from '@tabler/icons-svelte-runes/icons/help-circle'
  import IconItalic from '@tabler/icons-svelte-runes/icons/italic'
  import IconLanguage from '@tabler/icons-svelte-runes/icons/language'
  import IconLayoutSidebar from '@tabler/icons-svelte-runes/icons/layout-sidebar'
  import IconLayoutSidebarLeftCollapse from '@tabler/icons-svelte-runes/icons/layout-sidebar-left-collapse'
  import IconLink from '@tabler/icons-svelte-runes/icons/link'
  import IconList from '@tabler/icons-svelte-runes/icons/list'
  import IconListNumbers from '@tabler/icons-svelte-runes/icons/list-numbers'
  import IconLoader2 from '@tabler/icons-svelte-runes/icons/loader-2'
  import IconMapPin from '@tabler/icons-svelte-runes/icons/map-pin'
  import IconMapPinPin from '@tabler/icons-svelte-runes/icons/map-pin-pin'
  import IconMessageCircle from '@tabler/icons-svelte-runes/icons/message-circle'
  import IconMessageCirclePlus from '@tabler/icons-svelte-runes/icons/message-circle-plus'
  import IconMicrophone from '@tabler/icons-svelte-runes/icons/microphone'
  import IconMicroscope from '@tabler/icons-svelte-runes/icons/microscope'
  import IconPalette from '@tabler/icons-svelte-runes/icons/palette'
  import IconPencil from '@tabler/icons-svelte-runes/icons/pencil'
  import IconPhoto from '@tabler/icons-svelte-runes/icons/photo'
  import IconPlayerPause from '@tabler/icons-svelte-runes/icons/player-pause'
  import IconPlayerPlay from '@tabler/icons-svelte-runes/icons/player-play'
  import IconPlayerSkipBack from '@tabler/icons-svelte-runes/icons/player-skip-back'
  import IconPlayerSkipForward from '@tabler/icons-svelte-runes/icons/player-skip-forward'
  import IconPlus from '@tabler/icons-svelte-runes/icons/plus'
  import IconQuote from '@tabler/icons-svelte-runes/icons/quote'
  import IconRefresh from '@tabler/icons-svelte-runes/icons/refresh'
  import IconRotate from '@tabler/icons-svelte-runes/icons/rotate'
  import IconRotateClockwise from '@tabler/icons-svelte-runes/icons/rotate-clockwise'
  import IconSearch from '@tabler/icons-svelte-runes/icons/search'
  import IconSearchOff from '@tabler/icons-svelte-runes/icons/search-off'
  import IconSend from '@tabler/icons-svelte-runes/icons/send'
  import IconSettings from '@tabler/icons-svelte-runes/icons/settings'
  import IconSquare from '@tabler/icons-svelte-runes/icons/square'
  import IconStrikethrough from '@tabler/icons-svelte-runes/icons/strikethrough'
  import IconSuperscript from '@tabler/icons-svelte-runes/icons/superscript'
  import IconTable from '@tabler/icons-svelte-runes/icons/table'
  import IconTool from '@tabler/icons-svelte-runes/icons/tool'
  import IconTrash from '@tabler/icons-svelte-runes/icons/trash'
  import IconUnderline from '@tabler/icons-svelte-runes/icons/underline'
  import IconUnlink from '@tabler/icons-svelte-runes/icons/unlink'
  import IconVolume from '@tabler/icons-svelte-runes/icons/volume'
  import IconX from '@tabler/icons-svelte-runes/icons/x'
  import IconZoomIn from '@tabler/icons-svelte-runes/icons/zoom-in'
  import IconZoomOut from '@tabler/icons-svelte-runes/icons/zoom-out'
  import type { Component } from 'svelte'
  import type { ActionIconName, ActionIconSize } from './ActionIcon.types'
  import RotateFineCcw from './icons/RotateFineCcw.svelte'
  import RotateFineCw from './icons/RotateFineCw.svelte'

  /**
   * The props this gate passes to a glyph, and the only ones it may pass. The
   * in-house icons under ./icons declare the same shape, so they stay drop-in
   * interchangeable with a Tabler component in the map below.
   */
  type IconComponent = Component<{
    size?: number
    'aria-hidden'?: 'true'
    'data-action-icon'?: ActionIconName
  }>

  /**
   * Name -> glyph. Keyed by ActionIconName, so a name added to the catalogue
   * without an entry here is a compile error rather than a silently blank icon.
   *
   * Every choice is the Tabler glyph for the control's FUNCTION, not the one
   * that merely resembles the old drawing. Where Tabler ships no equivalent
   * compound glyph, the comment says what was approximated and why.
   */
  const ICONS: Record<ActionIconName, IconComponent> = {
    add: IconPlus,
    bell: IconBell,
    bold: IconBold,
    strikethrough: IconStrikethrough,
    table: IconTable,
    // A footnote marker is a superscript reference; Tabler has no footnote glyph.
    footnote: IconSuperscript,
    // Tabler ships no sweeping-brush glyph. This name is retired (a
    // DbBrowserView test guards against its return) and a plain brush keeps
    // the "clean up" reading.
    broom: IconBrush,
    check: IconCheck,
    'check-check': IconChecks,
    'chevron-down': IconChevronDown,
    'chevron-left': IconChevronLeft,
    'chevron-right': IconChevronRight,
    'chevron-up': IconChevronUp,
    'chevrons-left': IconChevronsLeft,
    'chevrons-right': IconChevronsRight,
    'circle-check': IconCircleCheck,
    'circle-help': IconHelpCircle,
    // Tabler ships no circled play outline. The circled caret is the same
    // affordance on the same grid, and the circle has to come from the glyph:
    // ItemCard's audio thumbnail has no circular container of its own.
    'circle-play': IconCircleCaretRight,
    'circle-x': IconCircleX,
    close: IconX,
    code: IconCode,
    copy: IconCopy,
    crop: IconCrop,
    database: IconDatabase,
    delete: IconTrash,
    download: IconDownload,
    edit: IconPencil,
    eraser: IconEraser,
    expand: IconArrowsMaximize,
    // import/export sit next to each other in the collection toolbar, so the
    // only thing that has to survive 16px is which way the arrow points:
    // export lifts data out (up), import brings it in (down). Same silhouette,
    // one axis, opposite directions.
    //
    // Tabler's own file-import/file-export are NOT that pair. Their arrows are
    // both horizontal and both point right ('h7m-3 -3l3 3l-3 3' in each), and
    // only the side they hang off the file differs, which is invisible at this
    // size.
    export: IconFileUpload,
    'external-link': IconExternalLink,
    eye: IconEye,
    'eye-off': IconEyeOff,
    file: IconFile,
    'file-audio': IconFileMusic,
    // Tabler's file-with-image glyphs all stamp an extension (JPG, PNG) on the
    // page. The asset is just an image, so the generic picture glyph is honest.
    'file-image': IconPhoto,
    // Creates a document, so it says so. Pairs with folder-plus, which is the
    // same gesture one level up in CollectionsView: a plain plus would have
    // both buttons claiming the generic "add something" affordance.
    'file-plus': IconFilePlus,
    'file-spreadsheet': IconFileSpreadsheet,
    'file-text': IconFileText,
    folder: IconFolder,
    'folder-plus': IconFolderPlus,
    hand: IconHandStop,
    'heading-1': IconH1,
    'heading-2': IconH2,
    'heading-3': IconH3,
    import: IconFileDownload,
    italic: IconItalic,
    languages: IconLanguage,
    link: IconLink,
    list: IconList,
    'list-ordered': IconListNumbers,
    loader: IconLoader2,
    'map-pin': IconMapPin,
    // Places or adjusts the selected marker. Tabler has no pin-with-pen; the
    // pin-on-pin glyph keeps the subject and reads as "set this location".
    'map-pin-pen': IconMapPinPin,
    'message-circle': IconMessageCircle,
    'message-circle-plus': IconMessageCirclePlus,
    mic: IconMicrophone,
    'panel-left': IconLayoutSidebar,
    'panel-left-close': IconLayoutSidebarLeftCollapse,
    pause: IconPlayerPause,
    play: IconPlayerPlay,
    rectangle: IconSquare,
    redo: IconArrowForwardUp,
    refresh: IconRefresh,
    // The research workspace, not a search box. A magnifier said "find a
    // string" and sat two buttons away from the toolbar's actual search field;
    // the microscope says "examine the corpus", which is what the agent does.
    research: IconMicroscope,
    // The plain rotate arrows, not the -2 variants: those draw a dotted arc,
    // which is the metric the in-house fine-rotation icons below already own.
    'rotate-ccw': IconRotate,
    'rotate-cw': IconRotateClockwise,
    'rotate-fine-ccw': RotateFineCcw,
    'rotate-fine-cw': RotateFineCw,
    save: IconDeviceFloppy,
    search: IconSearch,
    'search-x': IconSearchOff,
    send: IconSend,
    settings: IconSettings,
    'skip-back': IconPlayerSkipBack,
    'skip-forward': IconPlayerSkipForward,
    'text-quote': IconQuote,
    theme: IconPalette,
    // Toggles contrast (TopBar's toggleContrast), which the previous sun glyph
    // read as brightness. Tabler names the function outright.
    contrast: IconContrast,
    'triangle-alert': IconAlertTriangle,
    underline: IconUnderline,
    undo: IconArrowBackUp,
    unlink: IconUnlink,
    volume: IconVolume,
    wrench: IconTool,
    'zoom-in': IconZoomIn,
    'zoom-out': IconZoomOut,
  }

  let { name, size = 16 }: { name: ActionIconName; size?: ActionIconSize } = $props()

  const Icon = $derived(ICONS[name])
</script>

<!--
  Both attributes are set here rather than left to the icon library:

  - aria-hidden, because Tabler does not add it (Lucide did) and the icon must
    never carry the accessible name — that belongs on the wrapping button.
  - data-action-icon, so a test can assert WHICH icon rendered without reaching
    for the library's own class name. A DocumentViewer test querying
    `svg.lucide-rotate-ccw` is exactly what made this family swap a breaking
    change; the attribute outlives the next one.

  ActionIcon.test.ts asserts both across the whole catalogue, so the guarantee
  is checked once rather than repeated per glyph.
-->
<Icon {size} aria-hidden="true" data-action-icon={name} />
