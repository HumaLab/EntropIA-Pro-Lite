<script lang="ts">
  import { setPaneNavigation } from '../pane-context'
  import type { NavigationStore } from '../navigation'
  import ProbeFixture from './ProbeFixture.svelte'

  let { navigation, paneId }: { navigation: NavigationStore; paneId: string } = $props()

  // The context is set once at init, like WorkPane: the fixture is mounted
  // with fixed props.
  // svelte-ignore state_referenced_locally
  setPaneNavigation(navigation, paneId)
</script>

<ProbeFixture />
