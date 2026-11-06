<script lang="ts">
  import { onMount, tick } from 'svelte'
  import { locale, t } from '$lib/i18n'
  import { testOpenrouterConnection, type ModelInfo } from '$lib/settings'
  import { requestSettingsTab } from '$lib/settings-tab-request'
  import { workspace } from '$lib/workspace'

  /**
   * A model picker fed by the OpenRouter catalog, the same suggestions
   * Configuración shows. It loads the catalog with the saved credential;
   * without one, the field is replaced by a note pointing at Configuración.
   *
   * With a credential it is a combobox, not a menu: one text field that shows
   * the chosen model, filters the whole catalog as you type (id or name) and
   * still accepts an id the catalog does not list — the engine validates that
   * when it builds the request, so a model published after this build is not
   * out of reach.
   */
  let {
    selected = '',
    onchoose,
    label,
    emptyLabel,
  }: {
    selected?: string
    onchoose: (id: string) => void
    /** Field label, shown above the input. */
    label: string
    /** The "no model of its own" option (the general model applies). */
    emptyLabel: string
  } = $props()

  const currentLocale = locale
  let models = $state<ModelInfo[]>([])
  let noKey = $state(false)
  /** The catalog could not be loaded for a reason other than the key. */
  let failed = $state(false)
  let open = $state(false)
  let focused = $state(false)
  /**
   * Whether the field is filtering. Focusing a field that already shows the
   * chosen model lists every suggestion; only what the user types narrows it.
   * A shown id is not a query until there is a keystroke behind it.
   */
  let typing = $state(false)
  let highlight = $state(-1)
  let inputEl: HTMLInputElement | undefined = $state()
  // The field starts on the parent's value; from there the parent owns it and
  // the effect below follows it whenever the user is not typing.
  // svelte-ignore state_referenced_locally
  let query = $state(selected)
  /** The last id sent to the parent, so a Tab followed by its blur commits once. */
  // svelte-ignore state_referenced_locally
  let lastEmitted = selected
  const uid = `openrouter-model-${Math.random().toString(36).slice(2, 8)}`

  /** Today's suggestions, kept as they were; typing searches past them. */
  const SUGGESTION_KEYWORDS = ['gemma', 'llama', 'mistral', 'qwen', 'claude', 'gpt']
  /** More than this is a wall of rows, not a list; the rest are announced. */
  const MAX_OPTIONS = 50

  onMount(() => {
    let alive = true
    testOpenrouterConnection('')
      .then((list) => {
        if (alive) models = Array.isArray(list) ? list : []
      })
      .catch((err: unknown) => {
        if (!alive) return
        // Only a missing credential hides the field. Any other failure leaves
        // it as plain free text, which needs no catalog to be useful.
        noKey = String(err).includes('No protected credential')
        failed = !noKey
      })
    return () => {
      alive = false
    }
  })

  $effect(() => {
    // Configuración loads the model after mount: follow the parent while the
    // field is at rest, never mid-typing. `selected === lastEmitted` is our
    // own echo coming back.
    if (selected === lastEmitted || focused) return
    query = selected
    lastEmitted = selected
  })

  function labelFor(model: ModelInfo): string {
    return `${model.id} · ${Math.round(model.context_length / 1024)}k`
  }

  /** What the typing is looking for; empty until there is a keystroke. */
  const needle = $derived(typing ? query.trim().toLowerCase() : '')

  const matches = $derived.by(() => {
    if (!needle) return []
    return models.filter(
      (model) =>
        model.id.toLowerCase().includes(needle) || (model.name ?? '').toLowerCase().includes(needle)
    )
  })

  /** Without a query: the general model, then the usual suggestions. */
  const offered = $derived.by(() => {
    if (needle) return matches
    const suggested = models.filter((model) =>
      SUGGESTION_KEYWORDS.some((keyword) => model.id.includes(keyword))
    )
    const chosen = models.find((model) => model.id === selected)
    // A model chosen earlier outside the keyword list stays reachable.
    if (chosen && !suggested.some((model) => model.id === chosen.id)) {
      return [chosen, ...suggested]
    }
    return suggested
  })

  const shown = $derived(offered.slice(0, Math.max(0, needle ? MAX_OPTIONS : MAX_OPTIONS - 1)))

  const options = $derived([
    ...(needle ? [] : [{ id: '', label: emptyLabel }]),
    ...shown.map((model) => ({ id: model.id, label: labelFor(model) })),
  ])

  const hidden = $derived(Math.max(0, offered.length - shown.length))

  const listOpen = $derived(open && options.length > 0)

  const typed = $derived(query.trim())

  const unknown = $derived.by(() => {
    if (!typed || noKey || failed) return false
    if (models.some((model) => model.id.toLowerCase() === typed.toLowerCase())) return false
    // While typing, the warning waits until nothing is matching any more: a
    // partial id still on its way to a catalog model is not a mistake.
    return !open || (typing && matches.length === 0)
  })

  /** The highlighted row when the list opens; typing highlights the first match. */
  function landHighlight() {
    if (!needle) {
      // Focusing lists the suggestions with the current model highlighted;
      // clearing the text is asking for the general model.
      highlight = typing ? 0 : options.findIndex((option) => option.id === selected)
      return
    }
    highlight = matches.length > 0 ? 0 : -1
  }

  function showList() {
    if (failed) return
    open = true
    landHighlight()
  }

  function handleFocus(event: FocusEvent) {
    focused = true
    typing = false
    // Selecting the value means the first keystroke replaces it instead of
    // appending to a model id nobody can finish by hand.
    ;(event.currentTarget as HTMLInputElement).select?.()
    showList()
  }

  function handleInput(event: Event) {
    query = (event.currentTarget as HTMLInputElement).value
    typing = true
    open = !failed
    landHighlight()
  }

  function move(delta: number) {
    if (!open) return showList()
    if (options.length === 0) return
    const from = highlight < 0 ? (delta > 0 ? -1 : 0) : highlight
    highlight = Math.min(Math.max(from + delta, 0), options.length - 1)
  }

  /** Selects the whole value, so the next keystroke replaces the id. */
  function reselect() {
    void tick().then(() => {
      if (focused) inputEl?.select()
    })
  }

  /** Adopts `id` as the field's value, committing it once. */
  function choose(id: string) {
    query = id
    typing = false
    open = false
    highlight = -1
    reselect()
    if (id === selected || id === lastEmitted) return
    lastEmitted = id
    onchoose(id)
  }

  /** Free text: an id the catalog does not list travels as typed (trimmed). */
  function commit(typedValue: string) {
    choose(typedValue.trim())
  }

  function restore() {
    query = selected
    lastEmitted = selected
    typing = false
    open = false
    highlight = -1
    reselect()
  }

  function handleKeydown(event: KeyboardEvent) {
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault()
        return move(1)
      case 'ArrowUp':
        event.preventDefault()
        return move(-1)
      case 'Enter': {
        const option = highlight >= 0 ? options[highlight] : undefined
        if (open && option) {
          event.preventDefault()
          return choose(option.id)
        }
        if (open || query !== selected) {
          event.preventDefault()
          return commit(query)
        }
        return
      }
      case 'Escape':
        if (!open) return
        event.preventDefault()
        return restore()
      case 'Tab':
        // Tab commits what is typed and then moves on, like any text field.
        return commit(query)
    }
  }

  function handleBlur() {
    focused = false
    commit(query)
    open = false
    highlight = -1
  }

  function openApiSettings() {
    requestSettingsTab('api')
    workspace.navigateActive({ name: 'settings' })
  }
