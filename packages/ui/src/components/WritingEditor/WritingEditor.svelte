<script lang="ts">
  import { Editor } from '@tiptap/core'
  import { onDestroy, onMount, untrack } from 'svelte'
  import ActionIcon from '../Button/ActionIcon.svelte'
  import Button from '../Button/Button.svelte'
  import IconButton from '../IconButton/IconButton.svelte'
  import SearchBar from '../SearchBar/SearchBar.svelte'
  import ToolbarMenu from '../ToolbarMenu/ToolbarMenu.svelte'
  import ToolbarMenuList from '../ToolbarMenu/ToolbarMenuList.svelte'
  import type { ToolbarMenuItem } from '../ToolbarMenu/ToolbarMenu.types'
  import { fitToolbar } from './toolbar-fit'
  import {
    overflowSections,
    TOOLBAR_ROWS,
    type OverflowSection,
    type ToolbarGroup,
    type ToolbarPalette,
    type ToolbarRow,
  } from './toolbar-groups'
  import ColorPalette from './ColorPalette.svelte'
  import { colorLabelKey, parseWritingColor, type WritingColor } from './writing-colors'
  import { createDictation } from '../Dictation/dictation.svelte'
  import { createWritingExtensions } from './extensions'
  import type { TextCase } from './text-case'
  import { paragraphFormatOf, type LineHeight, type TextAlignment } from './paragraph-format'
  import {
    goToMatch,
    replaceAll,
    replaceCurrent,
    setSearch,
    type SearchState,
  } from './search-highlight'
  import { nextMatchIndex } from './search'
  import { deleteSection, insertSectionAfter, moveSection, renameSection } from './section-commands'
  import { sectionWeight, type SectionWeight } from './sections'
  import { newCitationId } from './unique-citation-ids'
  import { citeWork } from './citation-cluster'
  import { applySuggestion, insertBelow, locateText } from './apply-suggestion'
  import {
    WRITING_SCHEMA_VERSION,
    validateCanonical,
    type CanonicalDocument,
    type ValidationFailure,
  } from './document-contract'
  import {
    DEFAULT_WRITING_EDITOR_LABELS,
    refusalMessage,
    type WritingEditorProps,
  } from './WritingEditor.types'

  let {
    document: canonical,
    onchange,
    onready,
    editable = true,
    toolbar = true,
    oncitation,
    onnotelink,
    onzoterocitation,
    placeholder = '',
    resolveImage,
    importImage,
    oninsertimage,
    ondictate,
    ondictationlog,
    dictationMaxSeconds = 300,
    labels: labelOverrides,
  }: WritingEditorProps = $props()

  const labels = $derived({ ...DEFAULT_WRITING_EDITOR_LABELS, ...labelOverrides })

  let editorElement: HTMLDivElement | undefined = $state(undefined)
  let editor: Editor | undefined
  let refusal: ValidationFailure | null = $state(null)
  /** A construction failure. Six rounds of debugging came from this being
   *  silent: the editor threw, the box rendered empty, and nothing said why. */
  let buildError: string | null = $state(null)
  /** What we last emitted, so an echo back through `document` is a no-op. */
  let lastEmitted = ''

  /**
   * What the caret is currently inside. Tiptap's editor is not a Svelte store,
   * so its state is mirrored here and refreshed on every transaction — that is
   * what lets the toolbar show which formats are on.
   */
  let active = $state({
    bold: false,
    italic: false,
    underline: false,
    strike: false,
    code: false,
    h1: false,
    h2: false,
    h3: false,
    bulletList: false,
    orderedList: false,
    blockquote: false,
    link: false,
    inTable: false,
    subscript: false,
    superscript: false,
    textColor: null as WritingColor | null,
    highlight: null as WritingColor | null,
    canGrow: false,
    canShrink: false,
    /** Whether the caret is where paragraph formatting applies (not a note). */
    applicable: true,
    /** Shared by every selected block; null when they differ. */
    alignment: 'left' as TextAlignment | null,
    lineHeight: 'default' as LineHeight | 'default' | null,
    canIndent: false,
    canOutdent: false,
    hasSelection: false,
    canUndo: false,
    canRedo: false,
  })

  let linkDraft: string | null = $state(null)

  function refreshActive() {
    if (!editor) return
    active = {
      bold: editor.isActive('bold'),
      italic: editor.isActive('italic'),
      underline: editor.isActive('underline'),
      strike: editor.isActive('strike'),
      code: editor.isActive('code'),
      h1: editor.isActive('heading', { level: 1 }),
      h2: editor.isActive('heading', { level: 2 }),
      h3: editor.isActive('heading', { level: 3 }),
      bulletList: editor.isActive('bulletList'),
      orderedList: editor.isActive('orderedList'),
      blockquote: editor.isActive('blockquote'),
      link: editor.isActive('link'),
      inTable: editor.isActive('table'),
      subscript: editor.isActive('subscript'),
      superscript: editor.isActive('superscript'),
      // A name this build does not know draws as no colour, so it reads as none.
      textColor: parseWritingColor(editor.getAttributes('textStyle').color),
      highlight: editor.isActive('highlight')
        ? parseWritingColor(editor.getAttributes('highlight').color)
        : null,
      canGrow: editor.can().increaseFontSize(),
      canShrink: editor.can().decreaseFontSize(),
      ...paragraphFormatOf(editor.state),
      canIndent: editor.can().increaseIndent(),
      canOutdent: editor.can().decreaseIndent(),
      hasSelection: !editor.state.selection.empty,
      canUndo: editor.can().undo(),
      canRedo: editor.can().redo(),
    }
  }

  function buildEditor(source: CanonicalDocument) {
    if (!editorElement) return
    try {
      editor = buildEditorOn(editorElement, source)
    } catch (error) {
      // The manuscript is untouched either way, so the failure is reported
      // rather than swallowed: a blank surface with no explanation is how a
      // build error gets mistaken for a lost document.
      buildError = error instanceof Error ? error.message : String(error)
      return
    }
    onready?.()
  }

  function buildEditorOn(element: HTMLDivElement, source: CanonicalDocument): Editor {
    return new Editor({
      element,
      extensions: createWritingExtensions({
        placeholder,
        resolveImage,
        importImage,
        imageLabels: {
          alignLeft: labels.imageAlignLeft,
          alignCenter: labels.imageAlignCenter,
          alignRight: labels.imageAlignRight,
          altLabel: labels.imageAltLabel,
          resizeHandle: labels.imageResizeHandle,
          missingImage: labels.imageMissing,
          captionPlaceholder: labels.imageCaptionPlaceholder,
        },
      }),
      content: source.doc,
      editable,
      editorProps: {
        attributes: {
          class: 'writing-editor__surface',
          'aria-label': labels.editorLabel,
          'aria-multiline': 'true',
        },
        // Returning to the source (§10.2) starts here. The node is an atom, so
        // a click lands on it rather than inside it, and its attributes are the
        // whole anchor — the caller needs nothing else to resolve it.
        handleClickOn: (_view, _pos, node) => {
          if (node.type.name === 'documentCitation' && oncitation) {
            oncitation({ ...node.attrs })
            return true
          }
          // A note link is followable too (§13). Both are atoms carrying their
          // whole anchor, so a click lands on the node and the caller needs
          // nothing else to resolve it.
          if (node.type.name === 'noteLink' && onnotelink) {
            onnotelink({ ...node.attrs })
            return true
          }
          // A bibliographic citation opens its own panel rather than going
          // anywhere: its locator and affixes are what a click is for.
          if (node.type.name === 'zoteroCitation' && onzoterocitation) {
            onzoterocitation({ ...node.attrs })
            return true
          }
          return false
        },
      },
      onUpdate: ({ editor: instance }) => {
        const next: CanonicalDocument = {
          schemaVersion: WRITING_SCHEMA_VERSION,
          doc: instance.getJSON(),
        }
        const serialized = JSON.stringify(next)
        // ProseMirror normalises the document as it mounts and reports that as
        // an update. Identical JSON is not an edit, and forwarding it makes
        // every remount look like typing — which autosave then persists as a
        // new revision.
        if (serialized === lastEmitted) return
        lastEmitted = serialized
        onchange?.(next)
      },
      onTransaction: () => refreshActive(),
    })
  }

  onMount(() => {
    // Nothing is mounted without passing the contract first. Spike S1 measured
    // what happens otherwise: one unknown node or mark empties the whole
    // document, silently, and the next autosave writes that emptiness over the
    // real manuscript.
    const verdict = validateCanonical(canonical)
    if (!verdict.ok) {
      refusal = verdict
      return
    }
    lastEmitted = JSON.stringify(canonical)
    buildEditor(canonical)
    refreshActive()
  })

  /**
   * Dictation (the same capture NoteEditor has). The transcription goes in at
   * the caret, replacing any selection, as one ordinary transaction: it is one
   * undo step, and `onchange` hears about it, so autosave keeps it.
   */
  const dictation = createDictation({
    get editor() {
      return editor
    },
    get editorElement() {
      return editorElement
    },
    isEditorFocused: false,
    get ondictate() {
      return ondictate
    },
    get onlog() {
      return ondictationlog
    },
    get maxSeconds() {
      return dictationMaxSeconds
    },
    get labels() {
      return labels
    },
    logPrefix: '[WritingEditor/dictation]',
    insertion: 'caret',
  })

  onDestroy(() => {
    dictation.destroy()
    editor?.destroy()
    editor = undefined
  })

  // An external change — a recovery replay, a version restore — replaces the
  // content, but only when it is genuinely different from what we last emitted.
  $effect(() => {
    const incoming = JSON.stringify(canonical)
    if (!editor || incoming === lastEmitted) return
    const verdict = validateCanonical(canonical)
    if (!verdict.ok) {
      refusal = verdict
      return
    }
    lastEmitted = incoming
    editor.commands.setContent(canonical.doc, false)
  })

  $effect(() => {
    editor?.setEditable(editable)
  })

  export function focus() {
    editor?.commands.focus()
  }

  /** Scrolls the caret to a document position — used by the outline panel. */
  export function goToPosition(position: number) {
    editor?.chain().focus().setTextSelection(position).scrollIntoView().run()
  }

  /**
   * Inserts a corpus citation at the caret (plan-editor.md §10.1).
   *
   * The node carries the whole anchor: the asset, the page, the character range
   * and the quoted text. That is deliberate — the projection is derived from
   * the document, so anything the node does not carry does not reach the
   * database, and anything it does carry survives copy, move and undo without
   * bookkeeping.
   *
   * Returns the identity it minted, so the caller can attach the provenance
   * event that describes the same insertion.
   */
  export function insertCitation(attrs: Record<string, unknown>): string | null {
    if (!editor) return null
    const citationNodeId = newCitationId()
    const inserted = editor
      .chain()
      .focus()
      .insertContent({ type: 'documentCitation', attrs: { ...attrs, citationNodeId } })
      .run()
    return inserted ? citationNodeId : null
  }

  /**
   * Inserts a manuscript image. The app has already imported the bytes
   * (writing-images.ts) and read their size (image-dimensions.ts) by the
   * time this is called — this function only puts the node in the document,
   * exactly as insertCitation only puts the citation node in.
   *
   * `at` is the one thing a drop needs that the toolbar and a paste never
   * do: a drop lands where it was dropped, not wherever the caret happens to
   * be. Omitted, it falls through to `insertWritingImage`'s own default, the
   * current selection — the toolbar and paste's unchanged behaviour.
   */
  export function insertImage(
    attrs: {
      src: string
      alt?: string | null
      title?: string | null
      width?: number | null
      height?: number | null
      align?: 'left' | 'center' | 'right'
    },
    at?: number
  ): boolean {
    if (!editor) return false
    return editor.chain().focus().insertWritingImage(attrs, { at }).run()
  }

  /**
   * Whether a viewport point (`clientX`/`clientY`-style coordinates, the
   * same vocabulary every other pointer interaction in this component
   * already uses) falls over the manuscript surface, as opposed to the
   * toolbar or whatever sits beside this component in its host.
   *
   * The seam a Tauri-aware caller needs to scope an OS file drop to "landed
   * on the manuscript" without this package ever knowing what Tauri is
   * (apps/desktop/WritingView.svelte's `onDragDropEvent` handler is the
   * caller; see extensions.ts's removed `handleDrop` for why the browser's
   * own HTML5 drop can't do this job inside Tauri).
   */
  export function containsPoint(x: number, y: number): boolean {
    if (!editorElement) return false
    const rect = editorElement.getBoundingClientRect()
    return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom
  }

  /**
   * Maps a viewport point to a document position, for `insertImage`'s `at`.
   * `null` means the position could not be resolved — the caller falls back
   * to the caret, exactly as an omitted `at` does above; it never throws.
   */
  export function posAtCoords(x: number, y: number): number | null {
    if (!editor) return null
    const result = editor.view.posAtCoords({ left: x, top: y })
    return result ? result.pos : null
  }

  /**
   * Cites a work, joining the citation already at the caret when there is one.
   *
   * A citation is a cluster, so citing a second work beside the first must add
   * it to that cluster rather than open a new one: `(Acha, 2015; Acha, 2008)`
   * is one citation of two works, and `(Acha, 2015)(Acha, 2008)` is what you
   * get for treating them as two.
   *
   * "Beside" means immediately before the caret, which is where the previous
   * insertion left it. Anything typed in between — even a space — means the
   * writer moved on, and a new citation is then what they meant.
   *
   * Returns the cluster's identity, whether it was just minted or joined.
   */
  export function insertZoteroCitation(item: Record<string, unknown>): string | null {
    return editor ? citeWork(editor, item) : null
  }

  /** Every bibliographic citation in the manuscript, for re-rendering them. */
  export function zoteroCitations(): { id: string; attrs: Record<string, unknown> }[] {
    if (!editor) return []
    const found: { id: string; attrs: Record<string, unknown> }[] = []
    editor.state.doc.descendants((node) => {
      if (node.type.name !== 'zoteroCitation') return true
      const id = node.attrs.citationNodeId
      if (typeof id === 'string') found.push({ id, attrs: { ...node.attrs } })
      return true
    })
    return found
  }

  /**
   * Changes one citation's attributes, found by its identity rather than its
   * position.
   *
   * A position would be stale the moment anything above it changed, and a
   * citation is edited from a panel that stays open while the writer keeps
   * typing. Identity is what survives that.
   */
  export function updateZoteroCitation(
    citationNodeId: string,
    attrs: Record<string, unknown>
  ): boolean {
    if (!editor) return false
    let changed = false
    const tr = editor.state.tr
    editor.state.doc.descendants((node, pos) => {
      if (node.type.name !== 'zoteroCitation') return true
      if (node.attrs.citationNodeId !== citationNodeId) return true
      for (const [key, value] of Object.entries(attrs)) {
        tr.setNodeAttribute(pos, key, value)
      }
      changed = true
      return true
    })
    if (changed) editor.view.dispatch(tr)
    return changed
  }

  /**
   * The passage currently selected, for writing it down as a note (§13.1).
   *
   * Empty when nothing is selected, which is what disables the action: a note
   * made from no selection would be a blank note.
   */
  export function selectedText(): string {
    if (!editor) return ''
    const { from, to } = editor.state.selection
    return editor.state.doc.textBetween(from, to, ' ').trim()
  }

  /**
   * Puts a note's words in as independent text (§13).
   *
   * Plain text and nothing else: a copy has no node, no identity and no
   * relationship to the note it came from, which is precisely what makes it a
   * copy rather than a link.
   */
  export function insertNoteText(text: string): boolean {
    if (!editor || !text) return false
    return editor.chain().focus().insertContent(text).run()
  }

  /** Puts a live link to a note in, minting its identity (§13). */
  export function insertNoteLink(attrs: Record<string, unknown>): string | null {
    if (!editor) return null
    const noteLinkNodeId = newCitationId()
    const inserted = editor
      .chain()
      .focus()
      .insertContent({ type: 'noteLink', attrs: { ...attrs, noteLinkNodeId } })
      .run()
    return inserted ? noteLinkNodeId : null
  }

  /**
   * Section operations, driven from the outline panel (§6.1).
   *
   * They are exposed rather than wired to a panel here, because the outline
   * lives in the view and the document lives in the editor. Each is one
   * transaction, so each is one undo. Moves and renames go through unasked,
   * because they are visible the instant they happen and a mistake announces
   * itself. Deleting is confirmed in the view: the outline closes over the gap,
   * the writing continues, and by the time the loss is noticed the undo history
   * has moved on and autosave has persisted it.
   */
  export function renameOutlineSection(childIndex: number, title: string): boolean {
    return editor ? renameSection(editor, childIndex, title) : false
  }

  /** What deleting this section would cost, for the confirmation to report. */
  export function weighSection(childIndex: number): SectionWeight {
    return editor ? sectionWeight(editor.state.doc, childIndex) : { words: 0, headings: 0 }
  }

  export function deleteOutlineSection(childIndex: number): boolean {
    return editor ? deleteSection(editor, childIndex) : false
  }

  export function moveOutlineSection(childIndex: number, direction: 1 | -1): boolean {
    return editor ? moveSection(editor, childIndex, direction) : false
  }

  export function addSectionAfter(childIndex: number, title = ''): boolean {
    return editor ? insertSectionAfter(editor, childIndex, title) : false
  }

  /**
   * Applying an agent's proposal (plan-editor.md §14.2).
   *
   * The target is the passage itself, never a position stored when the proposal
   * was made: between proposing and accepting, the writer keeps typing, and a
   * saved range points at different words by then. So the passage is looked for
   * — and a passage that is gone, or that now occurs twice, is refused rather
   * than guessed at.
   */
  export function passageStillThere(passage: string): boolean {
    return editor ? locateText(editor, passage) !== null : false
  }

  export function replaceWithSuggestion(passage: string, proposal: string): boolean {
    return editor ? applySuggestion(editor, passage, proposal) : false
  }

  export function insertSuggestionBelow(passage: string, proposal: string): boolean {
    return editor ? insertBelow(editor, passage, proposal) : false
  }

  const chain = () => editor?.chain().focus()

  /**
   * Find and replace (plan-editor.md §26).
   *
   * The query lives here and the matches live in the editor's plugin, because
   * only the plugin can see the document change underneath a search. Nothing
   * here edits until "replace" is pressed: a search that advanced the document
   * would earn a revision for every letter typed into the field.
   */
  let searchOpen = $state(false)
  let searchQuery = $state('')
  let replaceDraft = $state('')
  let matches = $state<SearchState>({ query: '', caseSensitive: false, current: -1, matches: [] })

  function openSearch() {
    searchOpen = true
    // A selection is almost always what the writer wants to look for, so it
    // seeds the field rather than making them type it again. Only within one
    // block: a selection spanning paragraphs is a passage, not a query.
    const selection = editor?.state.selection
    if (selection && selection.$from.parent === selection.$to.parent) {
      const selected = editor?.state.doc.textBetween(selection.from, selection.to)
      if (selected) searchQuery = selected
    }
    runSearch(searchQuery)
  }

  function closeSearch() {
    searchOpen = false
    if (editor) matches = setSearch(editor, { query: '', current: -1 })
    editor?.commands.focus()
  }

  function runSearch(query: string) {
    searchQuery = query
    if (!editor) return
    const next = setSearch(editor, { query, current: query ? 0 : -1 })
    matches = next.matches.length > 0 ? goToMatch(editor, 0) : next
  }

  function step(direction: 1 | -1) {
    if (!editor) return
    matches = goToMatch(editor, nextMatchIndex(matches.matches, matches.current, direction))
  }

  function replaceOne() {
    if (!editor) return
    matches = replaceCurrent(editor, replaceDraft)
  }

  function replaceEvery() {
    if (!editor) return
    matches = replaceAll(editor, replaceDraft)
  }

  function onSearchKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault()
      closeSearch()
      return
    }
    if (event.key === 'Enter') {
      event.preventDefault()
      step(event.shiftKey ? -1 : 1)
    }
  }

  /** Ctrl+F from anywhere inside the editor, including the manuscript. */
  function onRootKeydown(event: KeyboardEvent) {
    if (!(event.ctrlKey || event.metaKey) || event.key.toLowerCase() !== 'f') return
    event.preventDefault()
    if (searchOpen) closeSearch()
    else openSearch()
  }

  function insertFootnote() {
    chain()?.addFootnote().run()
  }

  /**
   * The language the case rules follow: the one the app declares on <html>
   * (i18n sets it), so a Turkish i or a German ß capitalizes as it should.
   */
  function caseLocale(): string | undefined {
    return editorElement?.closest('[lang]')?.getAttribute('lang') || undefined
  }

  function setCase(mode: TextCase) {
    chain()?.setTextCase(mode, caseLocale()).run()
  }

  /** A colour, or none, onto the selection — or onto what is typed next. */
  function applyTextColor(name: WritingColor | null) {
    if (name) chain()?.setTextColor(name).run()
    else chain()?.unsetTextColor().run()
  }

  function applyHighlight(name: WritingColor | null) {
    if (name) chain()?.setHighlight({ color: name }).run()
    else chain()?.unsetHighlight().run()
  }

  const colorName = (name: WritingColor) => labels[colorLabelKey(name)]

  /** One of the four alignments: a radio set, left on when none is stored. */
  const alignmentTool = (alignment: TextAlignment, label: string) => ({
    id: `align-${alignment}`,
    label,
    icon: `align-${alignment}` as const,
    active: active.alignment === alignment,
    radio: true,
    disabled: !active.applicable,
    run: () => chain()?.setTextAlign(alignment).run(),
  })

  /** Line spacing's choices; the stored values are fixed, the labels localized. */
  const lineHeightItems: ToolbarMenuItem[] = $derived(
    (
      [
        ['1', labels.lineHeight1],
        ['1.15', labels.lineHeight115],
        ['1.5', labels.lineHeight15],
        ['2', labels.lineHeight2],
        ['default', labels.lineHeightDefault],
      ] as const
    ).map(([value, label]) => ({
      id: `lineHeight-${value}`,
      kind: 'radio' as const,
      label,
      checked: active.lineHeight === value,
      onselect: () =>
        chain()
          ?.setLineHeight(value === 'default' ? null : value)
          .run(),
    }))
  )

  function openLinkField() {
    if (!editor) return
    if (active.link) {
      chain()?.unsetLink().run()
      return
    }
    linkDraft = ''
  }

  function commitLink(value: string) {
    const href = value.trim()
    linkDraft = null
    if (!href) return
    chain()?.setLink({ href }).run()
  }

  /**
   * The toolbar, as groups (toolbar-groups.ts), on two rows: the original
   * tools on the first, the formatting tools (typography, paragraph) on the
   * second.
   *
   * Priorities say which groups give way first when their row runs out of
   * room, the least used in a manuscript first. On the first row:
   * strike-through and inline code, then lists and quotes, then links, tables
   * and footnotes, and headings last — they are the document's structure and
   * what the outline is built from. On the second: typography (size, case, sub
   * and superscript, clear formatting, colours) before paragraph formatting
   * (indent, alignment, line spacing).
   * History, bold/italic/underline, and find with the microphone never
   * collapse. Strike and code are their own group only so they can go before
   * the basic marks; `joined` keeps them in one run with those, as today.
   */
  const toolbarGroups: ToolbarGroup[] = $derived([
    {
      id: 'history',
      row: 'first',
      priority: 'pinned',
      tools: [
        {
          id: 'undo',
          label: labels.undo,
          icon: 'undo',
          disabled: !active.canUndo,
          run: () => chain()?.undo().run(),
        },
        {
          id: 'redo',
          label: labels.redo,
          icon: 'redo',
          disabled: !active.canRedo,
          run: () => chain()?.redo().run(),
        },
      ],
    },
    {
      id: 'marks',
      row: 'first',
      priority: 'pinned',
      tools: [
        {
          id: 'bold',
          label: labels.bold,
          icon: 'bold',
          active: active.bold,
          run: () => chain()?.toggleBold().run(),
        },
        {
          id: 'italic',
          label: labels.italic,
          icon: 'italic',
          active: active.italic,
          run: () => chain()?.toggleItalic().run(),
        },
        {
          id: 'underline',
          label: labels.underline,
          icon: 'underline',
          active: active.underline,
          run: () => chain()?.toggleUnderline().run(),
        },
      ],
    },
    {
      id: 'marksExtra',
      row: 'first',
      priority: 2,
      joined: true,
      tools: [
        {
          id: 'strike',
          label: labels.strike,
          icon: 'strikethrough',
          active: active.strike,
          run: () => chain()?.toggleStrike().run(),
        },
        {
          id: 'code',
          label: labels.code,
          icon: 'code',
          active: active.code,
          run: () => chain()?.toggleCode().run(),
        },
      ],
    },
    {
      id: 'headings',
      row: 'first',
      priority: 5,
      tools: [
        {
          id: 'heading1',
          label: labels.heading1,
          icon: 'heading-1',
          active: active.h1,
          run: () => chain()?.toggleHeading({ level: 1 }).run(),
        },
        {
          id: 'heading2',
          label: labels.heading2,
          icon: 'heading-2',
          active: active.h2,
          run: () => chain()?.toggleHeading({ level: 2 }).run(),
        },
        {
          id: 'heading3',
          label: labels.heading3,
          icon: 'heading-3',
          active: active.h3,
          run: () => chain()?.toggleHeading({ level: 3 }).run(),
        },
      ],
    },
    {
      id: 'lists',
      row: 'first',
      priority: 3,
      tools: [
        {
          id: 'bulletList',
          label: labels.bulletList,
          icon: 'list',
          active: active.bulletList,
          run: () => chain()?.toggleBulletList().run(),
        },
        {
          id: 'orderedList',
          label: labels.orderedList,
          icon: 'list-ordered',
          active: active.orderedList,
          run: () => chain()?.toggleOrderedList().run(),
        },
        {
          id: 'blockquote',
          label: labels.blockquote,
          icon: 'text-quote',
          active: active.blockquote,
          run: () => chain()?.toggleBlockquote().run(),
        },
      ],
    },
    {
      id: 'insert',
      row: 'first',
      priority: 4,
      tools: [
        {
          id: 'link',
          label: active.link ? labels.unlink : labels.link,
          icon: active.link ? 'unlink' : 'link',
          active: active.link,
          run: openLinkField,
        },
        {
          id: 'table',
          label: labels.table,
          icon: 'table',
          disabled: active.inTable,
          run: () => chain()?.insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run(),
        },
        { id: 'footnote', label: labels.footnote, icon: 'footnote', run: insertFootnote },
        ...(oninsertimage
          ? [
              {
                id: 'insertImage',
                label: labels.insertImage,
                icon: 'insert-image' as const,
                run: oninsertimage,
              },
            ]
          : []),
      ],
    },
    {
      id: 'typography',
      row: 'second',
      priority: 0,
      tools: [
        {
          id: 'fontSizeIncrease',
          label: labels.fontSizeIncrease,
          icon: 'text-increase',
          disabled: !active.canGrow,
          run: () => chain()?.increaseFontSize().run(),
        },
        {
          id: 'fontSizeDecrease',
          label: labels.fontSizeDecrease,
          icon: 'text-decrease',
          disabled: !active.canShrink,
          run: () => chain()?.decreaseFontSize().run(),
        },
        {
          id: 'changeCase',
          label: labels.changeCase,
          icon: 'letter-case',
          // Case is a change to words already written, so there has to be some.
          disabled: !active.hasSelection,
          menu: [
            { id: 'caseUpper', label: labels.caseUpper, onselect: () => setCase('upper') },
            { id: 'caseLower', label: labels.caseLower, onselect: () => setCase('lower') },
            { id: 'caseSentence', label: labels.caseSentence, onselect: () => setCase('sentence') },
            { id: 'caseWords', label: labels.caseWords, onselect: () => setCase('words') },
          ],
          // Never called: the button opens the menu, and the overflow menu
          // lists the entries themselves.
          run: () => {},
        },
        {
          id: 'subscript',
          label: labels.subscript,
          icon: 'subscript',
          active: active.subscript,
          run: () => chain()?.toggleSubscript().run(),
        },
        {
          id: 'superscript',
          label: labels.superscript,
          icon: 'superscript',
          active: active.superscript,
          run: () => chain()?.toggleSuperscript().run(),
        },
        {
          id: 'clearFormatting',
          label: labels.clearFormatting,
          icon: 'clear-formatting',
          run: () => chain()?.clearFormatting().run(),
        },
        {
          id: 'highlight',
          label: labels.highlight,
          icon: 'highlight',
          active: active.highlight !== null,
          palette: { kind: 'highlight', current: active.highlight, apply: applyHighlight },
          // Never called: the button opens the palette.
          run: () => {},
        },
        {
          id: 'textColor',
          label: labels.textColor,
          icon: 'text-color',
          active: active.textColor !== null,
          palette: { kind: 'text', current: active.textColor, apply: applyTextColor },
          run: () => {},
        },
      ],
    },
    {
      id: 'paragraph',
      row: 'second',
      priority: 1,
      tools: [
        {
          id: 'indentDecrease',
          label: labels.indentDecrease,
          icon: 'indent-decrease',
          disabled: !active.canOutdent,
          run: () => chain()?.decreaseIndent().run(),
        },
        {
          id: 'indentIncrease',
          label: labels.indentIncrease,
          icon: 'indent-increase',
          disabled: !active.canIndent,
          run: () => chain()?.increaseIndent().run(),
        },
        alignmentTool('left', labels.alignLeft),
        alignmentTool('center', labels.alignCenter),
        alignmentTool('right', labels.alignRight),
        alignmentTool('justify', labels.alignJustify),
        {
          id: 'lineHeight',
          label: labels.lineHeight,
          icon: 'line-height',
          disabled: !active.applicable,
          menu: lineHeightItems,
          menuHeading: true,
          // Never called: the button opens the menu.
          run: () => {},
        },
      ],
    },
    {
      id: 'utility',
      row: 'first',
      priority: 'pinned',
      joined: true,
      tools: [
        {
          id: 'find',
          label: labels.find,
          icon: 'search',
          active: searchOpen,
          run: () => (searchOpen ? closeSearch() : openSearch()),
        },
        ...(ondictate
          ? [
              {
                id: 'dictate',
                label: dictation.buttonLabel,
                icon: 'mic' as const,
                variant: dictation.state === 'recording' ? ('danger' as const) : ('ghost' as const),
                disabled: dictation.state === 'transcribing',
                // The caret the text is meant for stays where the writer left it.
                keepFocus: true,
                separated: true,
                run: dictation.toggle,
              },
            ]
          : []),
      ],
    },
  ])

  /**
   * Overflow, row by row: each row fits its own groups and has its own menu.
   * On the first row the trigger goes before the utility group, so find and
   * the microphone keep the end of the row; on the second it goes last.
   * Groups are measured while they are on their row and remembered, which is
   * what lets a collapsed one be priced without showing it — and what keeps
   * the decision from feeding back into itself: no group's width depends on
   * which others are showing.
   */
  const OVERFLOW_BEFORE: Record<ToolbarRow, string | undefined> = {
    first: 'utility',
    second: undefined,
  }
  let toolbarElement: HTMLDivElement | undefined = $state(undefined)
  let hiddenGroups = $state<Record<ToolbarRow, string[]>>({ first: [], second: [] })
  const groupWidths = new Map<string, number>()
  let overflowWidth = 0
  let toolbarObserver: ResizeObserver | undefined

  const toolbarRows = $derived(
    TOOLBAR_ROWS.map((row) => {
      const groups = toolbarGroups.filter((group) => group.row === row)
      const hidden = hiddenGroups[row]
      const visible = groups.filter((group) => !hidden.includes(group.id))
      const before = OVERFLOW_BEFORE[row]
      return {
        id: row,
        visible,
        hidden,
        overflow: overflowSections(groups, hidden),
        // Where the trigger goes: before that group when it is showing, else last.
        overflowBefore:
          hidden.length > 0 && visible.some((group) => group.id === before) ? before : undefined,
      }
    })
  )

  function px(value: string | undefined, fallback: number): number {
    const parsed = Number.parseFloat(value ?? '')
    return Number.isFinite(parsed) ? parsed : fallback
  }

  function fitToolbarRow(toolbar: HTMLElement, row: ToolbarRow) {
    const element = toolbar.querySelector<HTMLElement>(`[data-toolbar-row="${row}"]`)
    if (!element) return
    const groups = toolbarGroups.filter((group) => group.row === row)
    const measured = groups.map((group) => ({
      id: group.id,
      priority: group.priority,
      joined: group.joined,
      width: groupWidths.get(group.id),
    }))
    // A group never seen on the row has no width yet: show the whole row once
    // so it is measured, and decide on the next pass.
    if (measured.some((group) => group.width === undefined)) {
      if (hiddenGroups[row].length > 0) hiddenGroups[row] = []
      return
    }

    const style = getComputedStyle(element)
    // Separators and buttons are alike on both rows: take them from whichever
    // row shows one.
    const separator = toolbar.querySelector<HTMLElement>('.writing-editor__sep')
    const separatorStyle = separator ? getComputedStyle(separator) : undefined
    const fit = fitToolbar({
      groups: measured.map((group) => ({ ...group, width: group.width ?? 0 })),
      available: element.clientWidth - px(style.paddingLeft, 0) - px(style.paddingRight, 0),
      gap: px(style.columnGap, 4),
      separatorWidth: separator
        ? separator.getBoundingClientRect().width +
          px(separatorStyle?.marginLeft, 4) +
          px(separatorStyle?.marginRight, 4)
        : 9,
      // Before a trigger has ever been shown, any toolbar button is its size.
      overflowWidth:
        overflowWidth || toolbar.querySelector('button')?.getBoundingClientRect().width || 28,
      overflowBefore: OVERFLOW_BEFORE[row],
    })
    if (fit.hidden.join() !== hiddenGroups[row].join()) hiddenGroups[row] = fit.hidden
  }

  function fitToolbarGroups() {
    const toolbar = toolbarElement
    if (!toolbar) return
    for (const node of toolbar.querySelectorAll<HTMLElement>('[data-toolbar-group]')) {
      groupWidths.set(node.dataset.toolbarGroup ?? '', node.getBoundingClientRect().width)
    }
    const trigger = toolbar.querySelector<HTMLElement>('[data-toolbar-overflow]')
    if (trigger) overflowWidth = trigger.getBoundingClientRect().width
    for (const row of TOOLBAR_ROWS) fitToolbarRow(toolbar, row)
  }

  $effect(() => {
    const toolbar = toolbarElement
    if (!toolbar || typeof ResizeObserver === 'undefined') return
    // Each row (panel resize, window zoom) and every group on it (the
    // dictation timer widens the utility group while it records).
    const observer = new ResizeObserver(() => fitToolbarGroups())
    toolbarObserver = observer
    for (const node of toolbar.querySelectorAll<HTMLElement>(
      '[data-toolbar-row], [data-toolbar-group]'
    )) {
      observer.observe(node)
    }
    untrack(fitToolbarGroups)
    return () => {
      observer.disconnect()
      toolbarObserver = undefined
    }
  })

  function observeGroup(node: HTMLElement) {
    toolbarObserver?.observe(node)
    return () => toolbarObserver?.unobserve(node)
  }

  function onLinkKeydown(event: KeyboardEvent & { currentTarget: HTMLInputElement }) {
    if (event.key === 'Enter') {
      event.preventDefault()
      commitLink(event.currentTarget.value)
    }
    if (event.key === 'Escape') {
      event.preventDefault()
      linkDraft = null
      editor?.commands.focus()
    }
  }
