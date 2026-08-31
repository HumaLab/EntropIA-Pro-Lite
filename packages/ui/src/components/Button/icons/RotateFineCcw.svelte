<script lang="ts">
  /**
   * Fine counter-clockwise rotation — no Lucide equivalent.
   *
   * Authored on the same metric as the rest of the catalogue: 24x24 grid,
   * stroke 2, round caps and joins, currentColor. The dotted ring carries the
   * "small increment" meaning that the coarse `rotate-ccw` arrow lacks; the
   * pivot dot and needle read as the current angle.
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
  data-action-icon="rotate-fine-ccw"
>
  <circle cx="12" cy="12" r="8" stroke-dasharray="1 3.2" />
  <circle cx="12" cy="12" r="1.6" fill="currentColor" stroke="none" />
  <path d="M12 12 8.5 8.5" />
  <path d="M8.4 4.9 5.3 8l3.1 3.1" />
</svg>
