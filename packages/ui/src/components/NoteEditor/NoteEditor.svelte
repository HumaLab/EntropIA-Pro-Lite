<script lang="ts">
  import { tooltip } from '../Tooltip/tooltip'
  import { Dialog } from '../Dialog'
  import { Input } from '../Input'
  import { Button } from '../Button'
  import { IconButton } from '../IconButton'
  import { onDestroy, onMount, tick } from 'svelte'
  import { Editor } from '@tiptap/core'
  import ActionIcon from '../Button/ActionIcon.svelte'
  import type { ActionIconName } from '../Button/ActionIcon.types'

  import type { NoteEditorLabels, NoteEditorProps } from './NoteEditor.types'
  import { createDictation } from '../Dictation/dictation.svelte'
  import { createNoteExtensions } from './note-extensions'
  import {
    normalizeNoteContentForEditor,
    normalizeNoteContentForRender,
    sanitizeNoteHtml,
    shouldDisableNoteEditorSave,
  } from './note-content'

  let {
    content = '',
    placeholder = '',
    onsave,
    oncancel,
    ondictate,
    ondictationlog,
    ondictationhelp,
    dictationMaxSeconds = 300,
    clearOnSave = true,
    saveLabel = 'Save',
    cancelLabel = 'Cancel',
    labels: labelsProp = {},
  }: NoteEditorProps = $props()

  const defaultLabels: NoteEditorLabels = {
    toolbarAriaLabel: 'Formatting toolbar',
    textStyleGroup: 'Text style',
    structureGroup: 'Structure',
    insertGroup: 'Insert',
    dictationGroup: 'Dictation',
    bold: 'Bold',
    italic: 'Italic',
    underline: 'Underline',
    inlineCode: 'Inline code',
    heading1: 'Heading 1',
    heading2: 'Heading 2',
    heading3: 'Heading 3',
    bulletList: 'Bullet list',
    orderedList: 'Ordered list',
    quote: 'Quote',
    addLink: 'Add link',
    removeLink: 'Remove link',
    dictationStart: 'Start dictation',
    dictationStop: 'Stop dictation',
    dictationProcessing: 'Processing dictation...',
    helperText: 'Tip: select text to apply formatting or links.',
    dictationNoMicrophone: 'Microphone is not available on this device.',
    dictationNoAudio: 'Could not capture audio from the microphone.',
    dictationAutoStopProcessing: 'Reached the maximum of {duration}. Processing audio...',
    dictationTranscribing: 'Transcribing audio...',
    dictationAutoStopInserted: 'Reached the maximum of {duration}. Text inserted.',
    dictationInserted: 'Text inserted from the microphone.',
    dictationNoText: 'No text was detected in the audio.',
    dictationTranscriptionFailed: 'Could not transcribe the audio.',
    dictationOpenDependencies: 'Open AI dependencies',
    linkInvalidUrl: 'Enter a valid URL.',
    linkInvalidHttp: 'Use a valid http or https URL.',
    linkInvalidExample: 'Enter a valid URL, for example https://entropia.app.',
    linkModalTitle: 'Insert link',
    linkModalDescription: 'Paste a valid URL for the selected text.',
    linkUrlLabel: 'URL',
    linkPlaceholder: 'https://...',
    linkCancel: 'Cancel',
    linkSubmit: 'Insert',
  }

  const labels = $derived({ ...defaultLabels, ...labelsProp })

  let editorElement: HTMLDivElement | undefined = $state(undefined)
  let linkInputElement: HTMLInputElement | undefined = $state(undefined)
  let editor = $state<Editor | null>(null)
  let editorRevision = $state(0)
  let currentHtml = $state('<p></p>')
  let originalHtml = $state('<p></p>')
  let lastExternalHtml = $state('')
  let isFocused = $state(false)
  let isLinkModalOpen = $state(false)
  let linkDraftHref = $state('')
  let linkModalError = $state<string | null>(null)
  let linkSelection = $state<{ from: number; to: number } | null>(null)

  const showCancel = $derived(typeof oncancel === 'function')
  const supportsDictation = $derived(typeof ondictate === 'function')
  const isEditing = $derived(showCancel || !clearOnSave)
  const isSaveDisabled = $derived(
    shouldDisableNoteEditorSave({
      currentContent: currentHtml,
      originalContent: originalHtml,
      isEditing,
    })
  )

  const dictation = createDictation({
    get editor() {
      return editor
    },
    get editorElement() {
      return editorElement
    },
    get isEditorFocused() {
      return isFocused
    },
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
    logPrefix: '[NoteEditor/dictation]',
    insertion: 'append-unless-caret',
    oninserted: () => {
      if (editor) syncEditorState(sanitizeNoteHtml(editor.getHTML()) || '<p></p>')
    },
  })

  const linkModalTitleId = 'note-editor-link-modal-title'
  const linkFormId = 'note-editor-link-form'

  type ToolbarButton = {
    label: string
    icon: ActionIconName
    isActive: () => boolean
    action: () => void
  }

  type ToolbarGroup = {
    label: string
    buttons: ToolbarButton[]
  }

  const toolbarGroups = $derived.by<ToolbarGroup[]>(() => [
    {
      label: labels.textStyleGroup,
      buttons: [
        {
          label: labels.bold,
          icon: 'bold',
          isActive: () => editor?.isActive('bold') ?? false,
          action: () => editor?.chain().focus().toggleBold().run(),
        },
        {
          label: labels.italic,
          icon: 'italic',
          isActive: () => editor?.isActive('italic') ?? false,
          action: () => editor?.chain().focus().toggleItalic().run(),
        },
        {
          label: labels.underline,
          icon: 'underline',
          isActive: () => editor?.isActive('underline') ?? false,
          action: () => editor?.chain().focus().toggleUnderline().run(),
        },
        {
          label: labels.inlineCode,
          icon: 'code',
          isActive: () => editor?.isActive('code') ?? false,
          action: () => editor?.chain().focus().toggleCode().run(),
        },
      ],
    },
    {
      label: labels.structureGroup,
      buttons: [
        {
          label: labels.heading1,
          icon: 'heading-1',
          isActive: () => editor?.isActive('heading', { level: 1 }) ?? false,
          action: () => editor?.chain().focus().toggleHeading({ level: 1 }).run(),
        },
        {
          label: labels.heading2,
          icon: 'heading-2',
          isActive: () => editor?.isActive('heading', { level: 2 }) ?? false,
          action: () => editor?.chain().focus().toggleHeading({ level: 2 }).run(),
        },
        {
          label: labels.heading3,
          icon: 'heading-3',
          isActive: () => editor?.isActive('heading', { level: 3 }) ?? false,
          action: () => editor?.chain().focus().toggleHeading({ level: 3 }).run(),
        },
        {
          label: labels.bulletList,
          icon: 'list',
          isActive: () => editor?.isActive('bulletList') ?? false,
          action: () => editor?.chain().focus().toggleBulletList().run(),
        },
        {
          label: labels.orderedList,
          icon: 'list-ordered',
          isActive: () => editor?.isActive('orderedList') ?? false,
          action: () => editor?.chain().focus().toggleOrderedList().run(),
        },
        {
          label: labels.quote,
          icon: 'text-quote',
          isActive: () => editor?.isActive('blockquote') ?? false,
          action: () => editor?.chain().focus().toggleBlockquote().run(),
        },
      ],
    },
    {
      label: labels.insertGroup,
      buttons: [
        {
          label: labels.addLink,
          icon: 'link',
          isActive: () => editor?.isActive('link') ?? false,
          action: () => updateLink(),
        },
        {
          label: labels.removeLink,
          icon: 'unlink',
          isActive: () => false,
          action: () => removeLink(),
        },
      ],
    },
  ])

  function bumpEditorRevision() {
    editorRevision += 1
  }

  function syncEditorState(nextHtml: string) {
    currentHtml = nextHtml || '<p></p>'
    bumpEditorRevision()
  }

  function buildEditor() {
    if (!editorElement) return

    const instance = new Editor({
      element: editorElement,
      extensions: createNoteExtensions(placeholder),
      content: currentHtml,
      autofocus: false,
      editorProps: {
        attributes: {
          class: 'note-editor__content ProseMirror',
          role: 'textbox',
          'aria-multiline': 'true',
          'aria-placeholder': placeholder,
          'data-testid': 'note-editor-input',
        },
      },
      onCreate: ({ editor }: { editor: Editor }) => {
        syncEditorState(sanitizeNoteHtml(editor.getHTML()) || '<p></p>')
      },
      onUpdate: ({ editor }: { editor: Editor }) => {
        syncEditorState(sanitizeNoteHtml(editor.getHTML()) || '<p></p>')
      },
      onSelectionUpdate: () => {
        bumpEditorRevision()
      },
      onFocus: () => {
        isFocused = true
        bumpEditorRevision()
      },
      onBlur: () => {
        // Unmounting a focused editor (a save that removes the edit view
        // without a prior click) blurs it while Svelte is still rendering;
        // writing $state there throws state_unsafe_mutation. Defer it.
        queueMicrotask(() => {
          isFocused = false
          bumpEditorRevision()
        })
      },
    })

    editor = instance
  }

  function normalizeLinkHref(value: string) {
    const trimmed = value.trim()

    if (!trimmed) {
      return {
        isValid: false,
        normalized: '',
        error: labels.linkInvalidUrl,
      }
    }

    const candidate = /^[a-zA-Z][a-zA-Z\d+.-]*:/.test(trimmed) ? trimmed : `https://${trimmed}`

    try {
      const url = new URL(candidate)

      if (!['http:', 'https:'].includes(url.protocol)) {
        return {
          isValid: false,
          normalized: '',
          error: labels.linkInvalidHttp,
        }
      }

      return {
        isValid: true,
        normalized: url.toString(),
        error: null,
      }
    } catch {
      return {
        isValid: false,
        normalized: '',
        error: labels.linkInvalidExample,
      }
    }
  }

  async function updateLink() {
    if (!editor) return

    const { from, to } = editor.state.selection

    linkSelection = { from, to }
    linkDraftHref = editor.getAttributes('link').href ?? ''
    linkModalError = null
    isLinkModalOpen = true

    await tick()

    linkInputElement?.focus()
    linkInputElement?.select()
  }

  function closeLinkModal() {
    isLinkModalOpen = false
    linkModalError = null
    linkDraftHref = ''
    linkSelection = null

    editor?.commands.focus()
  }

  function handleLinkInput() {
    if (linkModalError) {
      linkModalError = null
    }
  }

  function submitLink() {
    if (!editor) return

    const result = normalizeLinkHref(linkDraftHref)

    if (!result.isValid) {
      linkModalError = result.error
      linkInputElement?.focus()
      return
    }

    let chain = editor.chain().focus()

    if (linkSelection) {
      chain = chain.setTextSelection(linkSelection)
    }

    chain.extendMarkRange('link').setLink({ href: result.normalized }).run()

    closeLinkModal()
    bumpEditorRevision()
  }

  function removeLink() {
    if (!editor) return

    editor.chain().focus().extendMarkRange('link').unsetLink().run()
    bumpEditorRevision()
  }

  function clearEditor() {
    editor?.commands.setContent('<p></p>', { emitUpdate: false })
    currentHtml = '<p></p>'
    originalHtml = '<p></p>'
    lastExternalHtml = normalizeNoteContentForEditor(content)
    bumpEditorRevision()
  }

  async function handleSave() {
    if (!editor || isSaveDisabled) return

    const html = normalizeNoteContentForRender(editor.getHTML())
    if (!html) return

    try {
      await onsave?.(html)

      if (clearOnSave) {
        clearEditor()
        editor.commands.focus('end')
        return
      }

      originalHtml = html
      lastExternalHtml = html
      currentHtml = html || '<p></p>'
      editor.commands.setContent(html, { emitUpdate: false })
      editor.commands.focus('end')
      bumpEditorRevision()
    } catch {
      // Save failed — keep content so the user can retry
    }
  }

  onMount(() => {
    const normalizedInitial = normalizeNoteContentForEditor(content)
    currentHtml = normalizedInitial
    originalHtml = normalizedInitial
    lastExternalHtml = normalizedInitial
    buildEditor()
  })

  onDestroy(() => {
    dictation.destroy()
    editor?.destroy()
    editor = null
  })

  $effect(() => {
    const normalizedExternal = normalizeNoteContentForEditor(content)

    if (normalizedExternal === lastExternalHtml) {
      return
    }

    lastExternalHtml = normalizedExternal
    originalHtml = normalizedExternal
    currentHtml = normalizedExternal

    if (!editor) {
      return
    }

    const currentEditorHtml = normalizeNoteContentForRender(editor.getHTML())
    const nextEditorHtml = normalizeNoteContentForRender(normalizedExternal)

    if (currentEditorHtml !== nextEditorHtml) {
      editor.commands.setContent(normalizedExternal, { emitUpdate: false })
      bumpEditorRevision()
    }
  })
