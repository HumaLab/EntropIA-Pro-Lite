<script lang="ts">
  /**
   * Batch-queue status indicator for the AppShell statusbar.
   * Subscribes to the module-level BatchStore (fed by the `processing:changed`
   * Tauri event plus bounded polling while work is active) and renders a
   * StatusBadge whose variant tracks queue pressure.
   *
   * Renders NOTHING when the queue is uninitialized and idle (opt-in: the
   * footer stays intact for users who never run batches). Clicking the badge
   * opens Configuración directly on the batch tab via a focus request.
   */
  import { onDestroy, onMount } from 'svelte'
  import { locale, t } from '$lib/i18n'
  import { workspace } from '$lib/workspace'
  import { batchStore, type BatchGlobalSummary } from '$lib/batch-processing'
  import {
    bibliographyDerivedProgress,
    writingZotero,
    type BibliographyDerivedProgress,
  } from '$lib/writing-zotero'
  import { requestSettingsTab } from '$lib/settings-tab-request'
  import { StatusBadge } from '@entropia/ui'

  let summary = $state<BatchGlobalSummary>(batchStore.snapshot())
  // Which batch work the footer has already reacted to. A bibliography sync
  // admitted after startup carries a backlog this session never requested,
  // so the follower is asked again whenever the active set changes — never
  // on every refresh of the same set.
  let seenActiveIds: string | null = null
  const unsubscribe = batchStore.subscribe((next) => {
    summary = next
    const ids = next.active
      .map((batch) => batch.id)
      .sort()
      .join(',')
    if (seenActiveIds === null) {
      seenActiveIds = ids
      return
    }
    if (ids !== seenActiveIds) {
      seenActiveIds = ids
      void writingZotero.followBibliographyBacklog()
    }
  })

  // P3: while the derived backlog of a library sync (fichas and pasajes)
  // still holds work, the footer says so compactly — with the real counts,
  // not a generic "running".
  let bibliography = $state<BibliographyDerivedProgress | null>(null)
  const unsubscribeZotero = writingZotero.subscribe((next) => {
    const status = next.bibliographyProgress?.status ?? null
    const derived = status ? bibliographyDerivedProgress(status) : null
    bibliography = derived && derived.remaining > 0 ? derived : null
  })

  onMount(() => {
    // Idempotent: the store memoizes the bootstrap + listener attach.
    void batchStore.initialize()
    // Restart-safe: a derived backlog still draining after an app restart was
    // never requested in this session, so the footer asks for it here.
    void writingZotero.followBibliographyBacklog()
  })

  onDestroy(() => {
    unsubscribe()
    unsubscribeZotero()
  })

  const currentLocale = locale
  const activeCount = $derived(summary.active.length)
  const failedCount = $derived(summary.active.reduce((sum, batch) => sum + batch.failedUnits, 0))
  const visible = $derived(
    summary.init !== null && (activeCount > 0 || failedCount > 0 || bibliography !== null)
  )

  const label = $derived.by(() => {
    $currentLocale
    const derived = bibliography
    if (derived) {
      return t('batch.statusBibliography', {
        worksDone: derived.worksDone,
        worksTotal: derived.worksTotal,
        passagesDone: derived.passagesDone,
        passagesTotal: derived.passagesTotal,
      })
    }
    if (failedCount > 0) return t('batch.statusAttention', { count: failedCount })
    if (activeCount > 0) return t('batch.statusRunning', { count: activeCount })
    return t('batch.statusIdle')
  })

  function openBatchTab(): void {
    requestSettingsTab('batch')
    batchStore.requestFocus(summary.active[0]?.id ?? null)
    workspace.navigateActive({ name: 'settings' })
  }
</script>

{#if visible}
  <span class="batch-indicator__sep" aria-hidden="true">·</span>
  <button
    type="button"
    class="batch-indicator"
    class:batch-indicator--running={failedCount === 0}
    onclick={openBatchTab}
    aria-label={`${t('batch.openBatchTab')} — ${label}`}
  >
    <StatusBadge
      variant={failedCount > 0 ? 'danger' : 'info'}
      size="sm"
      class="batch-indicator__badge">{label}</StatusBadge
    >
  </button>
{/if}

<style>
  .batch-indicator {
    display: inline-flex;
    align-items: center;
    padding: 0;
    border: none;
    background: none;
    cursor: pointer;
    font: inherit;
    color: inherit;
  }

  .batch-indicator__sep {
    opacity: 0.4;
  }

  :global(.batch-indicator__badge) {
    min-height: 18px;
    padding: 0 var(--space-2);
    font-size: calc(0.58rem + 3px);
    letter-spacing: 0.04em;
  }

  .batch-indicator--running :global(.batch-indicator__badge) {
    animation: batch-indicator-pulse 1.6s ease-in-out infinite;
  }

  @keyframes batch-indicator-pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.55;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .batch-indicator--running :global(.batch-indicator__badge) {
      animation: none;
    }
  }
</style>
