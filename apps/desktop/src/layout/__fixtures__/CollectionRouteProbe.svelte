<script lang="ts">
  import { untrack } from 'svelte'
  import { routeProbeLog } from './route-probe-log'

  const props: Record<string, unknown> = $props()
  routeProbeLog.push({ probe: 'collection', props: untrack(() => ({ ...props })) })
</script>

<div data-testid="collection-route-probe">collection probe</div>
