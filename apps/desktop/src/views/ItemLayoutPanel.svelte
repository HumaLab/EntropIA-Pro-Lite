<script lang="ts">
  import { Button } from '@entropia/ui'
  import {
    LAYOUT_BLOCK_FILTERS,
    type AssetLayout,
    type LayoutBlockFilterCounts,
    type LayoutBlockFilterId,
    type LayoutBlockView,
    type LayoutPageBlockCounts,
  } from '$lib/layouts'
  import {
    formatLayoutBbox,
    getLayoutOverlaySourceMeta,
    serializeLayoutBlock,
  } from '$lib/layout-inspector'
  import type { I18nKey, I18nParams } from '$lib/i18n'
  import type { Asset } from '@entropia/store'
  import { onDestroy } from 'svelte'

  let {
    selectedAssetType,
    viewerType,
    assetLayout,
    layoutLoading,
    layoutError,
    showLayout,
    layoutActivePage,
    layoutBlockCountsByPage,
    layoutBlocks,
    layoutPageRegionCount,
    layoutRegionCount,
    layoutPageOptions,
    layoutTypeFilter,
    layoutFilterLabels,
    layoutFilterCounts,
    layoutPageBlocks,
    visibleLayoutBlocks,
    layoutHoveredBlockId,
    layoutSelectedBlockId,
    selectedLayoutBlock,
    hasLayoutData,
    translate,
    onToggleLayout,
    onPageChange,
    onFilterChange,
    onHoverBlock,
    onSelectBlock,
  }: {
    selectedAssetType: Asset['type'] | null
    viewerType: 'image' | 'pdf' | 'audio'
    assetLayout: AssetLayout | null
    layoutLoading: boolean
    layoutError: string | null
    showLayout: boolean
    layoutActivePage: number
    layoutBlockCountsByPage: LayoutPageBlockCounts
    layoutBlocks: LayoutBlockView[]
    layoutPageRegionCount: number
    layoutRegionCount: number
    layoutPageOptions: number[]
    layoutTypeFilter: LayoutBlockFilterId
    layoutFilterLabels: Record<LayoutBlockFilterId, string>
    layoutFilterCounts: LayoutBlockFilterCounts
    layoutPageBlocks: LayoutBlockView[]
    visibleLayoutBlocks: LayoutBlockView[]
    layoutHoveredBlockId: string | null
    layoutSelectedBlockId: string | null
    selectedLayoutBlock: LayoutBlockView | null
    hasLayoutData: boolean
    translate: (key: I18nKey, params?: I18nParams) => string
    onToggleLayout: (show: boolean) => void
    onPageChange: (page: number) => void
    onFilterChange: (filter: LayoutBlockFilterId) => void
    onHoverBlock: (blockId: string | null) => void
    onSelectBlock: (blockId: string | null) => void
  } = $props()

  let layoutBlockListEl = $state<HTMLDivElement | null>(null)
  let lastAutoScrolledLayoutBlockId = $state<string | null>(null)
  let layoutInspectorCopyMessage = $state<{ tone: 'success' | 'error'; text: string } | null>(null)
  let layoutInspectorCopyTimer = $state<ReturnType<typeof setTimeout> | null>(null)

  function scrollSelectedLayoutBlockIntoView(blockId: string | null) {
    if (!layoutBlockListEl || !blockId) return
    const selector = `[data-layout-block-id="${blockId}"]`
    const blockEl = layoutBlockListEl.querySelector<HTMLElement>(selector)
    blockEl?.scrollIntoView({ block: 'nearest', behavior: 'smooth' })
  }

  function clearLayoutInspectorCopyMessage() {
    if (layoutInspectorCopyTimer) {
      clearTimeout(layoutInspectorCopyTimer)
      layoutInspectorCopyTimer = null
    }
  }

  function showLayoutInspectorCopyMessage(tone: 'success' | 'error', text: string) {
    clearLayoutInspectorCopyMessage()
    layoutInspectorCopyMessage = { tone, text }
    layoutInspectorCopyTimer = setTimeout(() => {
      layoutInspectorCopyMessage = null
      layoutInspectorCopyTimer = null
    }, 2200)
  }

  async function copyLayoutInspectorValue(value: string, successText: string) {
    try {
      if (!navigator?.clipboard?.writeText) {
        throw new Error('Clipboard API unavailable')
      }

      await navigator.clipboard.writeText(value)
      showLayoutInspectorCopyMessage('success', successText)
    } catch {
      showLayoutInspectorCopyMessage('error', 'No se pudo copiar desde el inspector.')
    }
  }

  $effect(() => {
    const selectedBlockId = layoutSelectedBlockId

    if (!selectedBlockId) {
      lastAutoScrolledLayoutBlockId = null
      return
    }

    if (!visibleLayoutBlocks.some((block) => block.id === selectedBlockId)) {
      lastAutoScrolledLayoutBlockId = null
      return
    }

    if (lastAutoScrolledLayoutBlockId === selectedBlockId) {
      return
    }

    lastAutoScrolledLayoutBlockId = selectedBlockId
    queueMicrotask(() => {
      scrollSelectedLayoutBlockIntoView(selectedBlockId)
    })
  })

  onDestroy(clearLayoutInspectorCopyMessage)
