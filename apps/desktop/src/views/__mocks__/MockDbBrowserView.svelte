<script lang="ts">
  import { onMount } from 'svelte'
  import {
    describeDbBrowserTable,
    listDbBrowserTables,
    queryAllDbBrowserRowsInChunks,
    queryDbBrowserRows,
  } from '$lib/db-browser'
  import { exportCollectionToCsv, exportCollectionToJson } from '$lib/export'

  let tables = $state<Array<{ name: string }>>([])
  let selectedTable = $state('')
  let columns = $state<Array<{ name: string; isPrimaryKey?: boolean }>>([])
  let sortColumn = $state('')
  let exportingTable = $state(false)

  onMount(async () => {
    tables = await listDbBrowserTables()
    selectedTable = tables[0]?.name ?? ''
    if (!selectedTable) return

    columns = await describeDbBrowserTable(selectedTable)
    sortColumn = columns.find((column) => column.isPrimaryKey)?.name ?? columns[0]?.name ?? ''
    await queryDbBrowserRows({
      table: selectedTable,
      page: 1,
      pageSize: 25,
      sortColumn,
      sortDirection: 'asc',
      search: undefined,
    })
  })

  async function exportTable(format: 'json' | 'csv') {
    if (!selectedTable || exportingTable) return

    const exportTable = selectedTable
    const exportColumnNames = columns.map((column) => column.name)
    const exportSortColumn = sortColumn

    exportingTable = true
    try {
      const response = await queryAllDbBrowserRowsInChunks({
        table: exportTable,
        sortColumn: exportSortColumn,
        sortDirection: 'asc',
        search: undefined,
      })

      if (format === 'csv') {
        await exportCollectionToCsv(response.rows, exportColumnNames, `${exportTable}.csv`)
        return
      }

      await exportCollectionToJson(
        {
          table: exportTable,
          scope: 'full_table',
          rows: response.rows,
        },
        `${exportTable}.json`
      )
    } finally {
      exportingTable = false
    }
  }
</script>

<section>
  <span>Base de datos</span>
  <h1>Consulta DB</h1>
  {#if selectedTable}
    <span>{selectedTable} · {columns.length} columnas</span>
  {/if}
  <label for="db-browser-table-select">Tabla</label>
  <select id="db-browser-table-select" bind:value={selectedTable}>
    {#each tables as table (table.name)}
      <option value={table.name}>{table.name}</option>
    {/each}
  </select>
</section>

{#if selectedTable}
  <div class="db-browser-export-actions">
    <button
      class="db-browser-export-button"
      type="button"
      aria-label="Exportar JSON"
      title="Exportar JSON"
      disabled={exportingTable}
      onclick={() => exportTable('json')}
    >
      <svg aria-hidden="true" viewBox="0 0 16 16"><path d="M8 1v10M4 7l4 4 4-4M2 14h12" /></svg>
      <span>JSON</span>
    </button>
    <button
      class="db-browser-export-button"
      type="button"
      aria-label="Exportar CSV"
      title="Exportar CSV"
      disabled={exportingTable}
      onclick={() => exportTable('csv')}
    >
      <svg aria-hidden="true" viewBox="0 0 16 16"><path d="M8 1v10M4 7l4 4 4-4M2 14h12" /></svg>
      <span>CSV</span>
    </button>
  </div>
  <p>Esta tabla no tiene filas para mostrar.</p>
{/if}
