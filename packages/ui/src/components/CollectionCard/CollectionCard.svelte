<script lang="ts">
  import { ActionIcon, Button } from '../Button'
  import type { CollectionCardProps } from './CollectionCard.types'

  let {
    id: _id,
    name,
    description,
    itemCount,
    updatedAt,
    locale = 'en',
    onclick,
    onedit,
    ondelete,
    editAriaLabel = 'Edit collection',
    deleteAriaLabel = 'Delete collection',
  }: CollectionCardProps = $props()

  function formatRelativeDate(timestamp: number): string {
    const now = Date.now()
    const diff = now - timestamp
    const seconds = Math.floor(diff / 1000)
    const minutes = Math.floor(seconds / 60)
    const hours = Math.floor(minutes / 60)
    const days = Math.floor(hours / 24)
    const formatter = new Intl.RelativeTimeFormat(locale, { numeric: 'always' })

    if (days > 0) return formatter.format(-days, 'day')
    if (hours > 0) return formatter.format(-hours, 'hour')
    if (minutes > 0) return formatter.format(-minutes, 'minute')
    return new Intl.RelativeTimeFormat(locale, { numeric: 'auto' }).format(0, 'second')
  }

  const itemLabel = $derived(itemCount === 1 ? 'item' : 'items')
  const relativeDate = $derived(formatRelativeDate(updatedAt))
  const visibleDescription = $derived(description?.trim() || name)
</script>

<div
  class="collection-card"
  role="button"
  tabindex="0"
  {onclick}
  onkeydown={(e: KeyboardEvent) => {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      onclick?.()
    }
  }}
>
  <div class="collection-card__header">
    <h3 class="collection-card__name">{name}</h3>
    <span class="collection-card__badge">{itemCount} {itemLabel}</span>
    {#if onedit || ondelete}
      <div class="collection-card__actions">
        {#if onedit}
          <Button
            class="collection-card__edit-action"
            variant="ghost"
            size="sm"
            iconOnly
            aria-label={editAriaLabel}
            data-testid="edit-button"
            onclick={(e: MouseEvent) => {
              e.stopPropagation()
              onedit()
            }}
          >
            <ActionIcon name="edit" />
          </Button>
        {/if}
        {#if ondelete}
          <Button
            class="collection-card__delete-action"
            variant="ghost"
            size="sm"
            iconOnly
            aria-label={deleteAriaLabel}
            data-testid="delete-button"
            onclick={(e: MouseEvent) => {
              e.stopPropagation()
              ondelete()
            }}
          >
            <ActionIcon name="delete" />
          </Button>
        {/if}
      </div>
    {/if}
  </div>

  <p class="collection-card__description" data-testid="collection-description">
    {visibleDescription}
  </p>

  <span class="collection-card__date" data-testid="collection-date">
    {relativeDate}
  </span>
</div>

<style>
  .collection-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4);
    background: var(--color-surface);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    cursor: pointer;
    transition:
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth);
    text-align: left;
    width: 100%;
    font-family: var(--font-sans);
    color: var(--color-text-primary);
  }

  .collection-card:hover {
    border-color: color-mix(in srgb, var(--color-accent) 28%, var(--color-border-strong));
    box-shadow: var(--shadow-surface);
  }

  .collection-card:focus-visible {
    outline: 2px solid var(--color-accent);
    outline-offset: 2px;
  }

  .collection-card__header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    width: 100%;
    min-width: 0;
  }

  .collection-card__name {
    margin: 0;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-bold);
    color: var(--color-text-primary);
    min-width: 0;
    flex: 1 1 auto;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .collection-card__badge {
    flex: 0 0 auto;
    flex-shrink: 0;
    white-space: nowrap;
    padding: var(--space-1) var(--space-2);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    color: var(--color-accent-hover);
    background-color: var(--color-accent-faint);
    border: 1px solid color-mix(in srgb, var(--color-accent) 22%, transparent);
    border-radius: var(--radius-control);
  }

  .collection-card__description {
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--color-text-muted);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .collection-card__actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex: 0 0 auto;
    flex-shrink: 0;
  }

  :global(.collection-card__edit-action) {
    background-color: transparent;
    border-color: transparent;
    box-shadow: none;
    color: var(--color-text-primary);
    opacity: 0.88;
  }

  :global(.collection-card__delete-action) {
    background-color: transparent;
    border-color: transparent;
    color: var(--color-danger);
    box-shadow: none;
    opacity: 0.88;
  }

  :global(.collection-card__edit-action:hover:not(:disabled)),
  :global(.collection-card__edit-action:focus-visible),
  :global(.collection-card__edit-action:active),
  :global(.collection-card__delete-action:hover:not(:disabled)),
  :global(.collection-card__delete-action:focus-visible),
  :global(.collection-card__delete-action:active) {
    background-color: transparent;
    border-color: transparent;
    box-shadow: none;
    opacity: 1;
    transform: none;
  }

  :global(.collection-card__edit-action:hover:not(:disabled)),
  :global(.collection-card__edit-action:focus-visible),
  :global(.collection-card__edit-action:active) {
    color: var(--color-text-primary);
  }

  :global(.collection-card__delete-action:hover:not(:disabled)),
  :global(.collection-card__delete-action:focus-visible),
  :global(.collection-card__delete-action:active) {
    color: var(--color-danger);
  }

  .collection-card__date {
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }
</style>
