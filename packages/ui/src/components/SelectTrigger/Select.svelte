<script lang="ts">
  /**
   * A choice among options: SelectTrigger opening a ToolbarMenu of radio
   * items. Choosing closes the menu and puts the focus back on the trigger.
   */
  import ToolbarMenu from '../ToolbarMenu/ToolbarMenu.svelte'
  import type { ToolbarMenuItem } from '../ToolbarMenu/ToolbarMenu.types'
  import SelectTrigger from './SelectTrigger.svelte'
  import type { SelectProps } from './SelectTrigger.types'

  let {
    label,
    value = $bindable(''),
    options,
    align = 'start',
    onchange,
    ...triggerProps
  }: SelectProps = $props()

  let open = $state(false)
  let triggerEl: HTMLButtonElement | undefined = $state()

  const current = $derived(options.find((option) => option.value === value))

  function choose(next: string) {
    open = false
    triggerEl?.focus()
    if (next === value) return
    value = next
    onchange?.(next)
  }

  const items = $derived<ToolbarMenuItem[]>(
    options.map((option) => ({
      kind: 'radio' as const,
      id: option.value,
      label: option.label,
      checked: option.value === value,
      disabled: option.disabled,
      onselect: () => choose(option.value),
    }))
  )
</script>

<ToolbarMenu {label} {items} {align} bind:open>
  {#snippet trigger(props, { open: isOpen })}
    <SelectTrigger
      {...triggerProps}
      {...props}
      label={current?.label}
      open={isOpen}
      bind:element={triggerEl}
    />
  {/snippet}
</ToolbarMenu>