</script>

<div class="note-editor">
  <div
    class="note-editor__toolbar"
    aria-label={labels.toolbarAriaLabel}
    data-editor-revision={editorRevision}
  >
    {#each toolbarGroups as group (group.label)}
      <div class="note-editor__tool-group" role="group" aria-label={group.label}>
        {#each group.buttons as button (button.label)}
          <button
            type="button"
            class="note-editor__tool"
            class:note-editor__tool--active={button.isActive()}
            aria-pressed={button.isActive()}
            aria-label={button.label}
            use:tooltip={button.label}
            onmousedown={(event) => event.preventDefault()}
            onclick={button.action}
          >
            <ActionIcon name={button.icon} size={16} />
          </button>
        {/each}
      </div>
    {/each}

    {#if supportsDictation}
      <div
        class="note-editor__tool-group note-editor__tool-group--dictation"
        role="group"
        aria-label={labels.dictationGroup}
      >
        <button
          type="button"
          class="note-editor__tool"
          class:note-editor__tool--recording={dictation.state === 'recording'}
          aria-label={dictation.buttonLabel}
          use:tooltip={dictation.buttonLabel}
          disabled={dictation.state === 'transcribing'}
          onmousedown={(event) => event.preventDefault()}
          onclick={dictation.toggle}
        >
          <ActionIcon name="mic" size={16} />
        </button>
        {#if dictation.state === 'recording' || dictation.state === 'transcribing'}
          <span
            class="note-editor__dictation-status"
            class:note-editor__dictation-status--recording={dictation.state === 'recording'}
            data-testid="note-editor-dictation-timer"
          >
            {#if dictation.state === 'recording'}
              {dictation.timerLabel}
            {:else}
              {labels.dictationProcessing}
            {/if}
          </span>
        {/if}
      </div>
    {/if}
  </div>

  <p class="note-editor__helper">{labels.helperText}</p>

  {#if dictation.message}
    <p
      class="note-editor__dictation-message"
      class:note-editor__dictation-message--error={dictation.state === 'error'}
      data-testid="note-editor-dictation-message"
    >
      {dictation.message}
      {#if dictation.state === 'error' && ondictationhelp}
        <Button
          size="sm"
          variant="ghost"
          data-testid="note-editor-dictation-help"
          onclick={() => ondictationhelp?.()}
        >
          {labels.dictationOpenDependencies}
        </Button>
      {/if}
    </p>
  {/if}

  <div class="note-editor__surface" class:note-editor__surface--focused={isFocused}>
    <div bind:this={editorElement}></div>
  </div>

  <div class="note-editor__actions">
    {#if showCancel}
      <Button variant="ghost" size="sm" data-testid="note-cancel" onclick={() => oncancel?.()}>
        {cancelLabel}
      </Button>
    {/if}

    <IconButton
      variant="primary"
      size="md"
      data-testid="note-save"
      disabled={isSaveDisabled}
      aria-disabled={isSaveDisabled}
      label={saveLabel}
      title={saveLabel}
      onclick={handleSave}
    >
      <ActionIcon name="save" size={16} />
    </IconButton>
  </div>

  {#if isLinkModalOpen}
    <!-- Dialog portals itself out of the work pane; see Portal/portal.ts. -->
    <Dialog
      title={labels.linkModalTitle}
      titleId={linkModalTitleId}
      description={labels.linkModalDescription}
      overlayClass="note-editor__modal-backdrop"
      onclose={closeLinkModal}
    >
      <form
        id={linkFormId}
        novalidate
        onsubmit={(event) => {
          event.preventDefault()
          submitLink()
        }}
      >
        <Input
          id="note-editor-link-input"
          bind:element={linkInputElement}
          bind:value={linkDraftHref}
          label={labels.linkUrlLabel}
          type="text"
          inputmode="url"
          autocapitalize="off"
          autocomplete="off"
          autocorrect="off"
          spellcheck="false"
          placeholder={labels.linkPlaceholder}
          error={linkModalError ?? undefined}
          data-testid="note-editor-link-input"
          oninput={handleLinkInput}
        />
      </form>

      {#snippet footer()}
        <Button variant="secondary" data-testid="note-editor-link-cancel" onclick={closeLinkModal}>
          {labels.linkCancel}
        </Button>
        <Button
          type="submit"
          variant="primary"
          form={linkFormId}
          data-testid="note-editor-link-submit"
        >
          {labels.linkSubmit}
        </Button>
      {/snippet}
    </Dialog>
  {/if}
</div>

<style>
  .note-editor {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .note-editor__toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: 2px;
    align-items: center;
    padding: 2px;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    background: var(--surface-toolbar);
  }

  .note-editor__tool-group {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px;
  }

  .note-editor__tool-group + .note-editor__tool-group::before {
    content: '';
    align-self: center;
    width: 1px;
    height: 16px;
    margin: 0 var(--space-1);
    background: var(--color-hairline);
  }

  .note-editor__tool-group--dictation {
    margin-left: auto;
  }

  .note-editor__tool {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: none;
    border-radius: var(--radius-xs);
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
    transition:
      background-color var(--transition-base),
      color var(--transition-base),
      box-shadow var(--transition-base);
  }

  .note-editor__tool :global(svg) {
    flex-shrink: 0;
    pointer-events: none;
  }

  .note-editor__tool:hover:not(:disabled) {
    background: var(--color-accent-faint);
    color: var(--color-text-primary);
  }

  .note-editor__tool:disabled {
    cursor: not-allowed;
    opacity: var(--opacity-disabled);
  }

  .note-editor__tool:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .note-editor__tool--active {
    background: var(--color-accent-soft);
    color: var(--color-text-primary);
  }

  .note-editor__tool--recording {
    background: var(--color-danger-soft);
    color: var(--color-danger);
  }

  .note-editor__tool--recording:hover:not(:disabled) {
    background: var(--color-danger-soft);
    color: var(--color-danger-hover);
  }

  .note-editor__helper {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .note-editor__dictation-status {
    display: inline-flex;
    align-items: center;
    padding: 0 var(--space-2);
    font-size: var(--font-size-2xs);
    font-variant-numeric: tabular-nums;
    color: var(--color-text-muted);
  }

  .note-editor__dictation-status--recording {
    color: var(--color-danger);
  }

  .note-editor__dictation-message {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  .note-editor__dictation-message--error {
    color: var(--color-danger);
  }

  .note-editor__surface {
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    background: var(--color-surface-raised);
    overflow: hidden;
    transition:
      border-color var(--transition-base),
      box-shadow var(--transition-base);
  }

  .note-editor__surface--focused {
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
  }

  .note-editor__surface :global(.ProseMirror) {
    min-height: 88px;
    padding: var(--space-3);
    color: var(--color-text-primary);
    outline: none;
    font-family: var(--font-reading);
    font-size: var(--font-size-md);
    line-height: var(--line-height-reading);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .note-editor__surface :global(.ProseMirror p.is-editor-empty:first-child::before) {
    content: attr(data-placeholder);
    color: var(--color-text-muted);
    float: left;
    height: 0;
    pointer-events: none;
  }

  .note-editor__surface :global(.ProseMirror p:first-child) {
    margin-top: 0;
  }

  .note-editor__surface :global(.ProseMirror p:last-child) {
    margin-bottom: 0;
  }

  .note-editor__surface :global(.ProseMirror a) {
    color: var(--color-accent-hover);
    text-decoration: underline;
  }

  .note-editor__surface :global(.ProseMirror blockquote) {
    margin: var(--space-3) 0;
    padding-left: var(--space-3);
    border-left: 3px solid color-mix(in srgb, var(--color-accent) 45%, var(--color-border));
    color: var(--color-text-secondary);
  }

  .note-editor__surface :global(.ProseMirror code) {
    background: color-mix(in srgb, var(--color-border) 65%, transparent);
    border-radius: var(--radius-sm);
    padding: 0.1rem 0.3rem;
    font-size: var(--font-size-code-inline);
  }

  .note-editor__surface :global(.ProseMirror pre) {
    background: var(--color-surface-sunken);
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    overflow-x: auto;
  }

  .note-editor__surface :global(.ProseMirror ul),
  .note-editor__surface :global(.ProseMirror ol) {
    padding-left: 1.25rem;
  }

  .note-editor__surface :global(.ProseMirror:focus) {
    box-shadow: none;
  }

  .note-editor__surface :global(.ProseMirror ::selection) {
    background: color-mix(in srgb, var(--color-accent) 35%, transparent);
  }

  .note-editor__actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
  }
</style>
