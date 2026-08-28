<script lang="ts">
  import { getAssetUrl } from '$lib/file-import'
  import { getAssetDisplayPath, getAssetPathLabel, getAssetTypeLabel } from '$lib/item-metadata'
  import { splitHighlightedSegments } from '$lib/item-view-search'
  import { SearchClearButton } from '@entropia/ui'
  import type { I18nKey, I18nParams } from '$lib/i18n'
  import type { SimilarAsset } from '$lib/nlp'

  export type ItemFtsResult = {
    itemId: string
    title: string
    rank: number
    collectionId: string
  }

  export type ItemFtsDebug = {
    rawQuery: string
    sanitizedQuery: string
    strategy: 'empty' | 'strict' | 'relaxed'
    matchCount: number
    hydratedCount: number
    resultIds: string[]
  }

  let {
    assetsCount,
    selectedAsset,
    selectedAssetIndex,
    ftsQuery,
    ftsResults,
    ftsSearching,
    ftsSearchError,
    ftsIndexedRows,
    ftsDebug,
    ftsReadinessKey,
    similarAssets,
    similarAssetsReadinessKey,
    isDev,
    translate,
    onFtsInput,
    onFtsKeydown,
    onFtsClear,
    onNavigateToFtsItem,
    onPreviewSimilarAsset,
  }: {
    assetsCount: number
    selectedAsset: boolean
    selectedAssetIndex: number
    ftsQuery: string
    ftsResults: ItemFtsResult[]
    ftsSearching: boolean
    ftsSearchError: string | null
    ftsIndexedRows: number | null
    ftsDebug: ItemFtsDebug | null
    ftsReadinessKey: I18nKey | null
    similarAssets: SimilarAsset[]
    similarAssetsReadinessKey: I18nKey | null
    isDev: boolean
    translate: (key: I18nKey, params?: I18nParams) => string
    onFtsInput: (event: Event) => void
    onFtsKeydown: (event: KeyboardEvent) => void
    onFtsClear: () => void
    onNavigateToFtsItem: (item: { itemId: string; title: string; collectionId: string }) => void
    onPreviewSimilarAsset: (asset: SimilarAsset) => void
  } = $props()

  function getSimilarAssetTitle(asset: SimilarAsset) {
    return asset.title || getAssetPathLabel(asset.assetPath) || asset.itemId
  }

  function getSimilarAssetPreview(asset: SimilarAsset) {
    return asset.textPreview
      ?.split(/\r?\n/)
      .map((line) => line.trim())
      .filter(Boolean)
      .slice(0, 3)
      .join(' ')
  }

  function isImageAsset(asset: SimilarAsset) {
    return asset.assetType.toLowerCase() === 'image'
  }
</script>

