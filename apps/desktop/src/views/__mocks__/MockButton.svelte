<script lang="ts">
  /**
   * Stands in for `Button` — and, in several suites, for `IconButton` too.
   *
   * It is faithful on the two things tests query by:
   *
   *  - `title` goes through the tooltip action rather than onto the element,
   *    because that is what both real controls do. A mock that leaked a native
   *    `title` would let a test pass on behaviour the app does not have.
   *  - `label` becomes `aria-label`, which is IconButton's whole contract. Left
   *    spread as-is it lands as an inert `label` attribute and the control has
   *    no accessible name, so `getByRole('button', { name })` cannot see it.
   *
   * The action is imported by path, not from '@entropia/ui': that package is
   * what these suites mock, and reaching for it from inside a mock resolves the
   * mocked module before setup and hangs the run.
   */
  import { tooltip } from '../../../../../packages/ui/src/components/Tooltip/tooltip'
  import type { ButtonProps } from '@entropia/ui'

  let { children, title, label, ...rest }: ButtonProps & { label?: string } = $props()
</script>

<!-- aria-label comes FIRST so a caller that passes its own through the rest
     props wins: placed after the spread, the undefined `label` of a plain Button
     would wipe an aria-label the call site had set. -->
<button aria-label={label} {...rest} use:tooltip={title}>
  {#if children}
    {@render children()}
  {/if}
</button>
