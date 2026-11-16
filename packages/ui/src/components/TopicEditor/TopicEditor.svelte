<script lang="ts">
  import { ActionIcon } from '../Button'
  import StatusBadge from '../StatusBadge/StatusBadge.svelte'
  import type { TopicEditorProps } from './TopicEditor.types'

  let { topics = [], suggestions = [], onchange }: TopicEditorProps = $props()

  let inputText = $state('')
  let showSuggestions = $state(false)
  let inputEl: HTMLInputElement | undefined = $state(undefined)
  let containerEl: HTMLDivElement | undefined = $state(undefined)

  // Filter suggestions to exclude already-added topics and match input
  let filteredSuggestions = $derived.by(() => {
    const input = inputText.trim().toUpperCase()
    const existing = new Set(topics)
    return suggestions.filter((s) => {
      if (existing.has(s)) return false
      if (input && !s.startsWith(input) && !s.includes(input)) return false
      return true
    })
  })

  function normalizeInput(raw: string): string[] {
    return [
      ...new Set(
        raw
          .split(',')
          .map((s) => s.trim().toUpperCase())
          .filter(Boolean)
      ),
    ]
  }

  function handleKeydown(e: KeyboardEvent) {
    // On Enter or comma, parse and add topics
    if (e.key === 'Enter' || e.key === ',') {
      e.preventDefault()
      addFromInput()
    }
    // On Backspace with empty input, remove last topic
    if (e.key === 'Backspace' && !inputText && topics.length > 0) {
      removeTopic(topics[topics.length - 1]!)
    }
  }

  function addFromInput() {
    const newTopics = normalizeInput(inputText)
    if (newTopics.length === 0) return
    // Merge with existing, deduplicate
    const merged = [...new Set([...topics, ...newTopics])]
    inputText = ''
    showSuggestions = false
    onchange?.(merged)
  }

  function selectSuggestion(name: string) {
    const merged = [...new Set([...topics, name])]
    inputText = ''
    showSuggestions = false
    inputEl?.focus()
    onchange?.(merged)
  }

  function removeTopic(name: string) {
    const updated = topics.filter((t) => t !== name)
    onchange?.(updated)
  }

  function handleBlur() {
    // Delay to allow click on suggestion to register
    setTimeout(() => {
      showSuggestions = false
      // If there's pending text, add it
      if (inputText.trim()) {
        addFromInput()
      }
    }, 150)
  }

  function handleFocus() {
    showSuggestions = true
  }

  function handleInput(e: Event) {
    const target = e.target as HTMLInputElement
    inputText = target.value
    showSuggestions = true
  }
</script>

<div class="topic-editor" bind:this={containerEl}>
  <div class="topic-editor__input-area">
    {#each topics as topic (topic)}
      <StatusBadge class="topic-editor__chip" variant="neutral" size="sm">
        {topic}
        <button
          type="button"
          class="topic-editor__chip-remove"
          aria-label={`Quitar tópico ${topic}`}
          onclick={() => removeTopic(topic)}
        >
          <ActionIcon name="close" size={14} />
        </button>
      </StatusBadge>
    {/each}
    <input
      bind:this={inputEl}
      class="topic-editor__input"
      type="text"
      placeholder={topics.length === 0 ? 'Escribí un tópico...' : 'Agregar más...'}
      value={inputText}
      oninput={handleInput}
      onkeydown={handleKeydown}
      onfocus={handleFocus}
      onblur={handleBlur}
    />
  </div>

  {#if showSuggestions && filteredSuggestions.length > 0}
    <ul class="topic-editor__suggestions">
      {#each filteredSuggestions as suggestion (suggestion)}
        <li>
          <button
            type="button"
            class="topic-editor__suggestion"
            onmousedown={() => selectSuggestion(suggestion)}
          >
            {suggestion}
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .topic-editor {
    position: relative;
  }

  .topic-editor__input-area {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2);
    min-height: var(--control-height-lg);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-input);
    background-color: var(--color-surface);
    transition: border-color var(--transition-base);
    cursor: text;
  }

  .topic-editor__input-area:focus-within {
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
  }

  /* The chip is a neutral StatusBadge; only its remove control is drawn here,
     an inline icon inside the badge (too small for an IconButton). */
  .topic-editor__chip-remove {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    margin-left: var(--space-0-5);
    border: none;
    border-radius: var(--radius-xs);
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
    transition: color var(--transition-base);
  }

  .topic-editor__chip-remove :global(svg) {
    pointer-events: none;
  }

  .topic-editor__chip-remove:hover {
    color: var(--color-text-primary);
  }

  .topic-editor__chip-remove:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
    color: var(--color-text-primary);
  }

  .topic-editor__input {
    flex: 1;
    min-width: 80px;
    padding: 0;
    border: none;
    outline: none;
    background: transparent;
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
    line-height: var(--line-height-base);
  }

  .topic-editor__input::placeholder {
    color: var(--color-text-muted);
  }

  .topic-editor__suggestions {
    position: absolute;
    top: 100%;
    left: 0;
    right: 0;
    z-index: var(--z-floating);
    margin-top: var(--space-0-5);
    padding: var(--space-1) 0;
    max-height: 180px;
    overflow-y: auto;
    list-style: none;
    background-color: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-dialog);
    box-shadow: var(--shadow-md);
  }

  .topic-editor__suggestion {
    display: block;
    width: 100%;
    padding: var(--space-2) var(--space-3);
    border: none;
    background: transparent;
    text-align: left;
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
    cursor: pointer;
    transition: background-color var(--transition-base);
  }

  /* A hovered suggestion reads by contrast (the soft hover surface), never by
     an accent fill. */
  .topic-editor__suggestion:hover {
    background-color: var(--color-accent-faint);
  }

  @media (prefers-reduced-motion: reduce) {
    .topic-editor__input-area,
    .topic-editor__chip-remove,
    .topic-editor__suggestion {
      transition: none;
    }
  }
</style>
