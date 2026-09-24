<script lang="ts">
  /**
   * Apariencia: theme, contrast, zoom, typography and language, all in one
   * settings tab (user decision, 2026-09-24). Previously five separate
   * controls crowded the top bar, sitting beside navigation as if they were
   * sections themselves.
   *
   * Nothing here re-derives state or behaviour: theme/contrast/zoom/typography
   * come straight from their `$lib` modules (same as before, just no longer
   * routed through TopBar), and the typography cards are the same
   * FontPresetGrid the top bar used to open in a popover.
   */
  import { ActionIcon, Card, ToolbarMenu, tooltip, type ToolbarMenuItem } from '@entropia/ui'
  import { locale, setLocale, t, type Locale } from '$lib/i18n'
  import {
    THEME_CYCLE,
    applyTheme,
    readPersistedTheme,
    themeLabels,
    type AppTheme,
  } from '$lib/theme'
  import {
    CONTRAST_CYCLE,
    CONTRAST_STORAGE_KEY,
    applyContrast,
    contrastLabels,
    readContrast,
    type ContrastLevel,
  } from '$lib/contrast'
  import { resetZoom, zoomFactor, zoomIn, zoomOut, ZOOM_MAX, ZOOM_MIN } from '$lib/zoom'
  import FontPresetGrid from '../layout/FontPresetGrid.svelte'
  import {
    FONT_PRESETS,
    FONT_STORAGE_KEY,
    readFontPreset,
    type FontPresetId,
  } from '$lib/typography'

  /** The applied theme/contrast are already on the root element by the time
   *  this tab can ever mount (appearance.ts runs at app start); this only
   *  reads them back for the controls to reflect. */
  function readStoredContrast(): ContrastLevel {
    let stored: string | null = null
    try {
      stored = localStorage.getItem(CONTRAST_STORAGE_KEY)
    } catch {
      // Unavailable storage reads as nothing stored.
    }
    return readContrast(stored)
  }

  let theme = $state<AppTheme>(readPersistedTheme())
  let contrast = $state<ContrastLevel>(readStoredContrast())

  function readStoredFont(): FontPresetId {
    let stored: string | null = null
    try {
      stored = localStorage.getItem(FONT_STORAGE_KEY)
    } catch {
      // Unavailable storage reads as nothing stored.
    }
    return readFontPreset(stored)
  }

  let font = $state<FontPresetId>(readStoredFont())
  const fontLabelKey = $derived(
    FONT_PRESETS.find((preset) => preset.id === font)?.label ?? FONT_PRESETS[0]!.label
  )

  const currentLocale = locale
  const currentZoom = zoomFactor
  const zoomPercent = $derived(Math.round($currentZoom * 100))

  function chooseTheme(next: AppTheme) {
    applyTheme(next)
    theme = next
  }

  function chooseContrast(next: ContrastLevel) {
    applyContrast(next)
    contrast = next
  }

  async function chooseLocale(next: Locale) {
    await setLocale(next)
  }

  const themeItems = $derived<ToolbarMenuItem[]>(
    THEME_CYCLE.map((id) => ({
      kind: 'radio' as const,
      id,
      label: themeLabels[id],
      checked: theme === id,
      onselect: () => chooseTheme(id),
    }))
  )

  const contrastItems = $derived<ToolbarMenuItem[]>(
    CONTRAST_CYCLE.map((id) => ({
      kind: 'radio' as const,
      id,
      label: contrastLabels[id],
      checked: contrast === id,
      onselect: () => chooseContrast(id),
    }))
  )

  const languageOptionEs = $derived(t('settings.languageOptionEs'))
  const languageOptionEn = $derived(t('settings.languageOptionEn'))

  const localeItems = $derived<ToolbarMenuItem[]>([
    {
      kind: 'radio' as const,
      id: 'es',
      label: languageOptionEs,
      checked: $currentLocale === 'es',
      onselect: () => chooseLocale('es'),
    },
    {
      kind: 'radio' as const,
      id: 'en',
      label: languageOptionEn,
      checked: $currentLocale === 'en',
      onselect: () => chooseLocale('en'),
    },
  ])

  const currentLanguageOption = $derived(
    $currentLocale === 'en' ? languageOptionEn : languageOptionEs
  )
</script>

