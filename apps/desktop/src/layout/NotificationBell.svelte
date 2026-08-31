<script lang="ts">
  /**
   * Notification-center bell for the AppShell statusbar (NOTIFICATIONS.md §1).
   *
   * Renders a bell with an unread badge fed by `SyncUsage.unread_notifications` (which
   * piggybacks on the sync cycle's `/v1/usage`). Clicking opens a dropdown panel that
   * lists notifications (`syncListNotifications`), each with a severity colour, the
   * server-rendered title + body, and a relative timestamp. Items can be marked read,
   * deleted individually, or all marked read at once; read/delete keeps the badge in sync.
   *
   * Renders NOTHING when sync is `disabled` (no session) — the bell only exists for
   * accounts with an active sync session, matching the opt-in footer policy (§11).
   * The notification CONTENT is rendered by the server; this component only owns the
   * shell labels (it never builds or translates `title`/`body`).
   */
  import { onMount, onDestroy } from 'svelte'
  import { ActionIcon, IconButton } from '@entropia/ui'
  import { locale, t } from '$lib/i18n'
  import { describeSyncError, type NotificationItem } from '$lib/sync'
  import { syncStore } from '$lib/sync-store'
  import type { SyncStatus } from '$lib/sync'
  import {
    notificationStore,
    severityVariant,
    formatRelativeTime,
    type NotificationState,
  } from '$lib/notification-store'

  let status = $state<SyncStatus>(syncStore.status)
  let hadSession = false
  const unsubStatus = syncStore.subscribe((next) => {
    const nextHasSession = next.state !== 'disabled'
    status = next
    if (nextHasSession && !hadSession) {
      void notificationStore.refreshFromUsage()
    }
    hadSession = nextHasSession
  })

  let notif = $state<NotificationState>(notificationStore.state)
  const unsubNotif = notificationStore.subscribe((next) => {
    notif = next
  })

  let open = $state(false)
  let rootEl: HTMLDivElement | undefined = $state()

  const currentLocale = locale
  // The bell only exists once there is an active sync session.
  const hasSession = $derived(status.state !== 'disabled')
  const unread = $derived(notif.unread)
  const hasUnread = $derived(unread > 0)

  const bellLabel = $derived.by(() => {
    $currentLocale
    return hasUnread
      ? t('sync.notif.bellLabelCount', { count: unread })
      : t('sync.notif.bellLabel')
  })

  onMount(() => {
    // Idempotent: the sync store memoizes bootstrap; usage powers the badge.
    void syncStore.initialize()
  })

  onDestroy(() => {
    unsubStatus()
    unsubNotif()
  })

  async function togglePanel() {
    open = !open
    if (open) {
      // Fetch the list lazily, only when the panel actually opens.
      await notificationStore.loadNotifications(describeSyncError)
    }
  }

  function closePanel() {
    open = false
  }

  function handleWindowClick(event: MouseEvent) {
    if (!open) return
    if (rootEl && event.target instanceof Node && !rootEl.contains(event.target)) {
      open = false
    }
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape' && open) {
      open = false
    }
  }

  async function markRead(item: NotificationItem) {
    if (item.read_at !== null) return
    try {
      await notificationStore.markRead(item.id)
    } catch (error) {
      console.warn('[NotificationBell] mark read failed:', error)
    }
  }

  async function deleteNotification(item: NotificationItem) {
    try {
      await notificationStore.deleteNotification(item.id)
    } catch (error) {
      console.warn('[NotificationBell] delete notification failed:', error)
    }
  }

  async function markAllRead() {
    try {
      await notificationStore.markAllRead()
    } catch (error) {
      console.warn('[NotificationBell] mark all read failed:', error)
    }
  }
</script>

<svelte:window onclick={handleWindowClick} onkeydown={handleKeydown} />

