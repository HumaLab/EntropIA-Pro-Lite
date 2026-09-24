<script lang="ts">
  import { flushSync } from 'svelte'
  import { workspace, MAX_TABS, type WorkspaceSnapshot } from '$lib/workspace'
  import { tabTitle, tabIcon } from '$lib/tab-meta'
  import { t, locale } from '$lib/i18n'
  import { ActionIcon, IconButton, tooltip } from '@entropia/ui'

  const currentLocale = locale

  // Manual subscription (rather than `$derived($workspace)`) so a change made
  // straight through the store — as tests and, later, keyboard shortcuts do,
  // outside any DOM event Svelte already batches — flushes synchronously
  // instead of waiting for the next microtask.
  let wsSnapshot = $state<WorkspaceSnapshot>({
    tabs: workspace.tabs,
    activeTabId: workspace.activeTabId,
    split: null,
  })

  $effect(() => {
    return workspace.subscribe((snapshot) => {
      wsSnapshot = snapshot
      // `flushSync` is safe to call even while Svelte is already flushing
      // (it tracks re-entrancy and restores the prior state on the way
      // out) — it only needs to force a flush for the case that matters
      // here: a store change from outside any Svelte-driven flush.
      flushSync()
    })
  })

  const atCap = $derived(wsSnapshot.tabs.length >= MAX_TABS)
  const newTabTitle = $derived(
    $currentLocale && (atCap ? t('tabs.newDisabledTitle') : t('tabs.new'))
  )

  function isGrouped(tabId: string): boolean {
    const split = wsSnapshot.split
    return split !== null && (split.leftId === tabId || split.rightId === tabId)
  }
</script>

<div class="tab-strip" role="tablist" aria-label={t('tabs.new')}>
  {#each wsSnapshot.tabs as tab (tab.id)}
    <div
      class="tab-strip__tab"
      class:tab-strip__tab--active={tab.id === wsSnapshot.activeTabId}
      class:tab-strip__tab--grouped={isGrouped(tab.id)}
    >
      <button
        type="button"
        class="tab-strip__select"
        role="tab"
        aria-selected={tab.id === wsSnapshot.activeTabId}
        use:tooltip={tabTitle(tab.navigation.current)}
        onclick={() => workspace.activateTab(tab.id)}
      >
        <ActionIcon name={tabIcon(tab.navigation.current)} size={14} />
        <span class="tab-strip__label">{tabTitle(tab.navigation.current)}</span>
      </button>
      {#if wsSnapshot.tabs.length > 1}
        <button
          type="button"
          class="tab-strip__close"
          aria-label={t('tabs.closeAria', { title: tabTitle(tab.navigation.current) })}
          onclick={() => workspace.closeTab(tab.id)}
        >
          <ActionIcon name="close" size={12} />
        </button>
      {/if}
    </div>
  {/each}

  <IconButton
    class="tab-strip__new"
    size="sm"
    variant="ghost"
    label={t('tabs.newAria')}
    title={newTabTitle}
    disabled={atCap}
    onclick={() => workspace.openTab()}
  >
    <ActionIcon name="add" size={14} />
  </IconButton>
</div>

<style>
  .tab-strip {
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
    overflow: hidden;
  }

  .tab-strip__tab {
    display: flex;
    align-items: center;
    min-width: 96px;
    max-width: 200px;
    flex: 1 1 auto;
    border-radius: var(--radius-sm);
  }

  .tab-strip__tab--active {
    background: var(--color-accent-faint);
  }

  /* Paired tabs render as one visual group (spec, Split view). */
  .tab-strip__tab--grouped {
    box-shadow: inset 0 -2px 0 var(--color-accent);
  }

  .tab-strip__select {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    flex: 1;
    padding: var(--space-1) var(--space-2);
    border: none;
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
  }

  .tab-strip__label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tab-strip__close {
    display: none;
    flex-shrink: 0;
    padding: 2px;
    border: none;
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
  }

  .tab-strip__tab:hover .tab-strip__close,
  .tab-strip__tab--active .tab-strip__close {
    display: inline-flex;
  }

  @media (max-width: 900px) {
    .tab-strip__label {
      display: none;
    }
  }
</style>
