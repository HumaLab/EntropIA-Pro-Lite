<script lang="ts">
  /**
   * Fine clockwise rotation — mirror of RotateFineCcw, same metric.
   */
  let { size = 16 }: { size?: number } = $props()
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  data-action-icon="rotate-fine-cw"
>
  <circle cx="12" cy="12" r="8" stroke-dasharray="1 3.2" />
  <circle cx="12" cy="12" r="1.6" fill="currentColor" stroke="none" />
  <path d="M12 12 15.5 8.5" />
  <path d="M15.6 4.9 18.7 8l-3.1 3.1" />
</svg>