<section class="appearance-tab">
  <div class="appearance-tab__grid">
    <Card padding="sm">
      <div class="appearance-tab__field">
        <span class="appearance-tab__label" id="appearance-theme-label">
          {t('settings.appearance.themeLabel')}
        </span>
        <ToolbarMenu label={t('settings.appearance.themeLabel')} items={themeItems}>
          {#snippet trigger(props, { open })}
            <button
              type="button"
              class="appearance-tab__select"
              class:appearance-tab__select--open={open}
              aria-labelledby="appearance-theme-label appearance-theme-value"
              {...props}
            >
              <span id="appearance-theme-value">{themeLabels[theme]}</span>
              <ActionIcon name="chevron-down" size={12} />
            </button>
          {/snippet}
        </ToolbarMenu>
      </div>
    </Card>

    <Card padding="sm">
      <div class="appearance-tab__field">
        <span class="appearance-tab__label" id="appearance-contrast-label">
          {t('settings.appearance.contrastLabel')}
        </span>
        <ToolbarMenu label={t('settings.appearance.contrastLabel')} items={contrastItems}>
          {#snippet trigger(props, { open })}
            <button
              type="button"
              class="appearance-tab__select"
              class:appearance-tab__select--open={open}
              aria-labelledby="appearance-contrast-label appearance-contrast-value"
              {...props}
            >
              <span id="appearance-contrast-value">{contrastLabels[contrast]}</span>
              <ActionIcon name="chevron-down" size={12} />
            </button>
          {/snippet}
        </ToolbarMenu>
      </div>
    </Card>

    <Card padding="sm">
      <div class="appearance-tab__field">
        <span class="appearance-tab__label" id="appearance-language-label">
          {t('settings.languageLabel')}
        </span>
        <ToolbarMenu label={t('settings.languageLabel')} items={localeItems}>
          {#snippet trigger(props, { open })}
            <button
              type="button"
              class="appearance-tab__select"
              class:appearance-tab__select--open={open}
              aria-labelledby="appearance-language-label appearance-language-value"
              {...props}
            >
              <span id="appearance-language-value">{currentLanguageOption}</span>
              <ActionIcon name="chevron-down" size={12} />
            </button>
          {/snippet}
        </ToolbarMenu>
      </div>
    </Card>

    <Card padding="sm">
      <div class="appearance-tab__field">
        <span class="appearance-tab__label" id="appearance-zoom-label">
          {t('topbar.zoomTitle')}
        </span>
        <div class="appearance-tab__zoom" role="group" aria-labelledby="appearance-zoom-label">
          <button
            type="button"
            class="appearance-tab__zoom-step"
            aria-label={t('topbar.zoomOut')}
            use:tooltip={t('topbar.zoomOut')}
            disabled={$currentZoom <= ZOOM_MIN}
            onclick={() => void zoomOut()}
          >
            <ActionIcon name="zoom-out" size={14} />
          </button>
          <span
            class="appearance-tab__zoom-level"
            data-testid="appearance-zoom-level"
            aria-label={t('topbar.zoomLevelAria', { value: zoomPercent })}
            aria-live="polite">{zoomPercent}%</span
          >
          <button
            type="button"
            class="appearance-tab__zoom-step"
            aria-label={t('topbar.zoomIn')}
            use:tooltip={t('topbar.zoomIn')}
            disabled={$currentZoom >= ZOOM_MAX}
            onclick={() => void zoomIn()}
          >
            <ActionIcon name="zoom-in" size={14} />
          </button>
          <button type="button" class="appearance-tab__zoom-reset" onclick={() => void resetZoom()}>
            {t('topbar.zoomReset')}
          </button>
        </div>
      </div>
    </Card>

    <Card padding="sm">
      <div class="appearance-tab__field">
        <span class="appearance-tab__label" id="appearance-font-label">
          {t('typography.title')}
        </span>
        <!-- A dropdown like Tema; opened, it shows the preset cards with their
             previews instead of a plain list. -->
        <ToolbarMenu label={t('typography.title')}>
          {#snippet trigger(props, { open })}
            <button
              type="button"
              class="appearance-tab__select"
              class:appearance-tab__select--open={open}
              aria-labelledby="appearance-font-label appearance-font-value"
              {...props}
            >
              <span id="appearance-font-value">{t(fontLabelKey)}</span>
              <ActionIcon name="chevron-down" size={12} />
            </button>
          {/snippet}
          {#snippet children(menu)}
            <div class="appearance-tab__font-menu">
              <FontPresetGrid bind:current={font} onchoose={() => menu.close()} />
            </div>
          {/snippet}
        </ToolbarMenu>
      </div>
    </Card>
  </div>
</section>

<style>
  /* The preset cards need room for their previews; the menu floats in <body>,
     so this sets its width, not the card's. */
  .appearance-tab__font-menu {
    width: min(34rem, calc(100vw - 2 * var(--space-6)));
    padding: var(--space-2);
  }

  .appearance-tab {
    display: flex;
    flex-direction: column;
  }

  /* Sized from the grid's own box, not the viewport — the Configuración
     sidebar sits beside the panels, so a media query would measure the wrong
     thing. `align-items: start` stops a short card being stretched to a tall
     neighbour's height. */
  .appearance-tab__grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 320px), 1fr));
    gap: var(--space-4);
    align-items: start;
  }

  /* Label and control on one line, the control hard right. */
  .appearance-tab__field {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .appearance-tab__label {
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
  }

  /* The trigger of a preference choice. It replaces a native <select>, so it
     states every surface the operating system used to decide. */
  .appearance-tab__select {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    min-height: var(--control-height-sm);
    padding: 0 var(--space-2);
    border: 1px solid color-mix(in srgb, var(--color-hairline) 78%, transparent);
    border-radius: var(--radius-control);
    background: color-mix(in srgb, var(--color-surface-glass) 78%, transparent);
    color: var(--color-text-primary);
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    cursor: pointer;
  }

  .appearance-tab__select--open {
    border-color: var(--color-border-hover);
  }

  .appearance-tab__zoom {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }

  .appearance-tab__zoom-step {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--control-height-sm);
    height: var(--control-height-sm);
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
  }

  .appearance-tab__zoom-step:hover:not(:disabled) {
    background: var(--surface-toolbar);
    color: var(--color-text-primary);
  }

  .appearance-tab__zoom-step:disabled {
    opacity: 0.4;
    cursor: default;
  }

  .appearance-tab__zoom-level {
    min-width: 3.5ch;
    text-align: center;
    color: var(--color-text-primary);
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    font-variant-numeric: tabular-nums;
  }

  .appearance-tab__zoom-reset {
    padding: 0 var(--space-2);
    min-height: var(--control-height-sm);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-secondary);
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    cursor: pointer;
  }

  .appearance-tab__zoom-reset:hover {
    background: var(--surface-toolbar);
    color: var(--color-text-primary);
  }
</style>
