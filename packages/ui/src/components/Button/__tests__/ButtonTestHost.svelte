<script lang="ts">
  import Button from '../Button.svelte'
  import type { ButtonVariant } from '../Button.types'

  let {
    iconOnly = false,
    wrap = false,
    size = 'md',
    variant,
    disabled,
    onclick,
    label,
    class: className,
  }: {
    iconOnly?: boolean
    wrap?: boolean
    size?: 'sm' | 'md' | 'lg'
    variant?: ButtonVariant
    disabled?: boolean
    onclick?: () => void
    label: string
    class?: string
  } = $props()

  const buttonProps = $derived({
    iconOnly,
    wrap,
    size,
    'aria-label': label,
    ...(variant === undefined ? {} : { variant }),
    ...(disabled === undefined ? {} : { disabled }),
    ...(onclick === undefined ? {} : { onclick }),
    ...(className === undefined ? {} : { class: className }),
  } as Record<string, unknown>)
</script>

<Button {...buttonProps}>
  <svg aria-hidden="true" viewBox="0 0 24 24">
    <path d="M0 0h24v24H0z" />
  </svg>
</Button>
