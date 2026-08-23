<script lang="ts">
  let {
    path = '',
    assetUrl = '',
    readOnly = false,
  } = $props()
</script>

<div
  data-testid="mock-item-asset-panel-document-viewer"
  data-path={path}
  data-asset-url={assetUrl}
  data-read-only={String(readOnly)}
></div>
