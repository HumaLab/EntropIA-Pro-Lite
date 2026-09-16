<script lang="ts">
  import { Editor } from '@tiptap/core'
  import { onDestroy, onMount } from 'svelte'
  import ActionIcon from '../Button/ActionIcon.svelte'
  import Button from '../Button/Button.svelte'
  import IconButton from '../IconButton/IconButton.svelte'
  import { createWritingExtensions } from './extensions'
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

  const chain = () => editor?.chain().focus()

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
  <div class="writing-editor">
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

  :global(.writing-editor__surface a) {
    color: var(--color-text-primary);
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  :global(.writing-editor__surface [data-document-citation]) {
    padding: 0 var(--space-1);
    border-radius: var(--radius-xs);
    background: var(--color-surface-raised);
    color: var(--color-text-secondary);
  }
</style>
