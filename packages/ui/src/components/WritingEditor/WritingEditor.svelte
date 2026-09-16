<script lang="ts">
  import { Editor } from '@tiptap/core'
  import { onDestroy, onMount } from 'svelte'
  import ActionIcon from '../Button/ActionIcon.svelte'
  import Button from '../Button/Button.svelte'
  import IconButton from '../IconButton/IconButton.svelte'
  import SearchBar from '../SearchBar/SearchBar.svelte'
  import { createWritingExtensions } from './extensions'
  import {
    goToMatch,
    readSearch,
    replaceAll,
    replaceCurrent,
    setSearch,
    type SearchState,
  } from './search-highlight'
  import { nextMatchIndex } from './search'
  import {
    deleteSection,
    insertSectionAfter,
    moveSection,
    renameSection,
  } from './section-commands'
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
      extensions: createWritingExtensions({ placeholder }),
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

  onDestroy(() => {
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
      <div class="writing-editor__toolbar" role="toolbar" aria-label={labels.toolbarLabel}>
        <IconButton size="sm" variant="ghost" label={labels.undo} disabled={!active.canUndo}
          onclick={() => chain()?.undo().run()}><ActionIcon name="undo" size={14} /></IconButton>
        <IconButton size="sm" variant="ghost" label={labels.redo} disabled={!active.canRedo}
          onclick={() => chain()?.redo().run()}><ActionIcon name="redo" size={14} /></IconButton>

        <span class="writing-editor__sep" aria-hidden="true"></span>

        <IconButton size="sm" variant="ghost" label={labels.bold} active={active.bold}
          onclick={() => chain()?.toggleBold().run()}><ActionIcon name="bold" size={14} /></IconButton>
        <IconButton size="sm" variant="ghost" label={labels.italic} active={active.italic}
          onclick={() => chain()?.toggleItalic().run()}><ActionIcon name="italic" size={14} /></IconButton>
        <IconButton size="sm" variant="ghost" label={labels.underline} active={active.underline}
          onclick={() => chain()?.toggleUnderline().run()}><ActionIcon name="underline" size={14} /></IconButton>
        <IconButton size="sm" variant="ghost" label={labels.strike} active={active.strike}
          onclick={() => chain()?.toggleStrike().run()}><ActionIcon name="strikethrough" size={14} /></IconButton>
        <IconButton size="sm" variant="ghost" label={labels.code} active={active.code}
          onclick={() => chain()?.toggleCode().run()}><ActionIcon name="code" size={14} /></IconButton>

        <span class="writing-editor__sep" aria-hidden="true"></span>

        <IconButton size="sm" variant="ghost" label={labels.heading1} active={active.h1}
          onclick={() => chain()?.toggleHeading({ level: 1 }).run()}><ActionIcon name="heading-1" size={14} /></IconButton>
        <IconButton size="sm" variant="ghost" label={labels.heading2} active={active.h2}
          onclick={() => chain()?.toggleHeading({ level: 2 }).run()}><ActionIcon name="heading-2" size={14} /></IconButton>
        <IconButton size="sm" variant="ghost" label={labels.heading3} active={active.h3}
          onclick={() => chain()?.toggleHeading({ level: 3 }).run()}><ActionIcon name="heading-3" size={14} /></IconButton>

        <span class="writing-editor__sep" aria-hidden="true"></span>

        <IconButton size="sm" variant="ghost" label={labels.bulletList} active={active.bulletList}
          onclick={() => chain()?.toggleBulletList().run()}><ActionIcon name="list" size={14} /></IconButton>
        <IconButton size="sm" variant="ghost" label={labels.orderedList} active={active.orderedList}
          onclick={() => chain()?.toggleOrderedList().run()}><ActionIcon name="list-ordered" size={14} /></IconButton>
        <IconButton size="sm" variant="ghost" label={labels.blockquote} active={active.blockquote}
          onclick={() => chain()?.toggleBlockquote().run()}><ActionIcon name="text-quote" size={14} /></IconButton>

        <span class="writing-editor__sep" aria-hidden="true"></span>

        <IconButton size="sm" variant="ghost" label={active.link ? labels.unlink : labels.link}
          active={active.link} onclick={openLinkField}
        ><ActionIcon name={active.link ? 'unlink' : 'link'} size={14} /></IconButton>
        <IconButton size="sm" variant="ghost" label={labels.table} disabled={active.inTable}
          onclick={() => chain()?.insertTable({ rows: 3, cols: 3, withHeaderRow: true }).run()}
        ><ActionIcon name="table" size={14} /></IconButton>
        <IconButton size="sm" variant="ghost" label={labels.footnote}
          onclick={insertFootnote}><ActionIcon name="footnote" size={14} /></IconButton>
        <IconButton size="sm" variant="ghost" label={labels.find} active={searchOpen}
          onclick={() => (searchOpen ? closeSearch() : openSearch())}
        ><ActionIcon name="search" size={14} /></IconButton>
      </div>

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
        <IconButton size="sm" variant="ghost" label={labels.findPrevious}
          disabled={matches.matches.length === 0} onclick={() => step(-1)}
        ><ActionIcon name="chevron-up" size={14} /></IconButton>
        <IconButton size="sm" variant="ghost" label={labels.findNext}
          disabled={matches.matches.length === 0} onclick={() => step(1)}
        ><ActionIcon name="chevron-down" size={14} /></IconButton>

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
        <Button variant="ghost" size="sm" disabled={matches.matches.length === 0}
          onclick={replaceEvery}
        >{labels.replaceAll}</Button>
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


  .writing-editor__toolbar {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex-wrap: wrap;
    padding: var(--space-1) var(--space-2);
    border-bottom: 1px solid var(--border-subtle);
    background: var(--surface-toolbar);
  }

  .writing-editor__sep {
    width: 1px;
    height: 16px;
    margin: 0 var(--space-1);
    background: var(--border-subtle);
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
    font-family: var(--font-sans);
    font-size: var(--font-size-md);
    line-height: var(--line-height-base);
  }

  :global(.writing-editor__surface h1),
  :global(.writing-editor__surface h2),
  :global(.writing-editor__surface h3),
  :global(.writing-editor__surface h4) {
    font-family: var(--font-display);
    line-height: var(--line-height-tight);
    margin: var(--space-5) 0 var(--space-2);
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

  :global(.writing-editor__surface [data-document-citation]) {
    padding: 0 var(--space-1);
    border-radius: var(--radius-xs);
    background: var(--color-surface-raised);
    color: var(--color-text-secondary);
    cursor: pointer;
  }
</style>
