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
   * cards are the design system's RadioCard: native radios, so arrow keys move
   * the choice and the group is announced as one.
   */
  import { onMount } from 'svelte'
  import { RadioCard, RadioGroup } from '@entropia/ui'
  import { translator } from '$lib/i18n'
  import {
    FONT_PRESET_DEFAULT,
    FONT_PRESETS,
    applyFontPreset,
    restoreFontPreset,
    type FontPreset,
    type FontPresetId,
  } from '$lib/typography'

  let {
    current = $bindable(FONT_PRESET_DEFAULT),
    onchoose,
  }: {
    /** The chosen preset; bindable so a menu trigger can name it. */
    current?: FontPresetId
    /** Runs after a preset is applied, e.g. to close the menu around the cards. */
    onchoose?: (id: FontPresetId) => void
  } = $props()

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
    onchoose?.(id)
  }
</script>

<RadioGroup label={title} hideLabel value={current} onchange={(id) => choose(id as FontPresetId)}>
  {#each FONT_PRESETS as preset (preset.id)}
    <RadioCard value={preset.id} label={$translator(preset.label)} description={familiesOf(preset)}>
      <span
        class="font-preset-grid__preview"
        data-font={preset.id}
        data-font-preview={preset.id}
        aria-hidden="true"
      >
        <span class="font-preset-grid__sample font-preset-grid__sample--ui">EntropIA</span>
        <span class="font-preset-grid__sample font-preset-grid__sample--reading">
          {sampleReading}
        </span>
        <span class="font-preset-grid__sample font-preset-grid__sample--mono">
          Aa 01 / corpus_1948
        </span>
      </span>
    </RadioCard>
  {/each}
</RadioGroup>

<style>
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
    font-weight: var(--font-weight-normal);
    line-height: var(--line-height-base);
  }

  .font-preset-grid__sample--mono {
    color: var(--color-text-secondary);
    font-family: var(--font-mono);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-normal);
  }
</style>
