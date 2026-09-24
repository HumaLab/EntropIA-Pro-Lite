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
  import { StatusBadge } from '@entropia/ui'

  let summary = $state<BatchGlobalSummary>(batchStore.snapshot())
  const unsubscribe = batchStore.subscribe((next) => {
    summary = next
  })

  onMount(() => {
    // Idempotent: the store memoizes the bootstrap + listener attach.
    void batchStore.initialize()
  })

  onDestroy(() => {
    unsubscribe()
  })

  const currentLocale = locale
  const activeCount = $derived(summary.active.length)
  const failedCount = $derived(summary.active.reduce((sum, batch) => sum + batch.failedUnits, 0))
  const visible = $derived(summary.init !== null && (activeCount > 0 || failedCount > 0))

  const label = $derived.by(() => {
    $currentLocale
    if (failedCount > 0) return t('batch.statusAttention', { count: failedCount })
    if (activeCount > 0) return t('batch.statusRunning', { count: activeCount })
    return t('batch.statusIdle')
  })

  function openBatchTab(): void {
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
