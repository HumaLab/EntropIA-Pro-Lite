<script lang="ts">
  import { untrack } from 'svelte'
  import { routeProbeLog } from './route-probe-log'

  const props: Record<string, unknown> = $props()
  routeProbeLog.push({ probe: 'item', props: untrack(() => ({ ...props })) })
</script>

<div data-testid="item-route-probe">item probe</div>