</script>

{#if refusal}
  <div class="writing-editor writing-editor--refused" role="alert">
    <p class="writing-editor__refused-title">{labels.refusedTitle}</p>
    <p class="writing-editor__refused-body">{refusalMessage(refusal, labels)}</p>
  </div>
{:else if buildError}
  <div class="writing-editor writing-editor--refused" role="alert">
    <p class="writing-editor__refused-title">{labels.buildFailedTitle}</p>
    <p class="writing-editor__refused-body">{labels.buildFailedBody}</p>
    <p class="writing-editor__refused-detail">{buildError}</p>
  </div>
{:else}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <div class="writing-editor" role="group" onkeydown={onRootKeydown}>
    {#if toolbar}
      <!-- Two rows, each on one line: groups that do not fit collapse into
           that row's overflow menu (see fitToolbarGroups). One toolbar, not
           two: the rows are a line break, not a second set of controls, so
           they share one name and one Tab order. Every button carries its
           label as a tooltip through the shared system, never a native
           title. -->
      <div
        class="writing-editor__toolbar"
        role="toolbar"
        aria-label={labels.toolbarLabel}
        bind:this={toolbarElement}
      >
        {#each toolbarRows as row (row.id)}
          <div class="writing-editor__toolbar-row" data-toolbar-row={row.id}>
            {#each row.visible as group, index (group.id)}
              {#if row.overflowBefore === group.id}
                <span class="writing-editor__sep" aria-hidden="true"></span>
                {@render overflowMenu(row.overflow)}
              {/if}
              {#if index > 0 && !group.joined}
                <span class="writing-editor__sep" aria-hidden="true"></span>
              {/if}
              <span
                class="writing-editor__group"
                data-toolbar-group={group.id}
                {@attach observeGroup}
              >
                {#each group.tools as tool (tool.id)}
                  {#if tool.separated}
                    <span class="writing-editor__sep" aria-hidden="true"></span>
                  {/if}
                  {#if tool.palette}
                    {@const palette = tool.palette}
                    <ToolbarMenu label={tool.label}>
                      {#snippet trigger(props, { open })}
                        <IconButton
                          size="sm"
                          variant="ghost"
                          label={tool.label}
                          title={tool.label}
                          active={open || tool.active}
                          disabled={tool.disabled}
                          {...props}><ActionIcon name={tool.icon} size={14} /></IconButton
                        >
                      {/snippet}
                      {#snippet children({ close })}
                        {@render colorPalette(tool.label, palette, false, close)}
                      {/snippet}
                    </ToolbarMenu>
                  {:else if tool.menu}
                    <ToolbarMenu label={tool.label} items={tool.menu}>
                      {#snippet trigger(props, { open })}
                        <IconButton
                          size="sm"
                          variant="ghost"
                          label={tool.label}
                          title={tool.label}
                          active={open}
                          disabled={tool.disabled}
                          {...props}><ActionIcon name={tool.icon} size={14} /></IconButton
                        >
                      {/snippet}
                    </ToolbarMenu>
                  {:else}
                    <IconButton
                      size="sm"
                      variant={tool.variant ?? 'ghost'}
                      label={tool.label}
                      title={tool.label}
                      active={tool.active}
                      disabled={tool.disabled}
                      onmousedown={tool.keepFocus ? (event) => event.preventDefault() : undefined}
                      onclick={tool.run}><ActionIcon name={tool.icon} size={14} /></IconButton
                    >
                  {/if}
                  {#if tool.id === 'dictate' && (dictation.state === 'recording' || dictation.state === 'transcribing')}
                    <span
                      class="writing-editor__dictation-status"
                      class:writing-editor__dictation-status--recording={dictation.state ===
                        'recording'}
                      data-testid="writing-editor-dictation-timer"
                    >
                      {dictation.state === 'recording'
                        ? dictation.timerLabel
                        : labels.dictationProcessing}
                    </span>
                  {/if}
                {/each}
              </span>
            {/each}
            {#if row.hidden.length > 0 && row.overflowBefore === undefined}
              {#if row.visible.length > 0}
                <span class="writing-editor__sep" aria-hidden="true"></span>
              {/if}
              {@render overflowMenu(row.overflow)}
            {/if}
          </div>
        {/each}
      </div>

      {#snippet overflowMenu(sections: OverflowSection[])}
        <ToolbarMenu label={labels.moreTools}>
          {#snippet trigger(props, { open })}
            <IconButton
              size="sm"
              variant="ghost"
              label={labels.moreTools}
              title={labels.moreTools}
              active={open}
              data-toolbar-overflow=""
              {...props}><ActionIcon name="more" size={14} /></IconButton
            >
          {/snippet}
          {#snippet children({ close, select })}
            <!-- In toolbar order. The palettes are drawn whole, headed, rather
                 than as a menu that opens a menu: the grid is two rows, a list
                 would be nine. -->
            {#each sections as section, index (section.kind === 'items' ? section.id : section.tool.id)}
              {#if section.kind === 'items'}
                <ToolbarMenuList items={section.items} onselect={select} />
              {:else}
                {#if index > 0}
                  <div class="writing-editor__menu-separator" role="separator"></div>
                {/if}
                {@render colorPalette(section.tool.label, section.tool.palette, true, close)}
              {/if}
            {/each}
          {/snippet}
        </ToolbarMenu>
      {/snippet}

      {#snippet colorPalette(
        label: string,
        palette: ToolbarPalette,
        heading: boolean,
        close: (options?: { returnFocus?: boolean }) => void
      )}
        <ColorPalette
          {label}
          {heading}
          kind={palette.kind}
          current={palette.current}
          noColorLabel={labels.noColor}
          colorLabel={colorName}
          onpick={(name) => {
            // The command takes the focus back to the text; the menu must not
            // hand it to its button first.
            close({ returnFocus: false })
            palette.apply(name)
          }}
        />
      {/snippet}

      {#if ondictate && dictation.message}
        <p
          class="writing-editor__dictation-message"
          class:writing-editor__dictation-message--error={dictation.state === 'error'}
          role="status"
          data-testid="writing-editor-dictation-message"
        >
          {dictation.message}
        </p>
      {/if}

      {#if active.inTable}
        <div class="writing-editor__table-row" role="group" aria-label={labels.tableControls}>
          <Button size="sm" variant="ghost" onclick={() => chain()?.addRowAfter().run()}>
            {labels.addRow}
          </Button>
          <Button size="sm" variant="ghost" onclick={() => chain()?.addColumnAfter().run()}>
            {labels.addColumn}
          </Button>
          <Button size="sm" variant="ghost" onclick={() => chain()?.deleteRow().run()}>
            {labels.deleteRow}
          </Button>
          <Button size="sm" variant="ghost" onclick={() => chain()?.deleteColumn().run()}>
            {labels.deleteColumn}
          </Button>
          <Button size="sm" variant="danger" onclick={() => chain()?.deleteTable().run()}>
            {labels.deleteTable}
          </Button>
        </div>
      {/if}

      {#if linkDraft !== null}
        <!-- A one-field disclosure rather than a dialog: the shared modal shell
             does not exist yet, and a link does not warrant inventing one. -->
        <div class="writing-editor__link-row">
          <input
            class="writing-editor__link-input"
            type="url"
            inputmode="url"
            placeholder="https://"
            aria-label={labels.link}
            onkeydown={onLinkKeydown}
            onblur={(event) => commitLink(event.currentTarget.value)}
            {@attach (node) => node.focus()}
          />
        </div>
      {/if}
    {/if}
    {#if searchOpen}
      <div class="writing-editor__search" role="search">
        <!-- The shared field, not a hand-rolled input: it carries the
             magnifier every search field in this app owes, and the aesthetic
             guard enforces that. `debounceMs` is zero because the matches are
             the feedback — waiting 300ms to highlight what was just typed
             reads as the search being broken. -->
        <div class="writing-editor__search-field">
          <SearchBar
            value={searchQuery}
            debounceMs={0}
            emitSearch={false}
            ariaLabel={labels.find}
            placeholder={labels.find}
            clearAriaLabel={labels.closeSearch}
            onvaluechange={(query) => runSearch(query)}
            onkeydown={onSearchKeydown}
            inputRef={(node) => node?.focus()}
          />
        </div>
        <!-- Numbers and a slash read the same in every language, so the count
             needs no string of its own; only its absence does. -->
        <span class="writing-editor__search-count" role="status">
          {#if matches.matches.length === 0}
            {searchQuery ? labels.noMatches : ''}
          {:else}
            {matches.current + 1} / {matches.matches.length}
          {/if}
        </span>
        <IconButton
          size="sm"
          variant="ghost"
          label={labels.findPrevious}
          disabled={matches.matches.length === 0}
          onclick={() => step(-1)}><ActionIcon name="chevron-up" size={14} /></IconButton
        >
        <IconButton
          size="sm"
          variant="ghost"
          label={labels.findNext}
          disabled={matches.matches.length === 0}
          onclick={() => step(1)}><ActionIcon name="chevron-down" size={14} /></IconButton
        >

        <input
          class="writing-editor__link-input writing-editor__replace-field"
          type="text"
          bind:value={replaceDraft}
          aria-label={labels.replace}
          placeholder={labels.replace}
          onkeydown={onSearchKeydown}
        />
        <Button variant="ghost" size="sm" disabled={matches.current < 0} onclick={replaceOne}>
          {labels.replaceOne}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          disabled={matches.matches.length === 0}
          onclick={replaceEvery}>{labels.replaceAll}</Button
        >
        <IconButton size="sm" variant="ghost" label={labels.closeSearch} onclick={closeSearch}>
          <ActionIcon name="close" size={14} />
        </IconButton>
      </div>
    {/if}

    <!-- No role or tabindex here: ProseMirror builds its own contenteditable
         inside this element and carries the accessible name (see editorProps).
         A focusable wrapper would take the focus without being editable. -->
    <div bind:this={editorElement} class="writing-editor__host"></div>
  </div>
{/if}

<style>
  .writing-editor {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
    background: var(--color-surface);
    color: var(--color-text-primary);
  }

  /* Two rows, one under the other, the second a gap below the first: the same
     step as the padding above and below them, so the rhythm stays even.
     `overflow: hidden` is the backstop, not the mechanism: each row's overflow
     menu keeps it inside its width, and the padding is wider than the focus
     ring, so nothing on a row is ever clipped by it. */
  .writing-editor__toolbar {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
    overflow: hidden;
    padding: var(--space-1) var(--space-2);
    border-bottom: 1px solid var(--border-subtle);
    background: var(--surface-toolbar);
  }

  /* One line each, never wrapped; no clipping of its own, so a focus ring
     can spill into the gap between the rows. */
  .writing-editor__toolbar-row {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex-wrap: nowrap;
    min-width: 0;
  }

  /* A group is measured as one box; inside, the same gap as the row, so the
     buttons sit exactly where they did as direct children of it. */
  .writing-editor__group {
    display: flex;
    align-items: center;
    flex-shrink: 0;
    gap: var(--space-1);
  }

  .writing-editor__sep {
    flex-shrink: 0;
    width: 1px;
    height: 16px;
    margin: 0 var(--space-1);
    background: var(--border-subtle);
  }

  /* The overflow menu's own separator, for the palettes drawn inside it. */
  .writing-editor__menu-separator {
    height: 1px;
    margin: 0 var(--space-1);
    background: var(--border-subtle);
  }

  .writing-editor__dictation-status {
    padding: 0 var(--space-1);
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    font-variant-numeric: tabular-nums;
  }

  .writing-editor__dictation-status--recording {
    color: var(--color-danger);
  }

  .writing-editor__dictation-message {
    margin: 0;
    padding: var(--space-1) var(--space-2);
    border-bottom: 1px solid var(--border-subtle);
    background: var(--surface-toolbar);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .writing-editor__dictation-message--error {
    color: var(--color-danger);
  }

  .writing-editor__table-row {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex-wrap: wrap;
    padding: var(--space-1) var(--space-2);
    border-bottom: 1px solid var(--border-subtle);
    background: var(--surface-toolbar);
  }

  .writing-editor__search {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex-wrap: wrap;
    padding: var(--space-1) var(--space-2);
    border-bottom: 1px solid var(--border-subtle);
    background: var(--surface-toolbar);
  }

  .writing-editor__search-field,
  .writing-editor__replace-field {
    flex: 1 1 14ch;
    width: auto;
    min-width: 0;
    max-width: 28ch;
  }

  .writing-editor__search-count {
    min-width: 8ch;
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    font-variant-numeric: tabular-nums;
    text-align: center;
  }

  .writing-editor__link-row {
    padding: var(--space-1) var(--space-2);
    border-bottom: 1px solid var(--border-subtle);
    background: var(--surface-toolbar);
  }

  .writing-editor__link-input {
    width: 100%;
    max-width: 48ch;
    min-height: 28px;
    padding: 0 var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-input);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font: inherit;
    font-size: var(--font-size-sm);
  }

  .writing-editor__link-input:focus-visible {
    outline: none;
    border-color: var(--border-focus);
    box-shadow: var(--focus-ring);
  }

  /* A block, not a flex row. As a flex container it constrained the editing
     surface to its own height, so a manuscript longer than the viewport spilled
     out of a box that would not grow. */
  .writing-editor__host {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-6) var(--space-5);
  }

  .writing-editor--refused {
    gap: var(--space-2);
    padding: var(--space-5);
    justify-content: center;
    align-items: center;
    text-align: center;
  }

  .writing-editor__refused-title {
    margin: 0;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-medium);
  }

  .writing-editor__refused-detail {
    margin: 0;
    max-width: 60ch;
    color: var(--color-text-muted);
    font-family: var(--font-mono);
    font-size: var(--font-size-2xs);
  }

  .writing-editor__refused-body {
    margin: 0;
    max-width: 48ch;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
    line-height: var(--line-height-base);
  }

  /* The editing surface is created by ProseMirror, so its styles cannot be
     scoped by Svelte and are declared globally under this component's class. */
  :global(.writing-editor__surface) {
    /* An empty document is one empty paragraph — a single line high — so this
       gives the whole box something to click into. It is a minimum: the surface
       still grows past it as the manuscript does. */
    min-height: 100%;
    box-sizing: border-box;
    max-width: 78ch;
    margin: 0 auto;
    outline: none;
    /* The manuscript is read, not operated: it takes the reading face. */
    font-family: var(--font-reading);
    font-size: var(--font-size-md);
    line-height: var(--line-height-base);
  }

  /* Headings sit a little below the body's colour, and the reason is the
     weight rather than the hue: at 600 the strokes are thicker, so on a dark
     ground they throw more light than the prose around them and read as
     glaring. Body and headings were the same colour — the difference someone
     sees is entirely the weight — so softening the colour is what puts them
     back in the same voice.

     The value is the body's own secondary tone, and it lands there by the same
     reasoning: at 600 the strokes carry the emphasis, so the colour does not
     have to. A heading the same colour as the prose reads *louder* than the
     prose; one a step below it reads level with it, which is what a heading is
     for — it is found by weight and size, not by shouting.

     A token rather than a hex, because there are three themes and in the light
     one the text is dark on pale: "dimmer" there means lighter, the opposite
     value and the same intent. */
  :global(.writing-editor__surface h1),
  :global(.writing-editor__surface h2),
  :global(.writing-editor__surface h3),
  :global(.writing-editor__surface h4) {
    font-family: var(--font-reading);
    line-height: var(--line-height-tight);
    margin: var(--space-5) 0 var(--space-2);
    color: var(--color-text-secondary);
    /* A step below the 600 the global rule gives every other heading in the
       app. This is the one surface someone reads for an hour at a time, and at
       the sizes a manuscript uses the strokes thicken enough to fight the prose
       under them. Scoped here rather than changed globally: a heading in a
       panel is glanced at, and 600 is right for glancing. */
    font-weight: var(--font-weight-medium);
  }

  /* No focus ring. `:focus-visible` always matches an element that accepts text
     input, so a ring here is not an occasional keyboard affordance — it is
     drawn the whole time someone is writing, boxing in the text column two
     pixels from the words. The caret is the indicator a writing surface has. */
  :global(.writing-editor__surface:focus-visible) {
    outline: none;
  }

  :global(.writing-editor__surface p) {
    margin: 0 0 var(--space-3);
  }

  /* Paragraph indent (paragraph-format.ts): the block carries its level in
     --writing-indent, and each level is two of the body's em. The step is the
     body size, not the block's own em, so a heading lines up with the text
     at the same level instead of stepping further by its larger size. */
  :global(.writing-editor__surface [data-indent]) {
    margin-inline-start: calc(var(--writing-indent, 0) * 2 * var(--font-size-md));
  }

  /* Line spacing (paragraph-format.ts): a multiple of the reading face's single
     line, as in Word, where "single" is the font's own line and not one em.
     Every reading face is taller than one em, so a bare line-height of 1 ran
     the lines of a paragraph into each other. */
  :global(.writing-editor__surface [data-line-height]) {
    line-height: calc(var(--writing-line-height, 1) * var(--font-reading-single-line, 1.35));
  }

  /* A marker is rendered outside its item's content box by default. The
     surface is a fixed-width column, so `1.` and `•` land to the left of the
     text and read as though they had escaped the manuscript. The padding is
     what holds them inside it. */
  :global(.writing-editor__surface ul),
  :global(.writing-editor__surface ol) {
    margin: 0 0 var(--space-3);
    padding-inline-start: var(--space-5);
  }

  :global(.writing-editor__surface li > p) {
    margin: 0;
  }

  :global(.writing-editor__surface blockquote) {
    margin: var(--space-3) 0;
    padding-left: var(--space-4);
    border-left: 2px solid var(--border-subtle);
    color: var(--color-text-secondary);
    font-style: italic;
  }

  :global(.writing-editor__surface table) {
    border-collapse: collapse;
    /* Not a flat 100%: at full width the outer border sits flush against the
       text column's edge and reads as part of the frame. */
    width: calc(100% - 2px);
    margin: var(--space-3) 1px;
    table-layout: fixed;
  }

  :global(.writing-editor__surface th),
  :global(.writing-editor__surface td) {
    border: 1px solid var(--border-subtle);
    padding: var(--space-1) var(--space-2);
    text-align: left;
  }

  :global(.writing-editor__surface th) {
    background: var(--color-surface-raised);
    font-weight: var(--font-weight-medium);
  }

  :global(.writing-editor__surface p.is-editor-empty:first-child::before) {
    content: attr(data-placeholder);
    float: left;
    height: 0;
    pointer-events: none;
    color: var(--color-text-muted);
  }

  /* Footnotes render as an ordered list the extension appends at the end of
     the document, with superscript references in the body. */
  :global(.writing-editor__surface .footnotes) {
    margin-top: var(--space-6);
    padding-top: var(--space-3);
    padding-inline-start: var(--space-5);
    border-top: 1px solid var(--border-subtle);
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
    /* The note is numbered below instead, so the marker the browser would
       draw has to go: `::marker` takes a font size but not `vertical-align`,
       which makes it the one thing that cannot be raised to match the
       superscript reference in the body. */
    list-style: none;
    counter-reset: footnote;
  }

  :global(.writing-editor__surface .footnotes li) {
    position: relative;
    counter-increment: footnote;
    margin-bottom: var(--space-1);
  }

  /* Out of the flow on purpose. A `footnote` holds `paragraph+`, so an inline
     counter would open an anonymous block above the note's first paragraph
     rather than labelling it. Absolute placement also lets the number hang in
     the list's own padding, level with the body text, instead of pushing the
     note across or escaping the column. */
  :global(.writing-editor__surface .footnotes li::before) {
    content: counter(footnote);
    position: absolute;
    top: 0;
    left: calc(var(--space-5) * -1);
    width: var(--space-5);
    box-sizing: border-box;
    padding-right: var(--space-1);
    text-align: right;
    font-size: var(--font-size-2xs);
    line-height: var(--line-height-base);
    color: var(--color-text-muted);
  }

  :global(.writing-editor__surface sup[data-reference-id]),
  :global(.writing-editor__surface .footnote-reference) {
    color: var(--color-text-primary);
    cursor: pointer;
  }

  :global(.writing-editor__surface .ProseMirror-gapcursor:after) {
    border-top: 1px solid var(--color-text-primary);
  }

  /* Search hits are decorations, not marks: they never reach the canonical
     JSON, so they are styled on the class the plugin attaches. The current one
     has to be told apart at a glance while the caret is in the search field. */
  :global(.writing-editor__surface .writing-search__hit) {
    border-radius: var(--radius-xs);
    background: var(--color-accent-soft);
  }

  :global(.writing-editor__surface .writing-search__hit--current) {
    background: var(--color-warning-soft);
    box-shadow: inset 0 -2px 0 var(--color-warning);
  }

  /* Highlights (highlight.ts). The mark draws only its background; the ink is
     set here. A browser's own <mark> is black on yellow, which is unreadable
     on a dark page, so a mark with no colour this build knows draws nothing.
     Highlighted text takes the body colour even in a heading or a quote, where
     the text is a step softer: the palette is measured against the body colour
     (contrast-floor.test.ts), not the softer one. A text colour on the same
     words still wins — it is the mark's ancestor, since textStyle ranks
     first — and is measured on every highlight too. */
  :global(.writing-editor__surface mark) {
    background: none;
    color: inherit;
  }

  /* The colour arrives as --writing-highlight and is painted as a band one
     line tall, centred on the glyph box. A plain background fills the whole
     glyph box, which is taller than the line at tight spacing: at line-height
     1 each highlighted line covered the descenders of the line above. At the
     default spacing the glyph box is the shorter of the two and clips the
     band, so it reads as a plain background. */
  :global(.writing-editor__surface mark[data-highlight]) {
    background-image: linear-gradient(
      var(--writing-highlight, transparent),
      var(--writing-highlight, transparent)
    );
    background-size: 100% 1lh;
    background-position: center;
    background-repeat: no-repeat;
    /* Still, glyphs that reach past their line (p, g, j at line-height 1)
       meet the next line's highlight, which the browser paints after them.
       Blending keeps the ink on top: each theme picks the mode under which
       its ink is the pixel that wins (tokens.css). */
    mix-blend-mode: var(--writing-blend-highlight, normal);
    border-radius: 2px;
    color: var(--color-text-primary);
    box-decoration-break: clone;
    -webkit-box-decoration-break: clone;
  }

  /* Links too: a coloured link keeps its underline and takes the colour. */
  :global(.writing-editor__surface [data-text-color] mark[data-highlight]),
  :global(.writing-editor__surface [data-text-color] a) {
    color: inherit;
  }

  :global(.writing-editor__surface a) {
    color: var(--color-text-primary);
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  /* A live link to a note, distinguishable from a corpus citation at a glance:
     they are different kinds of provenance and lead to different places. */
  :global(.writing-editor__surface [data-note-link]) {
    padding: 0 var(--space-1);
    border-radius: var(--radius-xs);
    background: var(--color-accent-soft);
    color: var(--color-text-secondary);
    cursor: pointer;
  }

  :global(.writing-editor__surface [data-note-link]:hover),
  :global(.writing-editor__surface [data-document-citation]:hover) {
    color: var(--color-text-primary);
  }

  /* A quote keeps the page's line breaks and blank lines between paragraphs
     (rendered-selection.ts), and shows them; runs of spaces still fold.
     Tiptap's injected base style sets `.ProseMirror [contenteditable="false"]
     { white-space: normal }` on every atom, a citation included, and is
     injected after this sheet: at equal specificity it wins. Both classes are
     on the same element (the editor root), so naming both lifts this rule
     above it. */
  :global(.writing-editor__surface.ProseMirror [data-document-citation]) {
    white-space: pre-line;
  }

  /* A long quote (extensions.ts, isLongQuote) is set off as a boxed block,
     indented a tenth of the column on each side, and set a point below the
     body — as academic typesetting sets a long quotation off. A short one
     stays inline, at the size of the sentence it was written into. */
  :global(.writing-editor__surface.ProseMirror [data-document-citation][data-block-quote]) {
    display: block;
    margin: var(--space-3) 10%;
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    font-size: calc(1em - 1pt);
  }

  /* An image a quote took in. It is drawn where it stood on the page, on its
     own line and no wider than the column — a scan is far wider than a
     paragraph, and without a bound it would push the manuscript sideways. */
  :global(.writing-editor__surface [data-document-citation] img) {
    display: block;
    max-width: 100%;
    height: auto;
    margin: var(--space-2) 0;
    border-radius: var(--radius-xs);
  }

  :global(.writing-editor__surface [data-document-citation]) {
    padding: 0 var(--space-1);
    border-radius: var(--radius-xs);
    background: var(--color-surface-raised);
    color: var(--color-text-secondary);
    cursor: pointer;
  }

  /* The image's own shrink-wrapped frame (defect 3). `figure` stays a block
     spanning the whole column — the toolbar and the caption both need that
     width to lay out in — but the frame around the image itself sizes to
     its content (`width: fit-content`, a specific, non-'auto' value, which
     is what makes the `margin: auto` alignment rules below able to centre
     or side-align it at all: auto margins only distribute free space
     against a definite width). `max-width: 100%` keeps a stored width
     wider than the column — from a narrower window, a different variant,
     or a hand-edited document — from overflowing it. `position: relative`
     is what gives the resize handle the image's own corner as its
     positioning context (below), instead of the far edge of the
     full-width figure — the bug the user reported. */
  :global(.writing-editor__surface .writing-editor__image-frame) {
    position: relative;
    display: block;
    width: fit-content;
    max-width: 100%;
  }

  :global(
    .writing-editor__surface [data-writing-image][data-align='left'] .writing-editor__image-frame
  ) {
    margin: 0 auto 0 0;
  }

  :global(
    .writing-editor__surface [data-writing-image][data-align='center'] .writing-editor__image-frame
  ) {
    margin: 0 auto;
  }

  :global(
    .writing-editor__surface [data-writing-image][data-align='right'] .writing-editor__image-frame
  ) {
    margin: 0 0 0 auto;
  }

  :global(.writing-editor__surface [data-writing-image] img) {
    display: block;
    max-width: 100%;
    height: auto;
  }

  /* A stored file missing at render time (I6, spec Failure Handling): the
     node stays in the document, and this stands in for the browser's own
     broken-image glyph. Hidden unless the img actually failed to load. It
     lives in the same shrink-wrapped frame as the image (above) — without
     an image to size the frame to, it gets its own minimum footprint here
     instead of collapsing to the width of its short message. */
  :global(.writing-editor__surface [data-writing-image] .writing-editor__image-placeholder) {
    display: none;
  }

  :global(.writing-editor__surface [data-writing-image][data-broken] img) {
    display: none;
  }

  :global(
    .writing-editor__surface [data-writing-image][data-broken] .writing-editor__image-placeholder
  ) {
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: 12rem;
    min-height: 4rem;
    padding: var(--space-3);
    border: 1px dashed var(--border-subtle);
    border-radius: var(--radius-sm);
    background: var(--surface-toolbar);
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    text-align: center;
  }

  /* The resize handle is gated on selection independently of the toolbar
     below (I1 fix round): it now lives in `.writing-editor__image-frame`,
     not in the toolbar's own chrome container, precisely so it can be
     positioned on the image's corner instead of the toolbar's box. */
  /* The resize handle: a grabbable button pinned to the image's own corner,
     carrying the 'resize-diagonal' glyph (extensions.ts) so the drag it
     starts reads at a glance — I1 originally flagged it as an empty,
     zero-content square. The box is sized from padding around the icon
     rather than a fixed width/height, so the hit area is comfortably larger
     than the glyph itself without a hardcoded pixel box.

     Appearance and visibility live in ONE rule deliberately. They were split
     across two blocks for the same selector, and the later one's `display`
     overrode this one's `display: none` at equal specificity — leaving the
     handle visible on every image in the manuscript, selected or not. Like
     the chrome above, it belongs to the selection, so it stays hidden until
     ProseMirror marks the figure as the selected node. */
  :global(.writing-editor__surface .writing-editor__image-handle) {
    position: absolute;
    right: var(--space-2);
    bottom: var(--space-2);
    display: none;
    align-items: center;
    justify-content: center;
    /* Square, not merely padded. The icon sits in an inline host span, which
       carries the surface's line-height and stretched the box taller than it
       is wide; `aspect-ratio` pins the two equal without hardcoding a pixel
       box, and `line-height: 0` stops the host contributing any slack of its
       own. */
    aspect-ratio: 1;
    padding: var(--space-1);
    line-height: 0;
    border: 1px solid var(--color-accent);
    border-radius: var(--radius-xs);
    background: transparent;
    color: var(--color-accent);
    cursor: nwse-resize;
  }

  :global(.writing-editor__surface .writing-editor__image-handle > *) {
    display: flex;
  }

  :global(
    .writing-editor__surface
      [data-writing-image].ProseMirror-selectednode
      .writing-editor__image-handle
  ) {
    display: flex;
  }

  :global(.writing-editor__surface .writing-editor__image-handle:focus-visible) {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  /* The selection bubble (I1): alignment and alt, shown only while the
     figure is the selected node and never as a permanent panel (spec,
     Node View and Resizing). ProseMirror itself adds ProseMirror-selectednode
     to the figure on node selection — see writing-image.test.ts, "reveals
     the chrome only once ProseMirror marks the figure as the selected
     node" for the exact contract this relies on. */
  :global(.writing-editor__surface .writing-editor__image-chrome) {
    display: none;
  }

  :global(
    .writing-editor__surface
      [data-writing-image].ProseMirror-selectednode
      .writing-editor__image-chrome
  ) {
    display: block;
  }

  /* The caption (defect 4): the figcaption is the node's own document
     content — real, visible, editable prose — not the `title` attribute,
     which is an HTML tooltip and nothing more. Collapsed while empty and
     unselected, so an uncaptioned image costs no vertical space; while
     empty and selected, it opens up and shows a muted placeholder line so
     a writer can find it (spec, Node Shape). Matches the same
     data-placeholder/::before convention this editor's paragraph
     placeholder already uses, above. */
  :global(.writing-editor__surface [data-writing-image] figcaption) {
    display: block;
    margin-top: var(--space-1);
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    font-style: italic;
    text-align: center;
  }

  :global(.writing-editor__surface [data-writing-image][data-align='left'] figcaption) {
    text-align: left;
  }

  :global(.writing-editor__surface [data-writing-image][data-align='right'] figcaption) {
    text-align: right;
  }

  :global(.writing-editor__surface [data-writing-image] figcaption[data-empty]) {
    display: none;
  }

  :global(
    .writing-editor__surface [data-writing-image].ProseMirror-selectednode figcaption[data-empty]
  ) {
    display: block;
    cursor: text;
  }

  :global(
    .writing-editor__surface
      [data-writing-image].ProseMirror-selectednode
      figcaption[data-empty]::before
  ) {
    content: attr(data-placeholder);
  }

  /* Defect 5: the placeholder must also stay visible while the caret is
     genuinely inside the (empty) caption — not only while the figure carries
     ProseMirror-selectednode, which a caret landing in the caption always
     clears. `data-caret-inside` is set by the node view itself
     (extensions.ts) from `editor.on('selectionUpdate', ...)`, tracking a
     condition CSS alone cannot express: whether the current selection falls
     inside this node's own document range. */
  :global(.writing-editor__surface [data-writing-image][data-caret-inside] figcaption[data-empty]) {
    display: block;
    cursor: text;
  }

  :global(
    .writing-editor__surface [data-writing-image][data-caret-inside] figcaption[data-empty]::before
  ) {
    content: attr(data-placeholder);
  }

  :global(.writing-editor__surface .writing-editor__image-toolbar) {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin-top: var(--space-1);
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    background: var(--surface-toolbar);
  }

  :global(.writing-editor__surface .writing-editor__image-align) {
    display: flex;
    /* Icon-sized now (I2, this round) — pinned to its own content width so it
       never grows or gets squeezed, and the freed row space goes to
       .writing-editor__image-fields below, which already claims the rest via
       flex: 1. */
    flex: 0 0 auto;
    gap: var(--space-1);
  }

  :global(.writing-editor__surface .writing-editor__image-align button) {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-xs);
    background: var(--surface-input);
    color: var(--color-text-secondary);
    cursor: pointer;
  }

  :global(.writing-editor__surface .writing-editor__image-align button[aria-pressed='true']) {
    border-color: var(--border-focus);
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
  }

  :global(.writing-editor__surface .writing-editor__image-fields) {
    display: flex;
    flex: 1;
    min-width: 12rem;
    gap: var(--space-1);
  }

  :global(.writing-editor__surface .writing-editor__image-field) {
    flex: 1;
    min-width: 6rem;
    min-height: 24px;
    padding: 0 var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-input);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font: inherit;
    font-size: var(--font-size-2xs);
  }

  :global(.writing-editor__surface .writing-editor__image-field:focus-visible) {
    outline: none;
    border-color: var(--border-focus);
    box-shadow: var(--focus-ring);
  }
</style>
