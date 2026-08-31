<script lang="ts">
  import IconButton from '../IconButton.svelte'
  import type { IconButtonSize, IconButtonVariant } from '../IconButton.types'

  let {
    size = 'md',
    variant = 'ghost',
    label,
    active = false,
    disabled = false,
  }: {
    size?: IconButtonSize
    variant?: IconButtonVariant
    label: string
    active?: boolean
    disabled?: boolean
  } = $props()
</script>

<IconButton {size} {variant} {label} {active} {disabled}>
  <svg aria-hidden="true" viewBox="0 0 24 24">
    <path d="M0 0h24v24H0z" />
  </svg>
</IconButton>
