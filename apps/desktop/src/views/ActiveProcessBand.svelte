<script lang="ts">
  /**
   * A small, temporary status band for one long-running process (OCR,
   * embeddings, an import, or a sync run). Purely presentational: every
   * string is already composed by the caller, so this component carries no
   * i18n, formatting, or data-source knowledge of its own — it just lays the
   * three pieces out and renders nothing when the caller renders nothing
   * (odd/tasks/home-view.md T3d).
   */
  export interface ActiveProcessBandProps {
    /** e.g. "OCR" or "OCR · Movimiento Obrero Mar del Plata". */
    title: string
    /** e.g. "428 / 1.244 páginas · 34 %". */
    progress: string
    /** e.g. "Ver lote →". */
    openLabel: string
    onOpen: () => void
  }

  let { title, progress, openLabel, onOpen }: ActiveProcessBandProps = $props()
</script>

<div class="active-process-band" role="status">
  <span class="active-process-band__title">{title}</span>
  <span class="active-process-band__progress">{progress}</span>
  <button type="button" class="active-process-band__open" onclick={onOpen}>{openLabel}</button>
</div>

<style>
  .active-process-band {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-4);
    background: var(--color-surface-raised);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    font-size: var(--font-size-xs);
  }

  .active-process-band__title {
    flex-shrink: 0;
    font-weight: var(--font-weight-medium);
    color: var(--color-text-primary);
  }

  .active-process-band__progress {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--color-text-muted);
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .active-process-band__open {
    flex-shrink: 0;
    padding: 0;
    background: none;
    border: none;
    font: inherit;
    color: var(--color-text-secondary);
    cursor: pointer;
  }

  .active-process-band__open:hover,
  .active-process-band__open:focus-visible {
    color: var(--color-text-primary);
  }
</style>
