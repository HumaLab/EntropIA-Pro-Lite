<script lang="ts">
  /**
   * The typography preset cards, embedded directly in the Apariencia settings
   * tab.
   *
   * Extracted from the old TypographyMenu.svelte top-bar popover, which added
   * a trigger button and open/close chrome around exactly this grid. The tab
   * has no popover to open — the cards are always on screen — so only the
   * radiogroup and its cards come along; the trigger and positioning did not.
   *
   * The cards are generated from FONT_PRESETS; adding a preset never touches
   * this file. Each preview sets `data-font` on itself, so it renders in its
   * own preset's tokens while the application stays in the current one. The
   * cards are native radios: arrow keys move the choice and the group is
   * announced as one.
   */
  import { onMount } from 'svelte'
  import { ActionIcon } from '@entropia/ui'
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

  const title = $derived($translator('typography.title'))
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

  function choose(id: FontPresetId) {
    current = id
    applyFontPreset(id)
  }
</script>

<div class="font-preset-grid" role="radiogroup" aria-label={title}>
  <div class="font-preset-grid__grid">
    {#each FONT_PRESETS as preset (preset.id)}
      {@const selected = current === preset.id}
      <label class="font-preset-grid__card" class:font-preset-grid__card--selected={selected}>
        <input
          class="font-preset-grid__radio"
          type="radio"
          name="{uid}-preset"
          value={preset.id}
          checked={selected}
          aria-label={$translator(preset.label)}
          aria-describedby="{uid}-{preset.id}-families"
          onchange={() => choose(preset.id)}
        />

        <span class="font-preset-grid__head">
          <span class="font-preset-grid__name">{$translator(preset.label)}</span>
          <!-- Selection is marked by a glyph as well as by contrast, so it
               never rests on colour alone. -->
          <span class="font-preset-grid__check" aria-hidden="true">
            {#if selected}<ActionIcon name="check" size={14} />{/if}
          </span>
        </span>

        <span
          class="font-preset-grid__preview"
          data-font={preset.id}
          data-font-preview={preset.id}
          aria-hidden="true"
        >
          <span class="font-preset-grid__sample font-preset-grid__sample--ui">EntropIA</span>
          <span class="font-preset-grid__sample font-preset-grid__sample--reading"
            >{sampleReading}</span
          >
          <span class="font-preset-grid__sample font-preset-grid__sample--mono"
            >Aa 01 / corpus_1948</span
          >
        </span>

        <span class="font-preset-grid__families" id="{uid}-{preset.id}-families">
          {familiesOf(preset)}
        </span>
      </label>
    {/each}
  </div>
</div>

<style>
  .font-preset-grid {
    display: grid;
    gap: var(--space-2);
  }

  /* As many columns as fit, sized from the grid's own box — the Configuración
     sidebar sits beside the panels, so a viewport media query would be wrong
     here. */
  .font-preset-grid__grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 168px), 1fr));
    gap: var(--space-2);
  }

  .font-preset-grid__card {
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

  .font-preset-grid__card:hover {
    border-color: var(--color-border-hover);
    background: var(--surface-toolbar);
  }

  /* Selected reads as contrast, never as saturation. */
  .font-preset-grid__card--selected,
  .font-preset-grid__card--selected:hover {
    border-color: var(--color-border-strong);
    background: var(--surface-toolbar);
  }

  /* The whole card draws the ring, so there is one visible focus, not two. */
  .font-preset-grid__card:has(.font-preset-grid__radio:focus-visible) {
    box-shadow: var(--focus-ring);
  }

  .font-preset-grid__radio {
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

  .font-preset-grid__head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    min-height: 20px;
  }

  .font-preset-grid__name {
    overflow: hidden;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .font-preset-grid__card--selected .font-preset-grid__name {
    color: var(--color-text-primary);
  }

  .font-preset-grid__check {
    display: inline-flex;
    flex-shrink: 0;
    color: var(--color-text-primary);
  }

  .font-preset-grid__preview {
    display: grid;
    gap: 2px;
    min-width: 0;
    color: var(--color-text-primary);
  }

  .font-preset-grid__sample {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .font-preset-grid__sample--ui {
    font-family: var(--font-ui);
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
    line-height: var(--line-height-tight);
  }

  .font-preset-grid__sample--reading {
    font-family: var(--font-reading);
    font-size: var(--font-size-sm);
    line-height: var(--line-height-base);
  }

  .font-preset-grid__sample--mono {
    color: var(--color-text-secondary);
    font-family: var(--font-mono);
    font-size: var(--font-size-xs);
  }

  .font-preset-grid__families {
    overflow: hidden;
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