</script>

{#if selectedAssetType && selectedAssetType !== 'audio'}
  <section class="section">
    <div class="layout-section-header">
      <div>
        <h3>
          {translate('item.layoutTab')}{#if viewerType === 'pdf'}
            {translate('item.pageInline', { page: layoutActivePage })}{/if}
        </h3>
        {#if assetLayout}
          <p class="layout-meta">
            {assetLayout.model} · {viewerType === 'pdf'
              ? translate('item.layoutBlocksCount', {
                  count: layoutBlockCountsByPage[layoutActivePage] ?? 0,
                })
              : translate('item.layoutBlocksCount', { count: layoutBlocks.length })} · {viewerType ===
            'pdf'
              ? translate('item.layoutRegionsCount', { count: layoutPageRegionCount })
              : translate('item.layoutRegionsCount', { count: layoutRegionCount })}
          </p>
        {/if}
      </div>

      <Button
        variant="secondary"
        size="sm"
        type="button"
        disabled={!hasLayoutData}
        aria-pressed={showLayout}
        onclick={() => onToggleLayout(!showLayout)}
      >
        {showLayout ? translate('item.layoutToggleHide') : translate('item.layoutToggleShow')}
      </Button>
    </div>

    {#if layoutLoading}
      <p class="empty-text">{translate('item.layoutLoading')}</p>
    {:else if layoutError}
      <p class="error">{translate('item.layoutLoadError', { error: layoutError })}</p>
    {:else if !assetLayout}
      <p class="empty-text">{translate('item.layoutMissing')}</p>
    {:else if layoutBlocks.length === 0}
      <p class="empty-text">{translate('item.layoutNoBlocks')}</p>
    {:else}
      {#if showLayout}
        <p class="layout-help">{translate('item.layoutHelp')}</p>
      {/if}

      {#if viewerType === 'pdf' && layoutPageOptions.length > 1}
        <div class="layout-page-toolbar">
          <p class="layout-page-summary" data-testid="layout-page-summary">
            {translate('item.pageOf', {
              page: layoutActivePage,
              total: layoutPageOptions.length,
            })}
          </p>

          <div
            class="layout-page-group"
            role="group"
            aria-label={translate('item.layoutPageSelect')}
          >
            {#each layoutPageOptions as page (page)}
              <button
                type="button"
                class:active={layoutActivePage === page}
                class="layout-page-chip"
                data-testid={`layout-page-chip-${page}`}
                aria-pressed={layoutActivePage === page}
                onclick={() => onPageChange(page)}
              >
                <span>{translate('item.pageShort', { page })}</span>
                <span class="layout-page-chip__count">{layoutBlockCountsByPage[page] ?? 0}</span>
              </button>
            {/each}
          </div>
        </div>
      {/if}

      <div class="layout-filter-toolbar">
        <div
          class="layout-filter-group"
          role="group"
          aria-label={translate('item.layoutFilterGroup')}
        >
          {#each LAYOUT_BLOCK_FILTERS as filter (filter.id)}
            {@const count = layoutFilterCounts[filter.id]}
            <button
              type="button"
              class:active={layoutTypeFilter === filter.id}
              class="layout-filter-chip"
              data-testid={`layout-filter-${filter.id}`}
              aria-pressed={layoutTypeFilter === filter.id}
              onclick={() => onFilterChange(filter.id)}
            >
              <span>{layoutFilterLabels[filter.id]}</span>
              <span
                class="layout-filter-chip__count"
                data-testid={`layout-filter-count-${filter.id}`}
              >
                {count}
              </span>
            </button>
          {/each}
        </div>

        <p class="layout-filter-summary">
          {translate('item.layoutShowing', {
            visible: visibleLayoutBlocks.length,
            total: layoutPageBlocks.length,
          })}
        </p>
      </div>

      {#if layoutPageBlocks.length === 0}
        <p class="empty-text">{translate('item.layoutNoPageBlocks')}</p>
      {:else if visibleLayoutBlocks.length === 0}
        <p class="empty-text">{translate('item.layoutNoFilterBlocks')}</p>
      {:else}
        <div class="layout-block-list" bind:this={layoutBlockListEl}>
          {#each visibleLayoutBlocks as block (block.id)}
            {@const isHovered = layoutHoveredBlockId === block.id}
            {@const isSelected = layoutSelectedBlockId === block.id}
            {@const overlayMeta = getLayoutOverlaySourceMeta(block.overlaySource)}
            <button
              type="button"
              data-testid={`layout-block-item-${block.id}`}
              data-layout-block-id={block.id}
              class:hovered={isHovered}
              class:selected={isSelected}
              class:fallback={block.overlaySource === 'block'}
              class="layout-block-item"
              onmouseenter={() => onHoverBlock(block.id)}
              onmouseleave={() => onHoverBlock(null)}
              onclick={() => onSelectBlock(block.id)}
            >
              <span class="layout-block-order">#{block.order}</span>
              <span class="layout-block-content">
                <span class="layout-block-heading">
                  <span class="layout-block-label">{block.label}</span>
                  <span
                    class:layout-block-source-badge--fallback={block.overlaySource === 'block'}
                    class="layout-block-source-badge"
                  >
                    {overlayMeta.shortLabel}
                  </span>
                  {#if viewerType === 'pdf'}
                    <span class="layout-block-page-chip"
                      >{translate('item.pageShort', { page: block.page })}</span
                    >
                  {/if}
                </span>
                <span class="layout-block-preview"
                  >{block.preview || translate('item.layoutNoPreview')}</span
                >
              </span>
            </button>
          {/each}
        </div>

        <div class="layout-inspector" data-testid="layout-block-inspector">
          {#if selectedLayoutBlock}
            {@const overlayMeta = getLayoutOverlaySourceMeta(selectedLayoutBlock.overlaySource)}
            <div class="layout-inspector__header">
              <div>
                <p class="layout-inspector__eyebrow">
                  {translate('item.layoutInspector')}
                </p>
                <h4>
                  {translate('item.layoutSelectedBlock', {
                    order: selectedLayoutBlock.order,
                  })}
                </h4>
              </div>

              <div class="layout-inspector__actions">
                <Button
                  variant="secondary"
                  size="sm"
                  type="button"
                  data-testid="layout-inspector-copy-text"
                  disabled={!selectedLayoutBlock.content.trim()}
                  onclick={() =>
                    copyLayoutInspectorValue(
                      selectedLayoutBlock.content,
                      translate('item.layoutCopiedText')
                    )}
                >
                  {translate('item.layoutCopyText')}
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  type="button"
                  data-testid="layout-inspector-copy-bbox"
                  onclick={() =>
                    copyLayoutInspectorValue(
                      formatLayoutBbox(selectedLayoutBlock.overlayBbox),
                      translate('item.layoutCopiedBbox')
                    )}
                >
                  {translate('item.layoutCopyBbox')}
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  type="button"
                  data-testid="layout-inspector-copy-json"
                  onclick={() =>
                    copyLayoutInspectorValue(
                      serializeLayoutBlock(selectedLayoutBlock),
                      translate('item.layoutCopiedJson')
                    )}
                >
                  {translate('item.layoutCopyJson')}
                </Button>
              </div>
            </div>

            <div class="layout-inspector__grid">
              <div>
                <span class="layout-inspector__label">{translate('item.layoutLabel')}</span>
                <strong data-testid="layout-inspector-label">{selectedLayoutBlock.label}</strong>
              </div>
              <div>
                <span class="layout-inspector__label">{translate('item.layoutOrder')}</span>
                <strong>#{selectedLayoutBlock.order}</strong>
              </div>
              <div>
                <span class="layout-inspector__label">{translate('item.layoutPage')}</span>
                <strong>{selectedLayoutBlock.page}</strong>
              </div>
              <div>
                <span class="layout-inspector__label">{translate('item.layoutGroup')}</span>
                <strong>{selectedLayoutBlock.groupId || '—'}</strong>
              </div>
              <div>
                <span class="layout-inspector__label">{translate('item.layoutBlockBbox')}</span>
                <code>{formatLayoutBbox(selectedLayoutBlock.bbox)}</code>
              </div>
              <div>
                <span class="layout-inspector__label">{translate('item.layoutOverlayBbox')}</span>
                <code data-testid="layout-inspector-bbox"
                  >{formatLayoutBbox(selectedLayoutBlock.overlayBbox)}</code
                >
              </div>
              <div class="layout-inspector__field layout-inspector__field--wide">
                <span class="layout-inspector__label">{translate('item.layoutOverlaySource')}</span>
                <strong
                  class:layout-inspector__source--fallback={selectedLayoutBlock.overlaySource ===
                    'block'}
                  class="layout-inspector__source"
                  data-testid="layout-inspector-overlay-source"
                >
                  {overlayMeta.label}
                </strong>
                <p>{overlayMeta.description}</p>
              </div>
            </div>

            <div class="layout-inspector__content">
              <span class="layout-inspector__label">{translate('item.layoutPreview')}</span>
              <pre data-testid="layout-inspector-content">{selectedLayoutBlock.content ||
                  translate('item.layoutNoFullText')}</pre>
            </div>

            {#if layoutInspectorCopyMessage}
              <p
                class:layout-inspector__message--error={layoutInspectorCopyMessage.tone === 'error'}
                class="layout-inspector__message"
                data-testid="layout-inspector-copy-message"
              >
                {layoutInspectorCopyMessage.text}
              </p>
            {/if}
          {:else}
            <div class="layout-inspector__empty" data-testid="layout-inspector-empty">
              {translate('item.layoutEmptyInspector')}
            </div>
          {/if}
        </div>
      {/if}
    {/if}
  </section>
{:else}
  <section class="section">
    <p class="empty-text">{translate('item.layoutUnavailableForAudio')}</p>
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

  .section h3 {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
    margin-bottom: var(--space-1);
  }

  .layout-section-header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .layout-meta {
    margin: var(--space-1) 0 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .layout-help {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .layout-page-toolbar {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .layout-page-summary {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .layout-page-group {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .layout-page-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-control);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font-size: var(--font-size-xs);
    cursor: pointer;
    transition:
      border-color var(--transition-base),
      background-color var(--transition-base),
      color var(--transition-base),
      box-shadow var(--transition-base);
  }

  .layout-page-chip:hover {
    border-color: var(--border-focus);
    background: var(--color-accent-faint);
  }

  .layout-page-chip.active {
    border-color: var(--border-focus);
    background: var(--color-accent-soft);
    color: var(--color-text-primary);
  }

  .layout-page-chip:focus-visible {
    outline: none;
    border-color: var(--border-focus);
    box-shadow: var(--focus-ring);
  }

  .layout-page-chip__count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 22px;
    padding: 2px var(--space-2);
    border-radius: var(--radius-control);
    background: var(--surface-card);
    font-variant-numeric: tabular-nums;
  }

  .layout-filter-toolbar {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  .layout-filter-group {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .layout-filter-chip {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-control);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font-size: var(--font-size-xs);
    cursor: pointer;
    transition:
      border-color var(--transition-base),
      background-color var(--transition-base),
      color var(--transition-base),
      box-shadow var(--transition-base);
  }

  .layout-filter-chip:hover {
    border-color: var(--border-focus);
    background: var(--color-accent-faint);
  }

  .layout-filter-chip.active {
    border-color: var(--border-focus);
    background: var(--color-accent-soft);
    color: var(--color-text-primary);
  }

  .layout-filter-chip:focus-visible {
    outline: none;
    border-color: var(--border-focus);
    box-shadow: var(--focus-ring);
  }

  .layout-filter-chip__count {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 22px;
    padding: 2px var(--space-2);
    border-radius: var(--radius-control);
    background: var(--surface-card);
    font-variant-numeric: tabular-nums;
  }

  .layout-filter-summary {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .layout-block-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-height: 320px;
    overflow: auto;
  }

  .layout-block-item {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    background: var(--surface-card);
    color: inherit;
    text-align: left;
    cursor: pointer;
    transition:
      border-color var(--transition-base),
      background-color var(--transition-base),
      box-shadow var(--transition-base);
  }

  .layout-block-item:hover,
  .layout-block-item.hovered,
  .layout-block-item.selected {
    border-color: var(--color-accent);
    background: var(--color-accent-faint);
  }

  .layout-block-item.hovered:not(.selected) {
    border-color: var(--color-warning);
    background: color-mix(in srgb, var(--color-warning) 10%, var(--surface-card));
  }

  .layout-block-item.selected {
    box-shadow: var(--focus-ring);
  }

  .layout-block-item.fallback {
    border-style: dashed;
  }

  .layout-block-item.fallback.selected {
    border-color: color-mix(in srgb, var(--color-warning) 65%, var(--color-accent));
    background: color-mix(in srgb, var(--color-warning) 14%, var(--surface-card));
  }

  .layout-block-item:focus-visible {
    outline: none;
    border-color: var(--border-focus);
    box-shadow: var(--focus-ring);
  }

  .layout-block-order {
    flex-shrink: 0;
    min-width: 42px;
    color: var(--color-text-muted);
    font-family: var(--font-mono);
    font-size: var(--font-size-xs);
  }

  .layout-block-content {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .layout-block-heading {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }

  .layout-block-label {
    text-transform: uppercase;
    letter-spacing: 0.04em;
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  .layout-block-source-badge,
  .layout-block-page-chip {
    display: inline-flex;
    align-items: center;
    padding: 2px var(--space-2);
    border-radius: var(--radius-control);
    font-size: var(--font-size-2xs);
    line-height: var(--line-height-tight);
    border: 1px solid color-mix(in srgb, var(--color-accent) 35%, var(--border-subtle));
    background: color-mix(in srgb, var(--color-accent) 10%, var(--surface-card));
    color: var(--color-text-secondary);
  }

  .layout-block-source-badge--fallback {
    border-color: color-mix(in srgb, var(--color-warning) 45%, var(--border-subtle));
    background: color-mix(in srgb, var(--color-warning) 12%, var(--surface-card));
  }

  .layout-block-preview {
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    line-height: 1.4;
    word-break: break-word;
  }

  .layout-inspector {
    margin-top: var(--space-3);
    padding: var(--space-3);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--surface-card);
  }

  .layout-inspector__header {
    display: flex;
    justify-content: space-between;
    gap: var(--space-3);
    align-items: flex-start;
  }

  .layout-inspector__eyebrow {
    margin: 0 0 4px;
    font-size: var(--font-size-2xs);
    text-transform: uppercase;
    letter-spacing: 0.075em;
    color: var(--color-text-muted);
  }

  .layout-inspector h4 {
    margin: 0;
    font-size: var(--font-size-md);
    color: var(--color-text-primary);
  }

  .layout-inspector__actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
    justify-content: flex-end;
  }

  .layout-inspector__grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: var(--space-3);
    margin-top: var(--space-3);
  }

  .layout-inspector__field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .layout-inspector__field--wide {
    grid-column: 1 / -1;
  }

  .layout-inspector__label {
    display: block;
    margin-bottom: 4px;
    font-size: var(--font-size-2xs);
    text-transform: uppercase;
    letter-spacing: 0.075em;
    color: var(--color-text-muted);
  }

  .layout-inspector__grid strong,
  .layout-inspector__grid code {
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
  }

  .layout-inspector__grid p {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    line-height: 1.4;
  }

  .layout-inspector__source {
    display: inline-flex;
    align-items: center;
    width: fit-content;
    padding: 4px 10px;
    border-radius: var(--radius-control);
    background: color-mix(in srgb, var(--color-accent) 12%, var(--surface-card));
    border: 1px solid color-mix(in srgb, var(--color-accent) 35%, var(--border-subtle));
  }

  .layout-inspector__source--fallback {
    background: color-mix(in srgb, var(--color-warning) 14%, var(--surface-card));
    border-color: color-mix(in srgb, var(--color-warning) 45%, var(--border-subtle));
  }

  .layout-inspector__content {
    margin-top: var(--space-3);
  }

  .layout-inspector__content pre {
    margin: 0;
    padding: var(--space-3);
    max-height: 220px;
    overflow: auto;
    white-space: pre-wrap;
    word-break: break-word;
    border-radius: var(--radius-md);
    border: 1px solid var(--border-subtle);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font-family: var(--font-sans);
    font-size: var(--font-size-sm);
    line-height: 1.5;
  }

  .layout-inspector__message,
  .layout-inspector__empty {
    margin: var(--space-3) 0 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .layout-inspector__message {
    width: fit-content;
    padding: var(--space-1) var(--space-2);
    border: 1px solid color-mix(in srgb, var(--color-success) 28%, transparent);
    border-radius: var(--radius-sm);
    background: var(--color-success-soft);
    color: var(--color-success);
  }

  .layout-inspector__message--error {
    border-color: color-mix(in srgb, var(--color-danger) 30%, transparent);
    background: var(--color-danger-soft);
    color: var(--color-danger);
  }

  .empty-text {
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .error {
    color: var(--color-danger);
  }
</style>
