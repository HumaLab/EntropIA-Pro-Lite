<script lang="ts">
  let {
    text,
    onrendered,
  }: {
    text: string
    onrendered?: (container: HTMLDivElement) => void
  } = $props()
  let container = $state<HTMLDivElement | undefined>(undefined)

  $effect(() => {
    const el = container
    const current = text
    if (!el) return
    queueMicrotask(() => {
      if (container === el && current === text) onrendered?.(el)
    })
  })
</script>

<div data-testid="mock-ocr-rich-text" bind:this={container}>{text}</div>
