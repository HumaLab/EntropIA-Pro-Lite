<script lang="ts">
  import { renderOcrHtml } from '$lib/ocr-rich-text'
  import type { OcrRenderContext, OcrSourceType } from '$lib/ocr-rich-text'

  interface OcrRichTextProps {
    text: string
    assetUrl: string
    sourceType: OcrSourceType
    referenceWidth: number
    referenceHeight: number
  }

  let {
    text,
    assetUrl,
    sourceType,
    referenceWidth,
    referenceHeight,
  }: OcrRichTextProps = $props()

  let html = $state('')
  let renderGeneration = 0

  $effect(() => {
    const generation = ++renderGeneration
    const context: OcrRenderContext = {
      assetUrl,
      sourceType,
      referenceWidth,
      referenceHeight,
    }
    html = ''

    void renderOcrHtml(text, context)
      .then((nextHtml) => {
        if (generation === renderGeneration) html = nextHtml
      })
      .catch(() => {
        if (generation === renderGeneration) html = ''
      })
  })
</script>

<div class="ocr-rich-text" data-testid="ocr-rich-text">
  <!-- eslint-disable-next-line svelte/no-at-html-tags -- renderOcrHtml sanitizes the generated HTML -->
  {@html html}
</div>

<style>
  .ocr-rich-text {
    min-width: 0;
    overflow-wrap: anywhere;
    line-height: 1.6;
  }

  .ocr-rich-text :global(h1),
  .ocr-rich-text :global(h2),
  .ocr-rich-text :global(h3),
  .ocr-rich-text :global(h4),
  .ocr-rich-text :global(h5),
  .ocr-rich-text :global(h6) {
    margin: var(--space-4) 0 var(--space-2);
    color: var(--color-text-primary);
    line-height: 1.25;
  }

  .ocr-rich-text :global(p),
  .ocr-rich-text :global(ul),
  .ocr-rich-text :global(ol),
  .ocr-rich-text :global(blockquote),
  .ocr-rich-text :global(table),
  .ocr-rich-text :global(pre) {
    margin: 0 0 var(--space-3);
  }

  .ocr-rich-text :global(ul),
  .ocr-rich-text :global(ol) {
    padding-inline-start: var(--space-6);
  }

  .ocr-rich-text :global(blockquote) {
    padding-inline-start: var(--space-3);
    border-inline-start: 2px solid var(--border-subtle);
    color: var(--color-text-secondary);
  }

  .ocr-rich-text :global(table) {
    width: 100%;
    border-collapse: collapse;
    font-size: inherit;
  }

  .ocr-rich-text :global(th),
  .ocr-rich-text :global(td) {
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    text-align: start;
    vertical-align: top;
  }

  .ocr-rich-text :global(code),
  .ocr-rich-text :global(pre) {
    font-family: var(--font-family-mono, monospace);
  }

  .ocr-rich-text :global(pre) {
    padding: var(--space-3);
    overflow-x: auto;
    border-radius: var(--radius-xs);
    background: var(--surface-panel);
  }

  .ocr-rich-text :global(img) {
    display: inline-block;
    max-width: 100%;
    height: auto;
    vertical-align: middle;
    border-radius: var(--radius-xs);
  }

  .ocr-rich-text :global(.ocr-region-fallback) {
    display: inline-block;
    width: 0.7em;
    height: 0.7em;
    margin-inline: 0.15em;
    border: 1px solid var(--color-text-muted);
    border-radius: 50%;
    opacity: 0.55;
    vertical-align: middle;
  }

  .ocr-rich-text :global(a) {
    color: var(--color-accent);
  }
</style>
