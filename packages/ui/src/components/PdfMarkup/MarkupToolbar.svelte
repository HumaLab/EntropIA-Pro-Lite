<script lang="ts">
  import Button from '../Button/Button.svelte'
  import ActionIcon from '../Button/ActionIcon.svelte'
  import type { ActionIconName } from '../Button/ActionIcon.types'
  import type { MarkupTool, PdfMarkupLabels } from './types'
  import { MARKUP_TOOLS } from './types'

  interface Props {
    tool: MarkupTool
    color: string
    allowedTools: readonly MarkupTool[]
    palette: readonly string[]
    labels: PdfMarkupLabels
    onToolChange?: (tool: MarkupTool) => void
    onColorChange?: (color: string) => void
  }

  let {
    tool,
    color,
    allowedTools,
    palette,
    labels,
    onToolChange = () => {},
    onColorChange = () => {},
  }: Props = $props()

  const ICONS: Record<MarkupTool, ActionIconName> = {
    select: 'pointer',
    highlight: 'highlight',
    underline: 'underline',
    note: 'message-circle',
    image: 'rectangle',
    text: 'typography',
  }

  function label(name: MarkupTool) {
    switch (name) {
      case 'select':
        return labels.toolSelect
      case 'highlight':
        return labels.toolHighlight
      case 'underline':
        return labels.toolUnderline
      case 'note':
        return labels.toolNote
      case 'image':
        return labels.toolImage
      case 'text':
        return labels.toolText
    }
  }

  const tools = $derived(MARKUP_TOOLS.filter((name) => allowedTools.includes(name)))
</script>

<div class="markup-toolbar" role="toolbar" aria-label={labels.toolbarAriaLabel} data-testid="markup-toolbar">
  {#each tools as name (name)}
    <Button
      variant={tool === name ? 'subtle' : 'ghost'}
      size="sm"
      iconOnly
      aria-label={label(name)}
      title={label(name)}
      aria-pressed={tool === name}
      data-testid={`markup-tool-${name}`}
      onclick={() => onToolChange(name)}
    >
      <ActionIcon name={ICONS[name]} size={20} />
    </Button>
  {/each}
  <span class="markup-toolbar__divider" aria-hidden="true"></span>
  {#each palette as swatch (swatch)}
    <button
      type="button"
      class="markup-toolbar__swatch"
      class:markup-toolbar__swatch--active={swatch === color}
      style={`background:${swatch};`}
      aria-label={labels.colorAriaLabel(swatch)}
      aria-pressed={swatch === color}
      onclick={() => onColorChange(swatch)}
    ></button>
  {/each}
</div>

<style>
  .markup-toolbar {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1);
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    background: var(--color-surface);
    /* The viewer's toolbar anchor lets pointer events through its empty
       space; the toolbar takes them back, as AnnotationToolbar does. */
    pointer-events: auto;
  }

  .markup-toolbar__divider {
    width: 1px;
    align-self: stretch;
    margin: 0 var(--space-1);
    background: var(--color-border);
  }

  .markup-toolbar__swatch {
    width: 18px;
    height: 18px;
    padding: 0;
    border: 1px solid color-mix(in srgb, var(--color-text-primary) 20%, transparent);
    border-radius: 50%;
    cursor: pointer;
  }

  .markup-toolbar__swatch--active {
    outline: 2px solid var(--color-accent);
    outline-offset: 1px;
  }
</style>
