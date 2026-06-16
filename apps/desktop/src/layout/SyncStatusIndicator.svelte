<script lang="ts">
  /**
   * Cloud-sync status indicator for the AppShell statusbar (DESIGN §11).
   * Subscribes to the module-level SyncStore (fed by the `sync:status` Tauri
   * event) and renders a StatusBadge whose variant tracks the engine state.
   *
   * Renders NOTHING when the state is `disabled` (opt-in: the footer stays intact
   * for users who never enable sync). Clicking the badge opens the sync settings
   * card via `navigation.openRootSection({ name: 'settings' })`.
   */
  import { onMount, onDestroy } from 'svelte'
  import { locale, t } from '$lib/i18n'
  import { navigation } from '$lib/navigation'
  import { syncStore, badgeVariantForState } from '$lib/sync-store'
  import type { SyncStatus } from '$lib/sync'
  import { StatusBadge } from '@entropia/ui'

  let status = $state<SyncStatus>(syncStore.status)
  const unsubscribe = syncStore.subscribe((next) => {
    status = next
  })

  onMount(() => {
    // Idempotent: the store memoizes the bootstrap + listener attach.
    void syncStore.initialize()
  })

  onDestroy(() => {
    unsubscribe()
  })

  const currentLocale = locale
  const visible = $derived(status.state !== 'disabled')

  const variant = $derived(badgeVariantForState(status.state, status.conflicts))

  const label = $derived.by(() => {
    $currentLocale
    if (status.conflicts > 0) return t('sync.statusbar.conflicts', { count: status.conflicts })
    switch (status.state) {
      case 'syncing':
        return t('sync.statusbar.syncing')
      case 'offline':
        return t('sync.statusbar.offline')
      case 'error':
        return t('sync.statusbar.error')
      case 'idle':
      default:
        return t('sync.statusbar.idle')
    }
  })

  const tooltip = $derived.by(() => {
    $currentLocale
    const lines: string[] = []
    lines.push(
      status.last_sync_at
        ? t('sync.statusbar.lastSync', { when: new Date(status.last_sync_at).toLocaleString() })
        : t('sync.statusbar.neverSynced')
    )
    if (status.pending > 0) lines.push(t('sync.statusbar.pending', { count: status.pending }))
    if (status.blobs_pending > 0)
      lines.push(t('sync.statusbar.blobsPending', { count: status.blobs_pending }))
    if (status.conflicts > 0)
      lines.push(t('sync.statusbar.conflictsTooltip', { count: status.conflicts }))
    if (status.message) lines.push(status.message)
    if (status.clock_warning) lines.push(t('sync.statusbar.clockWarning'))
    return lines.join('\n')
  })

  function openSyncSettings() {
    navigation.openRootSection({ name: 'settings' })
  }
</script>

{#if visible}
  <!-- Leading separator owned by the indicator so a `disabled` state renders
       NOTHING (no orphan `·`), per DESIGN §11. -->
  <span class="sync-indicator__sep" aria-hidden="true">·</span>
  <button
    type="button"
    class="sync-indicator"
    class:sync-indicator--syncing={status.state === 'syncing'}
    onclick={openSyncSettings}
    title={tooltip}
    aria-label={`${t('sync.statusbar.openSettings')} — ${label}`}
  >
    <StatusBadge {variant} size="sm" class="sync-indicator__badge">{label}</StatusBadge>
  </button>
{/if}

<style>
  .sync-indicator {
    display: inline-flex;
    align-items: center;
    padding: 0;
    border: none;
    background: none;
    cursor: pointer;
    font: inherit;
    color: inherit;
  }

  /* Mirrors the existing `.statusbar__sep` opacity for visual consistency. */
  .sync-indicator__sep {
    opacity: 0.4;
  }

  :global(.sync-indicator__badge) {
    min-height: 18px;
    padding: 0 var(--space-2);
    font-size: calc(0.58rem + 3px);
    letter-spacing: 0.04em;
  }

  /* Subtle motion while syncing (DESIGN §11: syncing → info with subtle motion). */
  .sync-indicator--syncing :global(.sync-indicator__badge) {
    animation: sync-indicator-pulse 1.6s ease-in-out infinite;
  }

  @keyframes sync-indicator-pulse {
    0%,
    100% {
      opacity: 1;
    }
    50% {
      opacity: 0.55;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .sync-indicator--syncing :global(.sync-indicator__badge) {
      animation: none;
    }
  }
</style>
