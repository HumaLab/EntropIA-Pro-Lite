<script lang="ts">
  import { Editor } from '@tiptap/core'
  import { onDestroy, onMount } from 'svelte'
  import { WRITING_EXTENSIONS } from './extensions'
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
    placeholder = '',
    labels: labelOverrides,
  }: WritingEditorProps = $props()

  const labels = $derived({ ...DEFAULT_WRITING_EDITOR_LABELS, ...labelOverrides })

  let editorElement: HTMLDivElement | undefined = $state(undefined)
  let editor: Editor | undefined
  let refusal: ValidationFailure | null = $state(null)
  /** What we last emitted, so an echo back through `document` is a no-op. */
  let lastEmitted = ''

  function buildEditor(source: CanonicalDocument) {
    if (!editorElement) return
    editor = new Editor({
      element: editorElement,
      extensions: WRITING_EXTENSIONS,
      content: source.doc,
      editable,
      editorProps: { attributes: { class: 'writing-editor__surface' } },
      onUpdate: ({ editor: instance }) => {
        const next: CanonicalDocument = {
          schemaVersion: WRITING_SCHEMA_VERSION,
          doc: instance.getJSON(),
        }
        lastEmitted = JSON.stringify(next)
        onchange?.(next)
      },
    })
    onready?.()
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
</script>

{#if refusal}
  <div class="writing-editor writing-editor--refused" role="alert">
    <p class="writing-editor__refused-title">{labels.refusedTitle}</p>
    <p class="writing-editor__refused-body">{refusalMessage(refusal, labels)}</p>
  </div>
{:else}
  <div class="writing-editor">
    <div
      bind:this={editorElement}
      class="writing-editor__host"
      role="textbox"
      tabindex="0"
      aria-multiline="true"
      aria-label={labels.editorLabel}
      data-placeholder={placeholder}
    ></div>
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

  .writing-editor__host {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-6) var(--space-5);
  }

  .writing-editor__host:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
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

  :global(.writing-editor__surface p) {
    margin: 0 0 var(--space-3);
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
    width: 100%;
    margin: var(--space-3) 0;
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
