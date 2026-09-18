<script lang="ts">
  /**
   * The typography button in the top bar, beside theme and contrast, and the
   * preset cards it opens.
   *
   * The cards are generated from FONT_PRESETS; adding a preset never touches
   * this file. Each preview sets `data-font` on itself, so it renders in its
   * own preset's tokens while the application stays in the current one.
   *
   * The cards are native radios: arrow keys move the choice and the group is
   * announced as one, with no key handling of our own beyond Escape.
   */
  import { onMount, tick } from 'svelte'
  import { ActionIcon, IconButton } from '@entropia/ui'
  import { translator } from '$lib/i18n'
  import {
    FONT_PRESET_DEFAULT,
    FONT_PRESETS,
    applyFontPreset,
    restoreFontPreset,
    type FontPreset,
    type FontPresetId,
  } from '$lib/typography'

  const uid = $props.id()

  let current = $state<FontPresetId>(FONT_PRESET_DEFAULT)
  let open = $state(false)
  let containerEl: HTMLDivElement | undefined = $state()
  let buttonEl: HTMLElement | undefined = $state()

  const title = $derived($translator('typography.title'))
  const currentPreset = $derived(
    FONT_PRESETS.find((preset) => preset.id === current) ?? FONT_PRESETS[0]!
  )
  const presetName = $derived($translator(currentPreset.label))
  /** The tooltip names only the preset; the accessible name keeps what the button is for. */
  const buttonLabel = $derived($translator('typography.buttonLabel', { preset: presetName }))
  const sampleReading = $derived($translator('typography.sampleReading'))

  /** What `@font-face` calls a family, as a person would say it. */
  function familiesOf(preset: FontPreset): string {
    return Object.values(preset.families)
      .map((family) => family.replace(/ Variable$/, ''))
      .join(' · ')
  }

  onMount(() => {
    current = restoreFontPreset()
  })

  async function toggle() {
    open = !open
    if (!open) return
    // Land on the preset in use, so arrow keys start from where the user is.
    await tick()
    containerEl?.querySelector<HTMLInputElement>('input:checked')?.focus()
  }

  function choose(id: FontPresetId) {
    current = id
    applyFontPreset(id)
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key !== 'Escape') return
    event.stopPropagation()
    open = false
    buttonEl?.querySelector('button')?.focus()
  }

  function handleFocusOut(event: FocusEvent) {
    const nextFocused = event.relatedTarget
    if (nextFocused instanceof Node && containerEl?.contains(nextFocused)) return
    open = false
  }
</script>

<div class="typography" bind:this={containerEl} onfocusout={handleFocusOut}>
  <span class="typography__trigger" bind:this={buttonEl}>
    <IconButton
      class="topbar__icon-btn"
      size="md"
      variant="secondary"
      label={buttonLabel}
      title={presetName}
      active={open}
      onclick={toggle}
    >
      <ActionIcon name="typography" size={16} />
    </IconButton>
  </span>

  {#if open}
    <div
      class="typography__menu"
      role="radiogroup"
      aria-label={title}
      tabindex="-1"
      onkeydown={handleKeydown}
    >
      <p class="typography__title" aria-hidden="true">{title}</p>

      <div class="typography__grid">
        {#each FONT_PRESETS as preset (preset.id)}
          {@const selected = current === preset.id}
          <label class="typography__card" class:typography__card--selected={selected}>
            <input
              class="typography__radio"
              type="radio"
              name="{uid}-preset"
              value={preset.id}
              checked={selected}
              aria-label={$translator(preset.label)}
              aria-describedby="{uid}-{preset.id}-families"
              onchange={() => choose(preset.id)}
            />

            <span class="typography__head">
              <span class="typography__name">{$translator(preset.label)}</span>
              <!-- Selection is marked by a glyph as well as by contrast, so it
                   never rests on colour alone. -->
              <span class="typography__check" aria-hidden="true">
                {#if selected}<ActionIcon name="check" size={14} />{/if}
              </span>
            </span>

            <span
              class="typography__preview"
              data-font={preset.id}
              data-font-preview={preset.id}
              aria-hidden="true"
            >
              <span class="typography__sample typography__sample--ui">EntropIA</span>
              <span class="typography__sample typography__sample--reading">{sampleReading}</span>
              <span class="typography__sample typography__sample--mono">Aa 01 / corpus_1948</span>
            </span>

            <span class="typography__families" id="{uid}-{preset.id}-families">
              {familiesOf(preset)}
            </span>
          </label>
        {/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .typography {
    position: relative;
    display: inline-flex;
  }

  .typography__trigger {
    display: inline-flex;
  }

  /* Same surface as the zoom and language menus beside it. */
  .typography__menu {
    position: absolute;
    top: calc(100% + var(--space-1));
    right: 0;
    z-index: 210;
    display: grid;
    gap: var(--space-2);
    width: min(400px, calc(100vw - 2 * var(--space-3)));
    padding: var(--space-2);
    border: 1px solid var(--border-panel);
    border-radius: var(--radius-dialog);
    background: color-mix(in srgb, var(--color-surface-elevated) 96%, var(--color-bg));
    box-shadow: var(--shadow-lg);
    outline: none;
  }

  .typography__title {
    margin: 0;
    padding: var(--space-1) var(--space-1) 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  /* As many columns as fit: two in the menu's width, one on a narrow window. */
  .typography__grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 168px), 1fr));
    gap: var(--space-2);
  }

  .typography__card {
    position: relative;
    display: grid;
    gap: var(--space-2);
    min-width: 0;
    padding: var(--space-2) var(--space-3) var(--space-3);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    background: transparent;
    cursor: pointer;
    transition:
      background-color var(--transition-base),
      border-color var(--transition-base);
  }

  .typography__card:hover {
    border-color: var(--color-border-hover);
    background: var(--surface-toolbar);
  }

  /* Selected reads as contrast, never as saturation — as in Checkbox. */
  .typography__card--selected,
  .typography__card--selected:hover {
    border-color: var(--color-border-strong);
    background: var(--surface-toolbar);
  }

  /* The whole card draws the ring, so there is one visible focus, not two. */
  .typography__card:has(.typography__radio:focus-visible) {
    box-shadow: var(--focus-ring);
  }

  .typography__radio {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
    border: 0;
  }

  .typography__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    min-height: 20px;
  }

  .typography__name {
    overflow: hidden;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .typography__card--selected .typography__name {
    color: var(--color-text-primary);
  }

  .typography__check {
    display: inline-flex;
    flex-shrink: 0;
    color: var(--color-text-primary);
  }

  .typography__preview {
    display: grid;
    gap: 2px;
    min-width: 0;
    color: var(--color-text-primary);
  }

  .typography__sample {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .typography__sample--ui {
    font-family: var(--font-ui);
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
    line-height: var(--line-height-tight);
  }

  .typography__sample--reading {
    font-family: var(--font-reading);
    font-size: var(--font-size-sm);
    line-height: var(--line-height-base);
  }

  .typography__sample--mono {
    color: var(--color-text-secondary);
    font-family: var(--font-mono);
    font-size: var(--font-size-xs);
  }

  .typography__families {
    overflow: hidden;
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
