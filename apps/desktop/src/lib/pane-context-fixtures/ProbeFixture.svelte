<script lang="ts">
  import { getNavigation, getPaneId } from '../pane-context'

  const navigation = getNavigation()
  const paneId = getPaneId()
</script>

<span data-testid="probe-current">{navigation.current.name}</span>
<span data-testid="probe-pane-id">{paneId}</span>
