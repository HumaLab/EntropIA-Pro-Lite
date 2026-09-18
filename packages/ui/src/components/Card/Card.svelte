<script lang="ts">
  import type { CardProps } from './Card.types'

  let { padding = 'md', hoverable = false, header, children, footer }: CardProps = $props()
</script>

<div class="card card--padding-{padding}" class:card--hoverable={hoverable}>
  {#if header}
    <div class="card__header">
      {@render header()}
    </div>
  {/if}

  {#if children}
    <div class="card__body">
      {@render children()}
    </div>
  {/if}

  {#if footer}
    <div class="card__footer">
      {@render footer()}
    </div>
  {/if}
</div>

<style>
  .card {
    display: flex;
    flex-direction: column;
    background: var(--color-surface-elevated);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    box-shadow: var(--shadow-surface);
    overflow: hidden;
    transition:
      box-shadow var(--transition-base),
      border-color var(--transition-base),
      background-color var(--transition-base);
  }

  .card--hoverable:hover {
    box-shadow: var(--shadow-surface);
    border-color: var(--color-border-hover);
  }

  /* ─── Padding ─── */
  .card--padding-sm .card__header,
  .card--padding-sm .card__body,
  .card--padding-sm .card__footer {
    padding: var(--space-3);
  }

  .card--padding-md .card__header,
  .card--padding-md .card__body,
  .card--padding-md .card__footer {
    padding: var(--space-4);
  }

  .card--padding-lg .card__header,
  .card--padding-lg .card__body,
  .card--padding-lg .card__footer {
    padding: var(--space-6);
  }

  /* ─── Sections ─── */
  .card__header {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    background-color: color-mix(in srgb, var(--color-surface) 82%, transparent);
    border-bottom: 1px solid var(--color-hairline);
    font-family: var(--font-ui);
    font-weight: var(--font-weight-medium);
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
  }

  .card__body {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    font-family: var(--font-ui);
    color: var(--color-text-primary);
  }

  .card__footer {
    background-color: color-mix(in srgb, var(--color-surface) 82%, transparent);
    border-top: 1px solid var(--color-hairline);
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }
</style>