{#if hasSession}
  <!-- Leading separator owned by the bell so a `disabled` state renders no orphan `·`. -->
  <span class="notif__sep" aria-hidden="true">·</span>
  <div class="notif" bind:this={rootEl}>
    <button
      type="button"
      class="notif__bell"
      class:notif__bell--active={open}
      onclick={togglePanel}
      aria-label={bellLabel}
      aria-haspopup="true"
      aria-expanded={open}
      title={bellLabel}
    >
      <ActionIcon name="bell" size={16} />
      {#if hasUnread}
        <span class="notif__badge" data-testid="notif-badge">
          {unread > 99 ? '99+' : unread}
        </span>
      {/if}
    </button>

    {#if open}
      <div class="notif__panel" role="dialog" aria-label={t('sync.notif.panelTitle')}>
        <div class="notif__panel-head">
          <span class="notif__panel-title">{t('sync.notif.panelTitle')}</span>
          <div class="notif__panel-actions">
            {#if hasUnread}
              <button type="button" class="notif__text-btn" onclick={markAllRead}>
                {t('sync.notif.markAllRead')}
              </button>
            {/if}
            <IconButton
              size="xs"
              label={t('sync.notif.close')}
              title={t('sync.notif.close')}
              onclick={closePanel}
            >
              <ActionIcon name="close" size={12} />
            </IconButton>
          </div>
        </div>

        <div class="notif__panel-body">
          {#if notif.loading}
            <p class="notif__hint">{t('sync.notif.loading')}</p>
          {:else if notif.error}
            <p class="notif__hint notif__hint--error" role="alert">{notif.error}</p>
          {:else if notif.items.length === 0}
            <p class="notif__hint">{t('sync.notif.empty')}</p>
          {:else}
            <ul class="notif__list">
              {#each notif.items as item (item.id)}
                <li
                  class="notif__item notif__item--{severityVariant(item.severity)}"
                  class:notif__item--unread={item.read_at === null}
                >
                  <div class="notif__item-main">
                    <!-- Server-rendered content: NEVER translated client-side. -->
                    <p class="notif__item-title">{item.title}</p>
                    {#if item.body}
                      <p class="notif__item-body">{item.body}</p>
                    {/if}
                    <span class="notif__item-time">
                      {formatRelativeTime(item.created_at, $currentLocale)}
                    </span>
                  </div>
                  <div class="notif__item-actions">
                    {#if item.read_at === null}
                      <IconButton
                        size="xs"
                        class="notif__icon-btn--read"
                        label={t('sync.notif.markRead')}
                        title={t('sync.notif.markRead')}
                        onclick={() => markRead(item)}
                      >
                        <ActionIcon name="check" size={12} />
                      </IconButton>
                    {/if}
                    <IconButton
                      size="xs"
                      class="notif__icon-btn--delete"
                      label={t('sync.notif.delete')}
                      title={t('sync.notif.delete')}
                      onclick={() => deleteNotification(item)}
                    >
                      <ActionIcon name="delete" size={12} />
                    </IconButton>
                  </div>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      </div>
    {/if}
  </div>
{/if}

<style>
  .notif__sep {
    opacity: 0.4;
  }

  .notif {
    position: relative;
    display: inline-flex;
    align-items: center;
  }

  .notif__bell {
    position: relative;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--color-text-muted);
    cursor: pointer;
    transition:
      color var(--transition-base),
      background-color var(--transition-base);
  }

  .notif__bell:hover,
  .notif__bell--active {
    color: var(--color-accent);
    background: var(--color-accent-faint);
  }

  .notif__bell:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .notif__badge {
    position: absolute;
    top: -3px;
    right: -3px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 14px;
    height: 14px;
    padding: 0 3px;
    border-radius: 999px;
    background: var(--color-danger);
    color: #fff;
    font-family: var(--font-sans);
    font-size: 9px;
    font-weight: var(--font-weight-semibold);
    line-height: 1;
  }

  .notif__panel {
    position: absolute;
    bottom: calc(100% + 8px);
    right: 0;
    z-index: 60;
    display: flex;
    flex-direction: column;
    width: 340px;
    max-height: 420px;
    border: 1px solid var(--border-panel);
    border-radius: var(--radius-md);
    background: var(--surface-panel);
    box-shadow: var(--shadow-lg, 0 12px 32px rgba(0, 0, 0, 0.32));
    overflow: hidden;
  }

  .notif__panel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-bottom: 1px solid var(--border-subtle);
    background: color-mix(in srgb, var(--surface-toolbar) 78%, transparent);
  }

  .notif__panel-title {
    font-family: var(--font-sans);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
    color: var(--color-text-primary);
    letter-spacing: 0;
  }

  .notif__panel-actions {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .notif__text-btn {
    padding: 2px var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--color-accent);
    font-family: var(--font-sans);
    font-size: var(--font-size-xs);
    cursor: pointer;
  }

  .notif__text-btn:hover {
    background: var(--color-accent-faint);
  }

  /* Layout, size and hover chrome come from IconButton size="xs" (24px).
     Only the per-action hover colour stays local — it is semantics, not shape. */
  :global(.notif__icon-btn--read):hover:not(:disabled) {
    color: var(--color-success);
  }

  :global(.notif__icon-btn--delete):hover:not(:disabled) {
    color: var(--color-danger);
  }

  .notif__panel-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .notif__hint {
    margin: 0;
    padding: var(--space-4) var(--space-3);
    font-family: var(--font-sans);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    text-align: center;
  }

  .notif__hint--error {
    color: var(--color-danger);
  }

  .notif__list {
    display: flex;
    flex-direction: column;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .notif__item {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-3);
    border-bottom: 1px solid var(--border-subtle);
    border-left: 3px solid transparent;
  }

  .notif__item--info {
    border-left-color: color-mix(in srgb, var(--color-accent) 60%, transparent);
  }

  .notif__item--warning {
    border-left-color: var(--color-warning, #d99a00);
  }

  .notif__item--danger {
    border-left-color: var(--color-danger);
  }

  .notif__item--unread {
    background: color-mix(in srgb, var(--color-accent) 6%, transparent);
  }

  .notif__item-main {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .notif__item-actions {
    display: inline-flex;
    flex-shrink: 0;
    align-items: center;
    gap: var(--space-1);
  }

  .notif__item-title {
    margin: 0;
    font-family: var(--font-sans);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
    color: var(--color-text-primary);
  }

  .notif__item-body {
    margin: 0;
    font-family: var(--font-sans);
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    line-height: 1.5;
    word-break: break-word;
  }

  .notif__item-time {
    font-family: var(--font-sans);
    font-size: var(--font-size-2xs, 0.65rem);
    color: var(--color-text-muted);
  }
</style>
