<script module lang="ts">
  // A record of every paneId this mock was constructed with, in order. Since
  // Svelte only runs a component's top-level <script> once per instance (at
  // creation, never on a prop-only update), pushing here on every mount is
  // exactly "how many times WorkPane was newly instantiated" — the signal
  // AppShell.test.ts needs to prove a tab switch remounts WorkPane (per the
  // {#key wsSnapshot.activeTabId} binding in AppShell.svelte) rather than
  // just re-propping the same instance.
  export const mountLog: string[] = []
</script>

<script lang="ts">
  let { paneId }: { paneId: string } = $props()
  mountLog.push(paneId)
</script>

<div data-testid="app-shell-child" data-pane-id={paneId}>Contenido de prueba</div>
