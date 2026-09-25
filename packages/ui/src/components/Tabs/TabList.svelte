<script lang="ts">
  import type { TabListProps } from './Tabs.types'

  let { children, class: className = '', ...rest }: TabListProps = $props()
</script>

<div class="tab-list {className}" role="tablist" {...rest}>
  {#if children}
    {@render children()}
  {/if}
</div>

<style>
  .tab-list {
    display: inline-flex;
    /* Never wider than its container: in a narrow pane the tabs wrap onto
       another row instead of being clipped out of reach. */
    flex-wrap: wrap;
    max-width: 100%;
    box-sizing: border-box;
    align-items: center;
    gap: var(--space-1);
    min-height: 32px;
    padding: var(--space-1);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-input);
    background: var(--surface-input);
  }
</style>
