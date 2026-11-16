<script lang="ts">
  import { ActionIcon } from '../Button'
  import IconButton from '../IconButton/IconButton.svelte'
  import StatusBadge from '../StatusBadge/StatusBadge.svelte'
  import type { CollectionCardProps } from './CollectionCard.types'
  import { formatRelativeDate } from './formatRelativeDate'

  let {
    id: _id,
    name,
    description,
    itemCount,
    countLabel,
    updatedAt,
    locale = 'en',
    onclick,
    onedit,
    ondelete,
    editAriaLabel = 'Edit collection',
    deleteAriaLabel = 'Delete collection',
  }: CollectionCardProps = $props()

  const chipLabel = $derived(
    countLabel ?? `${itemCount} ${itemCount === 1 ? 'document' : 'documents'}`
  )
  const relativeDate = $derived(formatRelativeDate(updatedAt, locale))
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
  <h3 class="collection-card__name">{name}</h3>

  <p class="collection-card__description" data-testid="collection-description">
    {visibleDescription}
  </p>

  <div class="collection-card__footer">
    <span class="collection-card__date" data-testid="collection-date">
      {relativeDate}
    </span>

    <div class="collection-card__controls">
      <StatusBadge variant="neutral" size="sm">{chipLabel}</StatusBadge>
      {#if onedit || ondelete}
        <div class="collection-card__actions">
          {#if onedit}
            <IconButton
              variant="ghost"
              size="md"
              label={editAriaLabel}
              data-testid="edit-button"
              onclick={(e: MouseEvent) => {
                e.stopPropagation()
                onedit()
              }}
            >
              <ActionIcon name="edit" size={16} />
            </IconButton>
          {/if}
          {#if ondelete}
            <IconButton
              variant="ghost"
              size="md"
              hoverTone="danger"
              label={deleteAriaLabel}
              data-testid="delete-button"
              onclick={(e: MouseEvent) => {
                e.stopPropagation()
                ondelete()
              }}
            >
              <ActionIcon name="delete" size={16} />
            </IconButton>
          {/if}
        </div>
      {/if}
    </div>
  </div>
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
    font-family: var(--font-ui);
    color: var(--color-text-primary);
  }

  .collection-card:hover {
    border-color: color-mix(in srgb, var(--color-accent) 28%, var(--color-border-strong));
    box-shadow: var(--shadow-surface);
  }

  .collection-card:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .collection-card__name {
    margin: 0;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
    color: var(--color-text-primary);
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  /* Pinned to the bottom: the grid stretches every card in a row to the tallest
     one (an inline edit form makes that dramatic), so without margin-top:auto
     the controls would float mid-card on the stretched siblings. */
  .collection-card__footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    width: 100%;
    min-width: 0;
    margin-top: auto;
  }

  .collection-card__controls {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex: 0 0 auto;
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

  /* Yields first: a long relative date shrinks instead of pushing the controls
     out of the card. */
  .collection-card__date {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }
</style>
