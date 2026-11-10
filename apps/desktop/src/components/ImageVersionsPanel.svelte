<!--
  The versions of an image that are on disk: the original and the file every
  edit left next to it. The undo history of the viewer is gone once the
  document is closed; this list is how a person goes back to a version after
  that. Choosing one only changes which file the asset shows.
-->
<script lang="ts">
  import { Button, StatusBadge } from '@entropia/ui'
  import type { Asset } from '@entropia/store'
  import { getStore } from '$lib/db'
  import { getAssetUrl } from '$lib/file-import'
  import type { I18nKey, I18nParams } from '$lib/i18n'
  import { listImageVersions, type ImageVersion } from '$lib/image-versions'
  import { getLayoutByAsset } from '$lib/layouts'

  let {
    asset,
    visible = true,
    refreshKey = '',
    busy = false,
    translate,
    onUseVersion,
  }: {
    asset: Pick<Asset, 'id' | 'path'>
    /** The list reads the files of the family: only while someone can see it. */
    visible?: boolean
    /** Any change reloads the list (an edit landed, an OCR finished...). */
    refreshKey?: string
    /** An edit of this image is in progress: no version can be chosen now. */
    busy?: boolean
    translate: (key: I18nKey, params?: I18nParams) => string
    onUseVersion: (version: ImageVersion) => void
  } = $props()

  let versions = $state<ImageVersion[]>([])
  let loading = $state(false)
  let loadError = $state<string | null>(null)
  let loadToken = 0

  async function load(assetId: string) {
    const token = ++loadToken
    loading = true
    loadError = null
    try {
      const [layout, extraction] = await Promise.all([
        getLayoutByAsset(assetId),
        getStore().extractions.findByAsset(assetId),
      ])
      const loaded = await listImageVersions(assetId, {
        layout: layout ?? null,
        extraction: extraction ?? null,
      })
      if (token !== loadToken) return
      versions = loaded
    } catch (error) {
      if (token !== loadToken) return
      versions = []
      loadError = error instanceof Error ? error.message : String(error)
    } finally {
      if (token === loadToken) loading = false
    }
  }

  $effect(() => {
    const assetId = asset.id
    // The current one is the file the path names.
    void asset.path
    void refreshKey
    if (!visible) return
    void load(assetId)
  })

  function versionLabel(version: ImageVersion) {
    return version.version === 0
      ? translate('item.imageVersionOriginal')
      : translate('item.imageVersionNumber', { n: version.version })
  }

  function formatDate(ms: number) {
    return new Date(ms).toLocaleString()
  }
</script>

<section class="section" data-testid="image-versions-panel">
  <h3>{translate('item.imageVersions')}</h3>
  <p class="versions-help">{translate('item.imageVersionsHelp')}</p>

  {#if loadError}
    <p class="versions-error" role="alert">
      {translate('item.imageVersionsError', { error: loadError })}
    </p>
  {:else if versions.length === 0}
    <p class="versions-help" role="status">
      {translate(loading ? 'item.imageVersionsLoading' : 'item.imageVersionsEmpty')}
    </p>
  {:else}
    <ol class="versions-list">
      {#each versions as version (version.path)}
        {@const label = versionLabel(version)}
        <li
          class="version"
          class:version--current={version.current}
          data-testid="image-version"
          data-path={version.path}
        >
          <!-- Drawn by the WebView straight from the file, which applies the
               EXIF orientation: the thumbnail is upright like the viewer's image. -->
          <img
            class="version__thumbnail"
            src={getAssetUrl(version.path)}
            alt={translate('item.imageVersionThumbnailAlt', { label })}
            loading="lazy"
            decoding="async"
          />
          <div class="version__body">
            <div class="version__head">
              <strong>{label}</strong>
              {#if version.current}
                <StatusBadge variant="info" size="sm">
                  {translate('item.imageVersionCurrent')}
                </StatusBadge>
              {/if}
            </div>
            <p class="version__facts">
              {#if version.width !== null && version.height !== null}
                <span>{version.width}×{version.height}</span>
              {/if}
              <time datetime={new Date(version.modifiedMs).toISOString()}>
                {formatDate(version.modifiedMs)}
              </time>
            </p>
            {#if version.matchesLayout || version.matchesText}
              <div class="version__marks">
                {#if version.matchesLayout}
                  <StatusBadge variant="evidence" size="sm">
                    {translate('item.imageVersionMatchesLayout')}
                  </StatusBadge>
                {/if}
                {#if version.matchesText}
                  <StatusBadge variant="evidence" size="sm">
                    {translate('item.imageVersionMatchesText')}
                  </StatusBadge>
                {/if}
              </div>
            {/if}
            <div class="version__actions">
              <Button
                variant="secondary"
                size="sm"
                type="button"
                disabled={version.current || busy}
                onclick={() => onUseVersion(version)}
              >
                {translate('item.imageVersionUse')}
              </Button>
            </div>
          </div>
        </li>
      {/each}
    </ol>
  {/if}
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
    margin: 0;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
  }

  .versions-help {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    line-height: 1.4;
  }

  .versions-error {
    margin: 0;
    color: var(--color-danger);
    font-size: var(--font-size-sm);
  }

  .versions-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .version {
    display: flex;
    gap: var(--space-2);
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    background: var(--surface-card);
  }

  .version--current {
    border-color: var(--color-accent);
  }

  .version__thumbnail {
    flex: 0 0 auto;
    width: 72px;
    height: 72px;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    background: var(--surface-input);
    /* The whole picture, never a cropped strip: what differs between two
       versions is often their outline. */
    object-fit: contain;
  }

  .version__body {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .version__head,
  .version__marks,
  .version__actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .version__head {
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
  }

  .version__facts {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    font-variant-numeric: tabular-nums;
  }
</style>
