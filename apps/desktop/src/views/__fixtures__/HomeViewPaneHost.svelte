<script lang="ts">
  import { setPaneNavigation } from '$lib/pane-context'
  import type { NavigationStore } from '$lib/navigation'
  import HomeView from '../HomeView.svelte'

  let { navigation, paneId }: { navigation: NavigationStore; paneId: string } = $props()

  setPaneNavigation(navigation, paneId)
</script>

<HomeView />
