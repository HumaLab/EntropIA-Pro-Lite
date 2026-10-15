<script lang="ts">
  import { setPaneNavigation } from '$lib/pane-context'
  import type { NavigationStore } from '$lib/navigation'
  import HomeView from '../HomeView.svelte'

  let { navigation, paneId }: { navigation: NavigationStore; paneId: string } = $props()

  // The context is set once at init, like WorkPane: the fixture is mounted
  // with fixed props.
  // svelte-ignore state_referenced_locally
  setPaneNavigation(navigation, paneId)
</script>

<HomeView />
