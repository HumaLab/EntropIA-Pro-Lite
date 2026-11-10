<!--
  What the Layout tab shows instead of a layout that was computed on another
  version of the image: the layout is kept, it is just not this image's, and
  running it again replaces it.
-->
<script lang="ts">
  import { Button } from '@entropia/ui'
  import type { I18nKey, I18nParams } from '$lib/i18n'
  import type { LayoutVersionVerdict } from '$lib/layout-version'

  let {
    verdict,
    busy = false,
    translate,
    onRerun,
  }: {
    verdict: Extract<LayoutVersionVerdict, { status: 'other_version' }>
    /** An OCR of this image is already running. */
    busy?: boolean
    translate: (key: I18nKey, params?: I18nParams) => string
    onRerun: () => void
  } = $props()

  // The sizes explain the mismatch only when they differ: an erase or a half
  // turn changes the image and keeps its size.
  let message = $derived.by(() => {
    const { layout, current } = verdict
    if (!current || (current.width === layout.width && current.height === layout.height)) {
      return translate('item.layoutOtherVersion')
    }
    return translate('item.layoutOtherVersionSized', {
      width: layout.width,
      height: layout.height,
      currentWidth: current.width,
      currentHeight: current.height,
    })
  })
</script>

<section class="section" data-testid="layout-version-notice">
  <h3>{translate('item.layoutTab')}</h3>
  <p class="notice" role="status">{message}</p>
  <div class="actions">
    <Button
      variant="secondary"
      size="sm"
      type="button"
      disabled={busy}
      title={busy ? translate('item.ocrHighBusyTitle') : undefined}
      onclick={() => onRerun()}
    >
      {translate('item.layoutRerun')}
    </Button>
  </div>
</section>

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: var(--color-surface);
    box-shadow: var(--shadow-surface);
  }

  .section h3 {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
  }

  .notice {
    margin: 0;
    padding: var(--space-2);
    border: 1px solid color-mix(in srgb, var(--color-warning) 35%, transparent);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--color-warning) 12%, var(--surface-card));
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    line-height: 1.4;
  }

  .actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }
</style>