{#if assetsCount > 0}
  <section class="section">
    <div class="analysis-panel analysis-panel--tabbed">
      <div class="fts-search-section">
        <h4>{translate('item.searchBySimilarText')}</h4>
        <div class="fts-search-input-wrap">
          <input
            class="fts-search-input"
            type="search"
            placeholder={translate('item.ftsPlaceholder')}
            value={ftsQuery}
            oninput={onFtsInput}
            onkeydown={onFtsKeydown}
          />
          {#if ftsQuery}
            <SearchClearButton
              class="search-clear-button--overlay"
              label={translate('item.ftsClear')}
              onclick={onFtsClear}
            />
          {/if}
        </div>
        {#if ftsSearchError}
          <p class="ocr-error">{ftsSearchError}</p>
        {:else if ftsSearching}
          <p class="empty-text">{translate('item.ftsSearching')}</p>
        {:else if ftsQuery.trim().length === 0}
          <p class="empty-text">{translate('item.ftsPrompt')}</p>
          {#if ftsReadinessKey}
            <p class="readiness-callout">{translate(ftsReadinessKey)}</p>
          {/if}
        {:else if ftsResults.length === 0}
          <p class="empty-text">{translate('item.ftsNoResults')}</p>
          {#if ftsReadinessKey}
            <p class="readiness-callout">{translate(ftsReadinessKey)}</p>
          {/if}
        {:else}
          <ul class="similar-list">
            {#each ftsResults as result (result.itemId)}
              <li class="similar-item similar-item--search">
                <button
                  class="similar-item-btn similar-item-btn--search"
                  onclick={() => onNavigateToFtsItem(result)}
                >
                  <span class="similar-title">
                    {#each splitHighlightedSegments(result.title || result.itemId, ftsQuery) as segment, i (`${result.itemId}-seg-${i}-${segment.text}`)}
                      {#if segment.isMatch}
                        <mark class="fts-match">{segment.text}</mark>
                      {:else}
                        {segment.text}
                      {/if}
                    {/each}
                  </span>
                  <span class="similar-score similar-score--rank"
                    >{translate('item.rank', { value: result.rank.toFixed(3) })}</span
                  >
                </button>
              </li>
            {/each}
          </ul>
        {/if}

        {#if isDev}
          <details class="fts-debug-panel">
            <summary>{translate('item.ftsDebugTitle')}</summary>

            <div class="fts-debug-grid">
              <div class="fts-debug-row">
                <span class="fts-debug-label">{translate('item.ftsDebug.indexedRows')}</span>
                <code>{ftsIndexedRows ?? 'unknown'}</code>
              </div>
              <div class="fts-debug-row">
                <span class="fts-debug-label">{translate('item.ftsDebug.rawQuery')}</span>
                <code>{ftsDebug?.rawQuery ?? (ftsQuery.trim() || '—')}</code>
              </div>
              <div class="fts-debug-row">
                <span class="fts-debug-label">{translate('item.ftsDebug.sanitized')}</span>
                <code>{ftsDebug?.sanitizedQuery || '—'}</code>
              </div>
              <div class="fts-debug-row">
                <span class="fts-debug-label">{translate('item.ftsDebug.strategy')}</span>
                <code>{ftsDebug?.strategy ?? '—'}</code>
              </div>
              <div class="fts-debug-row">
                <span class="fts-debug-label">{translate('item.ftsDebug.dbMatches')}</span>
                <code>{ftsDebug?.matchCount ?? 0}</code>
              </div>
              <div class="fts-debug-row">
                <span class="fts-debug-label">{translate('item.ftsDebug.hydratedItems')}</span>
                <code>{ftsDebug?.hydratedCount ?? 0}</code>
              </div>
              <div class="fts-debug-row fts-debug-row--stacked">
                <span class="fts-debug-label">{translate('item.ftsDebug.resultIds')}</span>
                <code>{ftsDebug?.resultIds.join(', ') || '—'}</code>
              </div>
            </div>
          </details>
        {/if}
      </div>

      {#if similarAssets.length > 0}
        <div class="similar-section">
          <h4>
            {assetsCount > 1
              ? translate('item.similarAssetsPage', { page: selectedAssetIndex + 1 })
              : translate('item.similarAssets')}
          </h4>
          <ul class="similar-list">
            {#each similarAssets.slice(0, 5) as asset (asset.assetId)}
              <li class="similar-item">
                <button
                  class="similar-item-btn"
                  onclick={() => onPreviewSimilarAsset(asset)}
                  data-testid={`similar-asset-${asset.assetId}`}
                >
                  <span class="similar-thumbnail" aria-hidden="true">
                    {#if isImageAsset(asset) && asset.assetPath}
                      <img src={getAssetUrl(asset.assetPath)} alt="" loading="lazy" />
                    {:else}
                      <span>{getAssetTypeLabel(asset.assetType)}</span>
                    {/if}
                  </span>
                  <span class="similar-item-main">
                    <span class="similar-title-row">
                      <span class="similar-title">{getSimilarAssetTitle(asset)}</span>
                      <span class="similar-score">{(asset.similarity * 100).toFixed(1)}%</span>
                    </span>
                    {#if getSimilarAssetPreview(asset)}
                      <span class="similar-preview">{getSimilarAssetPreview(asset)}</span>
                    {:else}
                      <span class="similar-preview similar-preview--empty"
                        >{translate('item.similarAssetsNoPreview')}</span
                      >
                    {/if}
                  </span>
                </button>
                <details class="similar-technical-meta">
                  <summary
                    >{getAssetTypeLabel(asset.assetType)} · {getAssetPathLabel(
                      asset.assetPath
                    )}</summary
                  >
                  <span>
                    {translate('item.assetMetaLine', {
                      assetId: asset.assetId,
                      itemId: asset.itemId,
                      collectionId: asset.collectionId,
                    })}
                  </span>
                  {#if asset.assetPath && getAssetPathLabel(asset.assetPath) !== asset.assetPath}
                    <span>{getAssetDisplayPath(asset.assetPath)}</span>
                  {/if}
                </details>
              </li>
            {/each}
          </ul>
        </div>
      {:else}
        <div class="similar-section">
          <h4>
            {assetsCount > 1
              ? translate('item.similarAssetsPage', { page: selectedAssetIndex + 1 })
              : translate('item.similarAssets')}
          </h4>
          <p class="empty-text">
            {#if selectedAsset}
              {translate('item.similarAssetsEmpty')}
            {:else}
              {translate('item.similarAssetsNeedSelection')}
            {/if}
          </p>
          {#if selectedAsset && similarAssetsReadinessKey}
            <p class="readiness-callout">{translate(similarAssetsReadinessKey)}</p>
          {/if}
        </div>
      {/if}
    </div>
  </section>
{/if}

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: var(--color-surface);
    box-shadow: var(--shadow-surface);
  }

  .analysis-panel {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-top: none;
    border-radius: 0 0 var(--radius-surface) var(--radius-surface);
    overflow: hidden;
    background: var(--surface-card);
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.025);
  }

  .analysis-panel--tabbed {
    border-top: 1px solid var(--color-border);
    border-radius: var(--radius-surface);
  }

  .fts-search-section,
  .similar-section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .fts-search-section h4,
  .similar-section h4 {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
  }

  .fts-search-input-wrap {
    position: relative;
  }
  .fts-search-input {
    width: 100%;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-input);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    padding: var(--space-2) var(--space-3);
    padding-inline-end: calc(24px + var(--space-2));
    box-sizing: border-box;
    outline: none;
    font-family: var(--font-sans);
    transition:
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth);
  }

  .fts-search-input:focus {
    border-color: var(--border-focus);
    box-shadow: var(--focus-ring);
  }

  .fts-search-input::-webkit-search-cancel-button {
    display: none;
  }

  .readiness-callout {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--color-accent) 10%, transparent);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }

  .fts-match {
    background: color-mix(in srgb, var(--color-warning) 30%, transparent);
    color: var(--color-text-primary);
    border-radius: 2px;
    padding: 0 1px;
  }

  .fts-debug-panel {
    border: 1px dashed var(--color-hairline);
    border-radius: var(--radius-sm);
    padding: var(--space-2);
    background: var(--surface-card);
  }

  .fts-debug-panel summary {
    cursor: pointer;
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    font-weight: var(--font-weight-medium);
  }

  .fts-debug-grid {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .fts-debug-row {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: var(--space-2);
    font-size: var(--font-size-xs);
  }

  .fts-debug-row--stacked {
    flex-direction: column;
  }

  .fts-debug-label {
    color: var(--color-text-secondary);
    min-width: 90px;
  }

  .fts-debug-row code {
    white-space: pre-wrap;
    word-break: break-word;
    color: var(--color-text-primary);
    background: var(--surface-input);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-xs);
    padding: 2px var(--space-2);
    flex: 1;
  }

  .similar-list {
    list-style: none;
    padding: 0;
    margin: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .similar-item {
    padding: 0;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--surface-card);
    overflow: hidden;
    transition:
      background var(--transition-smooth),
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth);
  }

  .similar-item:hover {
    background: var(--surface-panel);
    border-color: var(--color-accent);
  }

  .similar-item:has(.similar-item-btn:focus-visible) {
    border-color: var(--border-focus);
    box-shadow: var(--focus-ring);
  }

  .similar-item-btn {
    display: grid;
    grid-template-columns: 72px minmax(0, 1fr);
    gap: var(--space-3);
    align-items: flex-start;
    width: 100%;
    padding: var(--space-3);
    background: none;
    border: none;
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    cursor: pointer;
    text-align: left;
  }

  .similar-item-btn:hover {
    background: transparent;
  }

  .similar-item-btn:focus-visible {
    outline: none;
  }

  .similar-item-btn--search {
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
  }

  .similar-thumbnail {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 72px;
    height: 72px;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    background: var(--surface-card);
    overflow: hidden;
    color: var(--color-text-tertiary, var(--color-text-secondary));
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.04em;
  }

  .similar-thumbnail img {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }

  .similar-item-main {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .similar-title-row {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .similar-title {
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
    line-height: 1.3;
    word-break: break-word;
  }

  .similar-preview {
    display: -webkit-box;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
    overflow: hidden;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 3;
    line-clamp: 3;
  }

  .similar-preview--empty {
    color: var(--color-text-tertiary, var(--color-text-secondary));
    font-style: italic;
  }

  .similar-technical-meta {
    margin: 0 var(--space-3) var(--space-2) calc(72px + var(--space-3) + var(--space-3));
    color: var(--color-text-tertiary, var(--color-text-secondary));
    font-size: var(--font-size-2xs);
  }

  .similar-technical-meta summary {
    cursor: pointer;
    width: fit-content;
    opacity: 0.72;
  }

  .similar-technical-meta span {
    display: block;
    margin-top: 2px;
    word-break: break-word;
  }

  .similar-score {
    font-size: var(--font-size-xs);
    color: var(--color-text-primary);
    background: var(--surface-input);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-control);
    padding: 2px var(--space-2);
    white-space: nowrap;
    font-weight: var(--font-weight-semibold);
  }

  .similar-score--rank {
    color: var(--color-text-secondary);
    font-family: var(--font-mono);
    font-weight: var(--font-weight-normal);
  }

  @media (max-width: 720px) {
    .similar-item-btn {
      grid-template-columns: 56px minmax(0, 1fr);
      gap: var(--space-2);
      padding: var(--space-2);
    }

    .similar-item-btn--search {
      grid-template-columns: minmax(0, 1fr);
    }

    .similar-thumbnail {
      width: 56px;
      height: 56px;
    }

    .similar-title-row {
      flex-direction: column;
      gap: var(--space-1);
    }

    .similar-technical-meta {
      margin: 0 var(--space-2) var(--space-2) calc(56px + var(--space-2) + var(--space-2));
    }
  }
</style>
