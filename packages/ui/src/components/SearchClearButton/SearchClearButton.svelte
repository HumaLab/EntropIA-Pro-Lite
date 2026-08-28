<script lang="ts">
  import { ActionIcon } from '../Button'
  import { IconButton } from '../IconButton'
  import type { SearchClearButtonProps } from './SearchClearButton.types'

  let { label, title, class: className = '', ...rest }: SearchClearButtonProps = $props()
</script>

<IconButton
  class="search-clear-button {className}"
  size="sm"
  variant="ghost"
  label={label}
  title={title ?? label}
  type="button"
  {...rest}
>
  <ActionIcon name="close" size={14} />
</IconButton>

<style>
  :global(.icon-button.icon-button--sm.search-clear-button) {
    width: 24px;
    height: 24px;
    border-radius: var(--radius-sm);
  }

  :global(.icon-button.search-clear-button--overlay) {
    position: absolute;
    top: 50%;
    right: var(--space-2);
    transform: translateY(-50%);
  }

  :global(.icon-button.search-clear-button--overlay:hover:not(:disabled)) {
    transform: translateY(-50%);
  }
</style>
