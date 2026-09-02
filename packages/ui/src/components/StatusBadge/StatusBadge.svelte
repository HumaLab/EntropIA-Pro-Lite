<script lang="ts">
  import type { StatusBadgeProps } from './StatusBadge.types'

  let {
    variant = 'neutral',
    size = 'sm',
    children,
    class: className = '',
    ...rest
  }: StatusBadgeProps = $props()
</script>

<span class="status-badge status-badge--{variant} status-badge--{size} {className}" {...rest}>
  {#if children}
    {@render children()}
  {/if}
</span>

<style>
  .status-badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    width: fit-content;
    max-width: 100%;
    border: 1px solid transparent;
    border-radius: var(--radius-control);
    font-family: var(--font-sans);
    font-weight: var(--font-weight-medium);
    line-height: var(--line-height-tight);
    white-space: nowrap;
  }

  .status-badge--sm {
    min-height: 22px;
    padding: 0 var(--space-2);
    font-size: var(--font-size-xs);
  }

  .status-badge--md {
    min-height: 28px;
    padding: 0 var(--space-3);
    font-size: var(--font-size-sm);
  }

  .status-badge--neutral {
    background: var(--surface-card);
    border-color: var(--border-subtle);
    color: var(--color-text-secondary);
  }

  /* Reposo con resultado guardado: el mismo verde de la app, muy bajado, para
     que se distinga del gris sin llegar a leerse como un estado activo. */
  .status-badge--stored {
    background: color-mix(in srgb, var(--color-success) 9%, var(--surface-card));
    border-color: color-mix(in srgb, var(--color-success) 20%, transparent);
    color: color-mix(in srgb, var(--color-success) 72%, var(--color-text-secondary));
  }

  .status-badge--success {
    background: var(--color-success-soft);
    border-color: color-mix(in srgb, var(--color-success) 28%, transparent);
    color: var(--color-success);
  }

  .status-badge--warning {
    background: var(--color-warning-soft);
    border-color: color-mix(in srgb, var(--color-warning) 28%, transparent);
    color: var(--color-warning);
  }

  .status-badge--danger {
    background: var(--color-danger-soft);
    border-color: color-mix(in srgb, var(--color-danger) 28%, transparent);
    color: var(--color-danger);
  }

  .status-badge--info {
    background: var(--color-info-soft);
    border-color: color-mix(in srgb, var(--color-info) 28%, transparent);
    color: var(--color-info);
  }

  .status-badge--ai {
    background: var(--state-ai-soft);
    border-color: var(--state-ai-border);
    color: var(--state-ai);
  }

  .status-badge--evidence {
    background: var(--state-evidence-soft);
    border-color: var(--state-evidence-border);
    color: var(--state-evidence);
  }
</style>
