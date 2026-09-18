<script lang="ts">
  import ToolbarMenu from '../ToolbarMenu.svelte'
  import ToolbarMenuList from '../ToolbarMenuList.svelte'
  import type { ToolbarMenuCloseReason, ToolbarMenuItem } from '../ToolbarMenu.types'

  let {
    items = [],
    swatches = [],
    listed = [],
    onswatch,
    onclose,
    open = $bindable(false),
  }: {
    items?: ToolbarMenuItem[]
    /** Free content: a row of radio choices, as a colour grid would be. */
    swatches?: string[]
    /** Items drawn inside the content, after the swatches. */
    listed?: ToolbarMenuItem[]
    onswatch?: (swatch: string) => void
    onclose?: (reason: ToolbarMenuCloseReason) => void
    open?: boolean
  } = $props()
</script>

<button type="button">Before</button>

<ToolbarMenu label="More tools" {items} {onclose} bind:open>
  {#snippet trigger(props)}
    <button type="button" aria-label="Open menu" {...props}>…</button>
  {/snippet}
  {#snippet children(menu)}
    {#if swatches.length > 0}
      <div role="group" aria-label="Colours">
        {#each swatches as swatch (swatch)}
          <button
            type="button"
            role="menuitemradio"
            aria-checked="false"
            tabindex="-1"
            onclick={() => {
              onswatch?.(swatch)
              menu.close({ returnFocus: false })
            }}>{swatch}</button
          >
        {/each}
      </div>
    {/if}
    {#if listed.length > 0}
      <ToolbarMenuList items={listed} onselect={menu.select} />
    {/if}
  {/snippet}
</ToolbarMenu>

<button type="button">After</button>
