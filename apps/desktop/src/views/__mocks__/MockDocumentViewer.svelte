<script lang="ts">
  let {
    path = '',
    assetUrl = '',
    type = 'image',
    readOnly = false,
    currentPage = 1,
    annotations = [],
    layoutRegions = [],
    hoveredLayoutRegionId = null,
    selectedLayoutRegionId = null,
    selectedAnnotationId = null,
    annotationTool = 'select',
    annotationColor = 'var(--color-accent)',
    editTool = 'none',
    onEditToolChange = () => {},
    onPageChange = () => {},
    onAnnotationsChange = () => {},
    onLayoutRegionHoverChange = () => {},
    onLayoutRegionSelect = () => {},
    onSelectedAnnotationIdChange = () => {},
    onAnnotationToolChange = () => {},
    onAnnotationColorChange = () => {},
    onDimensionsChange = () => {},
    onFineRotateCommit = () => {},
    onEditSelect = () => {},
    onRotateRight = () => {},
    onUndo = () => {},
    onRedo = () => {},
    onDuplicateAsset = () => {},
    duplicateAssetDisabled = false,
    annotationToolbarLabels = {} as { duplicateAsset?: string },
    canUndo = false,
    canRedo = false,
  } = $props()

  function createDraftAnnotation() {
    return {
      id: crypto.randomUUID(),
      assetId: '',
      page: 1,
      kind: 'rectangle',
      color: annotationColor,
      x: 0.1,
      y: 0.2,
      width: 0.3,
      height: 0.4,
      createdAt: 1,
      updatedAt: 1,
    }
  }
</script>

<div
  data-testid="mock-document-viewer"
  data-path={path}
  data-asset-url={assetUrl}
  data-read-only={String(readOnly)}
>
  <p data-testid="viewer-type">{type}</p>
  <p data-testid="viewer-annotation-count">{annotations.length}</p>
  <p data-testid="viewer-layout-region-count">{layoutRegions.length}</p>
  <p data-testid="viewer-current-page">{currentPage}</p>
  <p data-testid="viewer-hovered-layout-region">{hoveredLayoutRegionId ?? 'none'}</p>
  <p data-testid="viewer-selected-layout-region">{selectedLayoutRegionId ?? 'none'}</p>
  <p data-testid="viewer-selected-annotation">{selectedAnnotationId ?? 'none'}</p>
  <p data-testid="viewer-annotation-tool">{annotationTool}</p>
  <p data-testid="viewer-annotation-color">{annotationColor}</p>
  <p data-testid="viewer-edit-tool">{editTool}</p>

  {#if !readOnly}
    <button
      type="button"
      onclick={() => onAnnotationsChange([...annotations, createDraftAnnotation()])}
    >
      Add annotation
    </button>
    <button
      type="button"
      onclick={() => onSelectedAnnotationIdChange(annotations[0]?.id ?? 'missing-annotation')}
    >
      Select annotation
    </button>
  {/if}
  <button type="button" onclick={() => onLayoutRegionHoverChange(layoutRegions[0]?.id ?? null)}>
    Hover first layout region
  </button>
  <button type="button" onclick={() => onLayoutRegionHoverChange(null)}>
    Clear layout hover
  </button>
  <button
    type="button"
    onclick={() =>
      onLayoutRegionSelect(layoutRegions[1]?.id ?? layoutRegions[0]?.id ?? 'missing-layout-region')}
  >
    Select second layout region
  </button>
  <button type="button" aria-label="Go to page 2" onclick={() => onPageChange(2, 2)}>
    Go to page 2
  </button>
  {#if !readOnly}
    <button type="button" onclick={() => onAnnotationToolChange('rectangle')}>Rectangle tool</button
    >
    <button type="button" onclick={() => onEditToolChange('crop')}>Crop tool</button>
    <button type="button" onclick={() => onEditToolChange('erase')}>Erase tool</button>
    <button
      type="button"
      onclick={() => onEditSelect({ x: 0.2, y: 0.25, width: 0.5, height: 0.4 })}
    >
      Apply edit region
    </button>
    <button type="button" onclick={() => onRotateRight()}>Rotate right</button>
    <button
      type="button"
      disabled={duplicateAssetDisabled}
      onclick={() => onDuplicateAsset()}
    >
      {annotationToolbarLabels.duplicateAsset ?? 'Duplicate asset'}
    </button>
    <button type="button" onclick={() => onAnnotationColorChange('var(--color-warning)')}>
      Warning color
    </button>
  {/if}
  <button type="button" onclick={() => onDimensionsChange({ width: 200, height: 100 })}>
    Report image dimensions
  </button>
  {#if !readOnly}
    <button type="button" onclick={() => onFineRotateCommit(3)}> Commit fine rotation </button>
    <button type="button" disabled={!canUndo} onclick={() => onUndo()}> Undo edit </button>
    <button type="button" disabled={!canRedo} onclick={() => onRedo()}> Redo edit </button>
  {/if}
</div>

