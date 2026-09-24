<script lang="ts">
  import { setPaneNavigation } from '../pane-context'
  import type { NavigationStore } from '../navigation'
  import ProbeFixture from './ProbeFixture.svelte'

  let { navigation, paneId }: { navigation: NavigationStore; paneId: string } = $props()

  setPaneNavigation(navigation, paneId)
</script>

<ProbeFixture />