</script>

<div class="model-picker">
  <span class="model-picker__label" id="{uid}-label">{label}</span>
  {#if noKey}
    <p class="model-picker__nokey">
      {$currentLocale && t('settings.modelList.noKey')}
      <button type="button" class="model-picker__link" onclick={openApiSettings}>
        {$currentLocale && t('settings.modelList.openSettings')}
      </button>
    </p>
  {:else}
    <div class="model-picker__field">
      <input
        class="model-picker__input"
        type="text"
        role="combobox"
        aria-labelledby="{uid}-label"
        aria-expanded={listOpen}
        aria-controls="{uid}-listbox"
        aria-autocomplete="list"
        aria-activedescendant={listOpen && highlight >= 0
          ? `${uid}-option-${highlight}`
          : undefined}
        bind:this={inputEl}
        placeholder={emptyLabel}
        value={query}
        autocomplete="off"
        spellcheck="false"
        oninput={handleInput}
        onkeydown={handleKeydown}
        onfocus={handleFocus}
        onblur={handleBlur}
      />
      {#if listOpen}
        <div class="model-picker__dropdown">
          <div class="model-picker__list" id="{uid}-listbox" role="listbox" aria-label={label}>
            {#each options as option, index (option.id)}
              <button
                type="button"
                class="model-picker__option"
                class:model-picker__option--active={index === highlight}
                role="option"
                id="{uid}-option-{index}"
                aria-selected={index === highlight}
                tabindex="-1"
                onmousedown={(event) => event.preventDefault()}
                onclick={() => choose(option.id)}
              >
                {option.label}
              </button>
            {/each}
          </div>
          {#if hidden > 0}
            <p class="model-picker__more">
              {$currentLocale && t('settings.modelList.moreMatches', { count: hidden })}
            </p>
          {/if}
        </div>
      {/if}
    </div>
    {#if unknown}
      <p class="model-picker__hint" role="status">
        {$currentLocale && t('settings.modelList.unknownModel')}
      </p>
    {/if}
  {/if}
</div>

<style>
  /* Same shape as the Input next to it (label above, control the width of
     the column, `control-height-md` tall), so swapping one for the other
     moves nothing. */
  .model-picker {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    flex: 1 1 0;
    min-width: 0;
  }

  .model-picker__label {
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
    user-select: none;
  }

  .model-picker__nokey {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  .model-picker__link {
    border: none;
    background: transparent;
    padding: 0;
    font: inherit;
    color: var(--color-accent);
    text-decoration: underline;
    cursor: pointer;
  }

  .model-picker__field {
    position: relative;
    min-width: 0;
  }

  /* The app's own field, never a native <select>, whose popup the operating
     system paints. Same box as Input.svelte's. */
  .model-picker__input {
    width: 100%;
    min-height: var(--control-height-md);
    max-height: var(--control-height-md);
    padding: 0 var(--space-3);
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
    background-color: color-mix(in srgb, var(--color-surface-sunken) 88%, transparent);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
    outline: none;
    transition:
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth),
      background-color var(--transition-smooth);
    box-sizing: border-box;
    line-height: var(--line-height-base);
  }

  .model-picker__input::placeholder {
    color: var(--color-text-muted);
  }

  .model-picker__input::selection {
    background: var(--color-accent-soft);
  }

  .model-picker__input:focus,
  .model-picker__input:focus-visible {
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
    background-color: color-mix(in srgb, var(--color-surface) 92%, transparent);
  }

  /* The list floats over the form below it: the column is narrow and the rows
     are longer than it, so it may not push anything down. */
  .model-picker__dropdown {
    position: absolute;
    top: calc(100% + var(--space-1));
    left: 0;
    right: 0;
    z-index: 30;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-1);
    border: 1px solid var(--border-panel);
    border-radius: var(--radius-input);
    background: var(--surface-toolbar);
    box-shadow: var(--shadow-md);
  }

  .model-picker__list {
    display: flex;
    flex-direction: column;
    max-height: 240px;
    overflow-y: auto;
  }

  .model-picker__option {
    display: block;
    width: 100%;
    min-height: 28px;
    padding: var(--space-1) var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-secondary);
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    text-align: start;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    cursor: pointer;
  }

  .model-picker__option:hover,
  .model-picker__option--active {
    background: var(--surface-input);
    color: var(--color-text-primary);
  }

  .model-picker__more {
    margin: 0;
    padding: var(--space-1) var(--space-2);
    color: var(--color-text-muted);
    font-family: var(--font-ui);
    font-size: var(--font-size-2xs);
  }

  .model-picker__hint {
    margin: 0;
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }
</style>
