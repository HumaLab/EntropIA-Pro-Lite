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
  import {
    ActionIcon,
    Button,
    Card,
    IconButton,
    SelectTrigger,
    ToolbarMenu,
    type ToolbarMenuItem,
  } from '@entropia/ui'
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

<Card>
  <section class="appearance-tab">
    <div class="appearance-tab__grid">
      <div class="appearance-tab__field">
        <span class="appearance-tab__label" id="appearance-theme-label">
          {t('settings.appearance.themeLabel')}
        </span>
        <ToolbarMenu label={t('settings.appearance.themeLabel')} items={themeItems}>
          {#snippet trigger(props, { open })}
            <!-- Named by its label, then by itself: the current choice. -->
            <SelectTrigger
              {...props}
              id="appearance-theme-trigger"
              variant="quiet"
              size="sm"
              {open}
              label={themeLabels[theme]}
              aria-labelledby="appearance-theme-label appearance-theme-trigger"
            />
          {/snippet}
        </ToolbarMenu>
      </div>

      <div class="appearance-tab__field">
        <span class="appearance-tab__label" id="appearance-contrast-label">
          {t('settings.appearance.contrastLabel')}
        </span>
        <ToolbarMenu label={t('settings.appearance.contrastLabel')} items={contrastItems}>
          {#snippet trigger(props, { open })}
            <SelectTrigger
              {...props}
              id="appearance-contrast-trigger"
              variant="quiet"
              size="sm"
              {open}
              label={contrastLabels[contrast]}
              aria-labelledby="appearance-contrast-label appearance-contrast-trigger"
            />
          {/snippet}
        </ToolbarMenu>
      </div>

      <div class="appearance-tab__field">
        <span class="appearance-tab__label" id="appearance-language-label">
          {t('settings.languageLabel')}
        </span>
        <ToolbarMenu label={t('settings.languageLabel')} items={localeItems}>
          {#snippet trigger(props, { open })}
            <SelectTrigger
              {...props}
              id="appearance-language-trigger"
              variant="quiet"
              size="sm"
              {open}
              label={currentLanguageOption}
              aria-labelledby="appearance-language-label appearance-language-trigger"
            />
          {/snippet}
        </ToolbarMenu>
      </div>

      <div class="appearance-tab__field">
        <span class="appearance-tab__label" id="appearance-zoom-label">
          {t('topbar.zoomTitle')}
        </span>
        <div class="appearance-tab__zoom" role="group" aria-labelledby="appearance-zoom-label">
          <IconButton
            size="sm"
            label={t('topbar.zoomOut')}
            title={t('topbar.zoomOut')}
            disabled={$currentZoom <= ZOOM_MIN}
            onclick={() => void zoomOut()}
          >
            <ActionIcon name="zoom-out" size={14} />
          </IconButton>
          <span
            class="appearance-tab__zoom-level"
            data-testid="appearance-zoom-level"
            aria-label={t('topbar.zoomLevelAria', { value: zoomPercent })}
            aria-live="polite"
          >
            {zoomPercent}%
          </span>
          <IconButton
            size="sm"
            label={t('topbar.zoomIn')}
            title={t('topbar.zoomIn')}
            disabled={$currentZoom >= ZOOM_MAX}
            onclick={() => void zoomIn()}
          >
            <ActionIcon name="zoom-in" size={14} />
          </IconButton>
          <Button variant="ghost" size="sm" onclick={() => void resetZoom()}>
            {t('topbar.zoomReset')}
          </Button>
        </div>
      </div>

      <div class="appearance-tab__field">
        <span class="appearance-tab__label" id="appearance-font-label">
          {t('typography.title')}
        </span>
        <!-- A dropdown like Tema; opened, it shows the preset cards with their
             previews instead of a plain list. -->
        <ToolbarMenu label={t('typography.title')}>
          {#snippet trigger(props, { open })}
            <SelectTrigger
              {...props}
              id="appearance-font-trigger"
              variant="quiet"
              size="sm"
              {open}
              label={t(fontLabelKey)}
              aria-labelledby="appearance-font-label appearance-font-trigger"
            />
          {/snippet}
          {#snippet children(menu)}
            <div class="appearance-tab__font-menu">
              <FontPresetGrid bind:current={font} onchoose={() => menu.close()} />
            </div>
          {/snippet}
        </ToolbarMenu>
      </div>
    </div>
  </section>
</Card>

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

  /* A plain settings list: no cards, no surface, rows split by a hairline
     (user decision, 2026-10-08), inside one full-width panel like the other
     Configuración tabs. */
  .appearance-tab__grid {
    display: flex;
    flex-direction: column;
  }

  /* Label and control on one line, the control hard right. */
  .appearance-tab__field {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-3) 0;
  }

  .appearance-tab__field + .appearance-tab__field {
    border-top: 1px solid var(--color-hairline);
  }

  .appearance-tab__label {
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
  }

  /* The choices are quiet SelectTriggers: no box at rest, only the chevron
     says it opens (user decision, 2026-10-08). */
  .appearance-tab__zoom {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
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
</style>
