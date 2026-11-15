<script lang="ts">
  import IconButton from '../IconButton.svelte'
  import type { IconButtonHoverTone, IconButtonSize, IconButtonVariant } from '../IconButton.types'

  let {
    size = 'md',
    variant = 'ghost',
    label,
    active = false,
    disabled = false,
    hoverTone,
  }: {
    size?: IconButtonSize
    variant?: IconButtonVariant
    label: string
    active?: boolean
    disabled?: boolean
    hoverTone?: IconButtonHoverTone
  } = $props()
</script>

<IconButton {size} {variant} {label} {active} {disabled} {hoverTone}>
  <svg aria-hidden="true" viewBox="0 0 24 24">
    <path d="M0 0h24v24H0z" />
  </svg>
</IconButton>
