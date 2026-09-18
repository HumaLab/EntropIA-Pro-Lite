<script lang="ts">
  import { ActionIcon } from '../Button'
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
      <span class="topic-editor__chip">
        {topic}
        <button
          type="button"
          class="topic-editor__chip-remove"
          aria-label={`Quitar tópico ${topic}`}
          onclick={() => removeTopic(topic)}
        >
          <ActionIcon name="close" size={14} />
        </button>
      </span>
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

  .topic-editor__chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
    background-color: var(--color-accent);
    color: var(--color-bg);
    border-radius: var(--radius-sm);
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    line-height: 1.2;
    white-space: nowrap;
  }

  .topic-editor__chip-remove {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    margin-left: 2px;
    border: none;
    background: transparent;
    color: color-mix(in srgb, var(--color-bg) 76%, transparent);
    cursor: pointer;
    transition: color 0.15s ease;
  }

  .topic-editor__chip-remove :global(svg) {
    pointer-events: none;
  }

  .topic-editor__chip-remove:hover {
    color: var(--color-bg);
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
    line-height: 1.4;
  }

  .topic-editor__input::placeholder {
    color: var(--color-text-muted);
  }

  .topic-editor__suggestions {
    position: absolute;
    top: 100%;
    left: 0;
    right: 0;
    z-index: 100;
    margin-top: 2px;
    padding: var(--space-1) 0;
    max-height: 180px;
    overflow-y: auto;
    list-style: none;
    background-color: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
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
    transition: background-color 0.1s ease;
  }

  .topic-editor__suggestion:hover {
    background-color: var(--color-accent);
    color: var(--color-bg);
  }
</style>
