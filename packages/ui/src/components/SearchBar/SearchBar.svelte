<script lang="ts">
  import { ActionIcon } from '../Button'
  import { SearchClearButton } from '../SearchClearButton'
  import type { SearchBarProps } from './SearchBar.types'

  let {
    value = '',
    placeholder = '',
    debounceMs = 300,
    ariaLabel = '',
    clearAriaLabel = 'Clear search',
    emitSearch = true,
    onvaluechange,
    onsearch,
    onclear,
    oninput,
    onfocus,
    onblur,
    onkeydown,
    inputRef,
  }: SearchBarProps = $props()

  let internalValue = $state('')
  let lastExternalValue = $state<string | undefined>(undefined)
  let debounceTimer: ReturnType<typeof setTimeout> | null = null
  let inputEl = $state<HTMLInputElement | undefined>(undefined)

  function clearDebounceTimer() {
    if (!debounceTimer) {
      return
    }
    clearTimeout(debounceTimer)
    debounceTimer = null
  }

  function handleInput(e: Event) {
    const target = e.target as HTMLInputElement
    internalValue = target.value
    onvaluechange?.(internalValue, e)
    oninput?.(e)

    clearDebounceTimer()

    if (!emitSearch) {
      return
    }

    debounceTimer = setTimeout(() => {
      onsearch?.(internalValue)
    }, debounceMs)
  }

  function handleClear() {
    internalValue = ''
    clearDebounceTimer()
    onclear?.()
  }

  $effect(() => {
    if (lastExternalValue === undefined) {
      lastExternalValue = value
      internalValue = value
      return
    }

    if (value === lastExternalValue) {
      return
    }
    lastExternalValue = value
    internalValue = value
    clearDebounceTimer()
  })

  $effect(() => {
    inputRef?.(inputEl)
  })

  let showClear = $derived(internalValue.length > 0)
</script>

<div class="search-bar">
  <span class="search-bar__icon" data-testid="search-icon" aria-hidden="true">
    <ActionIcon name="search" size={16} />
  </span>
  <div class="search-bar__input-wrap">
    <input
      bind:this={inputEl}
      class="search-bar__input"
      type="search"
      {placeholder}
      aria-label={ariaLabel || placeholder || 'Search'}
      value={internalValue}
      oninput={handleInput}
      {onfocus}
      {onblur}
      {onkeydown}
    />
    {#if showClear}
      <SearchClearButton
        class="search-clear-button--overlay"
        data-testid="search-clear"
        label={clearAriaLabel}
        onclick={handleClear}
      />
    {/if}
  </div>
</div>

<style>
  .search-bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--control-height-md);
    padding: 0 var(--space-3);
    background-color: color-mix(in srgb, var(--color-surface-sunken) 88%, transparent);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
    transition:
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth),
      background-color var(--transition-smooth);
  }

  .search-bar:focus-within {
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
    background-color: var(--color-surface);
  }

  .search-bar__icon {
    display: inline-flex;
    flex-shrink: 0;
    color: var(--color-text-secondary);
    opacity: 0.9;
  }

  .search-bar__input-wrap {
    position: relative;
    flex: 1;
    min-width: 0;
  }

  .search-bar__input {
    width: 100%;
    flex: 1;
    border: none;
    outline: none;
    background: transparent;
    font-family: var(--font-sans);
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
    min-width: 0;
    line-height: var(--line-height-base);
    padding-inline-end: calc(24px + var(--space-2));
    box-sizing: border-box;
  }

  .search-bar__input::placeholder {
    color: var(--color-text-muted);
  }

  /* Remove default search input clear button */
  .search-bar__input::-webkit-search-cancel-button {
    display: none;
  }

</style>
