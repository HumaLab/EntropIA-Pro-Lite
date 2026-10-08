<script lang="ts">
  import DocumentViewer from '../DocumentViewer.svelte'

  // Pages a PDF the way a host does: only `currentPage` changes, every other
  // prop keeps its identity. (`rerender` replaces the whole props object, which
  // reloads the document and hides a paging bug.)
  let { onPageChange }: { onPageChange: (page: number, total: number) => void } = $props()

  let page = $state(1)
</script>

<button type="button" onclick={() => (page += 1)}>Next page</button>
<button type="button" onclick={() => (page -= 1)}>Previous page</button>
<DocumentViewer
  path="/path/to/doc.pdf"
  type="pdf"
  assetUrl="asset://localhost/path/to/doc.pdf"
  annotations={[]}
  selectedAnnotationId={null}
  annotationTool="select"
  annotationColor="var(--color-accent)"
  currentPage={page}
  {onPageChange}
/>
