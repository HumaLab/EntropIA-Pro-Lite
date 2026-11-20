<script lang="ts">
  import type { NoteEditorProps } from '@entropia/ui'

  let {
    content = '',
    onsave,
    oncancel,
    ondictate,
    labels,
    dictationMaxSeconds = 300,
    clearOnSave = true,
    saveLabel = 'Save note',
    cancelLabel = 'Cancel',
  }: NoteEditorProps = $props()

  const isEditing = $derived(typeof oncancel === 'function' || !clearOnSave)
  const isSaveDisabled = $derived(isEditing && !content.trim())
</script>

<div data-testid="mock-note-editor">
  <button type="button" aria-label={labels?.bold ?? 'Bold'}>{labels?.bold ?? 'Bold'}</button>
  <div role="group" aria-label={labels?.textStyleGroup ?? 'Text style'}></div>
  <div role="group" aria-label={labels?.structureGroup ?? 'Structure'}></div>
  <div role="group" aria-label={labels?.insertGroup ?? 'Insert'}></div>
  {#if ondictate}
    <div role="group" aria-label={labels?.dictationGroup ?? 'Dictation'}>
      <button type="button" aria-label={labels?.dictationStart ?? 'Iniciar dictado'}>🎙</button>
      <span data-testid="note-editor-dictation-timer">
        0:00 / {Math.floor(dictationMaxSeconds / 60)}:{(dictationMaxSeconds % 60)
          .toString()
          .padStart(2, '0')}
      </span>
    </div>
  {/if}
  <button type="button" aria-label={labels?.addLink ?? 'Add link'}>Link</button>
  <button type="button" aria-label={labels?.removeLink ?? 'Remove link'}>Unlink</button>
  <div role="textbox">{content}</div>
  <p>{labels?.helperText ?? 'Tip: seleccioná texto para aplicar formato o links.'}</p>
  {#if oncancel}
    <button type="button" data-testid="note-cancel" onclick={() => oncancel()}>
      {cancelLabel}
    </button>
  {/if}
  <button
    type="button"
    data-testid="note-save"
    disabled={isSaveDisabled}
    onclick={() => onsave?.(content || '<p>mock note</p>')}
  >
    {saveLabel}
  </button>
</div>
