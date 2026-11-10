<!--
  Says that something derived from an image (its layout, its OCR text) was
  computed on another version of that image, and offers to compute it again.
  What it is about is kept either way; running it again replaces it.
-->
<script lang="ts">
  import { Button, tooltip } from '@entropia/ui'

  let {
    heading,
    message,
    actionLabel,
    busy = false,
    busyTitle,
    testId,
    onRerun,
  }: {
    /** Title of the card, for a notice that stands in for a whole panel. */
    heading?: string
    message: string
    actionLabel: string
    /** An OCR of this image is already running. */
    busy?: boolean
    busyTitle?: string
    testId: string
    onRerun: () => void
  } = $props()
</script>

<section class="section" data-testid={testId}>
  {#if heading}
    <h3>{heading}</h3>
  {/if}
  <!-- One line whatever the width: the message gives way and the action never
       wraps or shrinks. The full message stays in the tooltip (the app's own, not a native title). -->
  <div class="notice">
    <span class="notice__message" role="status" use:tooltip={message}>{message}</span>
    <Button
      variant="secondary"
      size="sm"
      type="button"
      disabled={busy}
      title={busy ? busyTitle : undefined}
      onclick={() => onRerun()}
    >
      {actionLabel}
    </Button>
  </div>
</section>

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: var(--color-surface);
    box-shadow: var(--shadow-surface);
  }

  .section h3 {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
  }

  .notice {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    padding: var(--space-1) var(--space-2);
    border: 1px solid color-mix(in srgb, var(--color-warning) 35%, transparent);
    border-radius: var(--radius-sm);
    background: color-mix(in srgb, var(--color-warning) 12%, var(--surface-card));
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    line-height: 1.4;
  }

  .notice__message {
    flex: 1 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .notice :global(button) {
    flex: 0 0 auto;
    white-space: nowrap;
  }
</style>
