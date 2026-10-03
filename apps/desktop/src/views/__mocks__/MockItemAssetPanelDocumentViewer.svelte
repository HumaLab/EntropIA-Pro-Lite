<script lang="ts">
  let { path = '', assetUrl = '', readOnly = false, audioStartAtSeconds = null } = $props()
</script>

<div
  data-testid="mock-item-asset-panel-document-viewer"
  data-path={path}
  data-asset-url={assetUrl}
  data-read-only={String(readOnly)}
  data-start-at={audioStartAtSeconds === null ? '' : String(audioStartAtSeconds)}
></div>
