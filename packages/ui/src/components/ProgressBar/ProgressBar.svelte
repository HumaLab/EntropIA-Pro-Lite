<script lang="ts">
  import type { ProgressBarProps } from './ProgressBar.types'

  let {
    value = 0,
    max = 100,
    indeterminate = false,
    label,
    valueText,
    tone = 'accent',
    size = 'md',
    class: className = '',
    ...rest
  }: ProgressBarProps = $props()

  const safeMax = $derived(Number.isFinite(max) && max > 0 ? max : 100)
  const current = $derived(Number.isFinite(value) ? Math.min(Math.max(value, 0), safeMax) : 0)
  const percent = $derived((current / safeMax) * 100)
</script>

<div
  class="progress-bar progress-bar--{tone} progress-bar--{size} {className}"
  class:progress-bar--indeterminate={indeterminate}
  role="progressbar"
  aria-label={label}
  aria-valuemin={0}
  aria-valuemax={safeMax}
  aria-valuenow={indeterminate ? undefined : current}
  aria-valuetext={indeterminate ? undefined : valueText}
  {...rest}
>
  <span class="progress-bar__fill" style:width={indeterminate ? undefined : `${percent}%`}></span>
</div>

<style>
  .progress-bar {
    --progress-bar-height: 8px;
    position: relative;
    width: 100%;
    height: var(--progress-bar-height);
    overflow: hidden;
    border-radius: var(--radius-full);
    background: var(--color-surface-sunken);
  }

  .progress-bar--sm {
    --progress-bar-height: 4px;
  }

  .progress-bar--md {
    --progress-bar-height: 8px;
  }

  .progress-bar--accent {
    --progress-bar-fill: var(--color-accent);
  }

  .progress-bar--success {
    --progress-bar-fill: var(--color-success);
  }

  .progress-bar--warning {
    --progress-bar-fill: var(--color-warning);
  }

  .progress-bar--danger {
    --progress-bar-fill: var(--color-danger);
  }

  .progress-bar__fill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: inherit;
    background: var(--progress-bar-fill);
    transition: width var(--transition-smooth);
  }

  .progress-bar--indeterminate .progress-bar__fill {
    width: 40%;
    animation: progress-bar-slide calc(var(--duration-slow) * 5) ease-in-out infinite;
  }

  @keyframes progress-bar-slide {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(250%);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .progress-bar__fill {
      transition: none;
    }

    .progress-bar--indeterminate .progress-bar__fill {
      animation: none;
      width: 100%;
      opacity: var(--opacity-disabled);
    }
  }
</style>
