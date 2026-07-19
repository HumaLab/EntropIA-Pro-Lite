<script lang="ts">
  import { onDestroy, onMount, tick } from 'svelte'
  import { ActionIcon, Button } from '@entropia/ui'
  import {
    describeDbBrowserTable,
    listDbBrowserTables,
    queryAllDbBrowserRowsInChunks,
    queryDbBrowserRows,
    type DbBrowserColumn,
    type DbBrowserSortDirection,
    type DbBrowserTable,
  } from '$lib/db-browser'
  import { getDbBrowserCellContent, type DbBrowserCellContent } from '$lib/db-browser-view'
  import { shouldCopyExpandedCellFromShortcut } from '$lib/db-browser-shortcuts'
  import { exportCollectionToJson } from '$lib/export'
  import { locale, t } from '$lib/i18n'
  import { getFocusableElements, getNextFocusTrapTarget } from '$lib/modal-focus'

  const DEFAULT_PAGE_SIZE = 25
  const PAGE_SIZE_OPTIONS = [25, 50, 100] as const
  const COPY_FEEDBACK_TIMEOUT_MS = 2000

  type FeedbackTone = 'success' | 'error'

  type CopyFeedback = {
    tone: FeedbackTone
    text: string
  }

  type ExpandedCell = {
    columnName: string
    text: string
    isJson: boolean
  }

  let tables = $state<DbBrowserTable[]>([])
  let columns = $state<DbBrowserColumn[]>([])
  let rows = $state<Record<string, unknown>[]>([])
  let selectedTable = $state('')
  let searchDraft = $state('')
  let searchTerm = $state('')
  let sortColumn = $state('')
  let sortDirection = $state<DbBrowserSortDirection>('asc')
  let page = $state(1)
  let pageSize = $state<number>(DEFAULT_PAGE_SIZE)
  let total = $state(0)
  let loadingTables = $state(true)
  let loadingRows = $state(false)
  let error = $state<string | null>(null)
  let copyFeedback = $state<CopyFeedback | null>(null)
  let expandedCell = $state<ExpandedCell | null>(null)
  let exportingTable = $state(false)

  let copyFeedbackTimeout: ReturnType<typeof setTimeout> | null = null
  let expandedModalElement = $state<HTMLDivElement | null>(null)
  let expandedCellTrigger: HTMLElement | null = null

  const currentLocale = locale
  const translate = (key: string, params?: Record<string, string | number>) =>
    t(key as never, params)
  const totalPages = $derived(Math.max(1, Math.ceil(total / pageSize) || 1))
  const fromRow = $derived(total === 0 ? 0 : (page - 1) * pageSize + 1)
  const toRow = $derived(total === 0 ? 0 : Math.min(total, page * pageSize))
  const activeSortIcon = $derived(sortDirection === 'asc' ? 'chevron-up' : 'chevron-down')
  onMount(() => {
    loadTables()
  })

  onDestroy(() => {
    if (copyFeedbackTimeout) {
      clearTimeout(copyFeedbackTimeout)
      copyFeedbackTimeout = null
    }
  })

  async function loadTables() {
    loadingTables = true
    error = null

    try {
      const availableTables = await listDbBrowserTables()
      tables = availableTables

      if (availableTables.length === 0) {
        selectedTable = ''
        columns = []
        rows = []
        total = 0
        return
      }

      const firstTable = availableTables[0]
      if (!firstTable) {
        return
      }

      await initializeTable(firstTable.name)
    } catch (err) {
      error = err instanceof Error ? err.message : String(err)
    } finally {
      loadingTables = false
    }
  }

  async function initializeTable(table: string) {
    loadingRows = true
    error = null

    try {
      const nextColumns = await describeDbBrowserTable(table)
      columns = nextColumns
      selectedTable = table
      page = 1

      const defaultSort = pickDefaultSortColumn(nextColumns)
      sortColumn = defaultSort
      sortDirection = 'asc'

      const response = await queryDbBrowserRows({
        table,
        page: 1,
        pageSize,
        sortColumn: defaultSort,
        sortDirection: 'asc',
        search: undefined,
      })

      rows = response.rows
      total = response.total
    } catch (err) {
      error = err instanceof Error ? err.message : String(err)
      rows = []
      total = 0
    } finally {
      loadingRows = false
    }
  }

  async function loadRows() {
    if (!selectedTable || columns.length === 0) return

    loadingRows = true
    error = null

    try {
      const response = await queryDbBrowserRows({
        table: selectedTable,
        page,
        pageSize,
        sortColumn,
        sortDirection,
        search: searchTerm || undefined,
      })

      rows = response.rows
      total = response.total
    } catch (err) {
      error = err instanceof Error ? err.message : String(err)
      rows = []
      total = 0
    } finally {
      loadingRows = false
    }
  }

  function pickDefaultSortColumn(nextColumns: DbBrowserColumn[]): string {
    return nextColumns.find((column) => column.isPrimaryKey)?.name ?? nextColumns[0]?.name ?? ''
  }

  async function handleTableChange(event: Event) {
    searchDraft = ''
    searchTerm = ''
    await initializeTable((event.target as HTMLSelectElement).value)
  }

  async function handleSearchSubmit(event: SubmitEvent) {
    event.preventDefault()
    page = 1
    searchTerm = searchDraft.trim()
    await loadRows()
  }

  async function clearSearch() {
    if (!searchDraft && !searchTerm) return
    searchDraft = ''
    searchTerm = ''
    page = 1
    await loadRows()
  }

  async function handleSort(columnName: string) {
    if (sortColumn === columnName) {
      sortDirection = sortDirection === 'asc' ? 'desc' : 'asc'
    } else {
      sortColumn = columnName
      sortDirection = 'asc'
    }

    page = 1
    await loadRows()
  }

  async function goToPage(nextPage: number) {
    if (nextPage < 1 || nextPage > totalPages || nextPage === page) return
    page = nextPage
    await loadRows()
  }

  function setCopyFeedback(tone: FeedbackTone, text: string) {
    copyFeedback = { tone, text }
    if (copyFeedbackTimeout) clearTimeout(copyFeedbackTimeout)
    copyFeedbackTimeout = setTimeout(() => {
      copyFeedback = null
      copyFeedbackTimeout = null
    }, COPY_FEEDBACK_TIMEOUT_MS)
  }

  async function copyCellValue(value: string) {
    try {
      await navigator.clipboard.writeText(value)
      setCopyFeedback('success', translate('dbBrowser.copySuccess'))
    } catch {
      setCopyFeedback('error', translate('dbBrowser.copyError'))
    }
  }

  function getExpandedModalFocusables(): HTMLElement[] {
    return getFocusableElements(expandedModalElement)
  }

  async function focusExpandedModal() {
    await tick()
    const initialFocusTarget = getExpandedModalFocusables()[0] ?? expandedModalElement
    initialFocusTarget?.focus()
  }

  function restoreExpandedCellFocus() {
    const trigger = expandedCellTrigger
    expandedCellTrigger = null

    if (trigger?.isConnected) {
      trigger.focus()
    }
  }

  function handleExpandedCellKeydown(event: KeyboardEvent) {
    if (expandedCell && shouldCopyExpandedCellFromShortcut(event)) {
      event.preventDefault()
      void copyCellValue(expandedCell.text)
      return
    }

    if (event.key === 'Escape') {
      event.preventDefault()
      closeExpandedCell()
      return
    }

    if (event.key !== 'Tab') return

    const focusableElements = getExpandedModalFocusables()
    if (focusableElements.length === 0) {
      event.preventDefault()
      expandedModalElement?.focus()
      return
    }

    const currentElement =
      document.activeElement instanceof HTMLElement ? document.activeElement : null
    event.preventDefault()
    getNextFocusTrapTarget(
      focusableElements,
      currentElement,
      event.shiftKey,
      expandedModalElement
    )?.focus()
  }

  function openExpandedCell(
    columnName: string,
    text: string,
    isJson: boolean,
    trigger?: HTMLElement | null
  ) {
    expandedCellTrigger =
      trigger ?? (document.activeElement instanceof HTMLElement ? document.activeElement : null)

    expandedCell = {
      columnName,
      text,
      isJson,
    }

    void focusExpandedModal()
  }

  function closeExpandedCell() {
    expandedCell = null
    void tick().then(() => {
      restoreExpandedCellFocus()
    })
  }

  async function handlePageSizeChange(event: Event) {
    const nextPageSize = Number((event.target as HTMLSelectElement).value)

    if (!Number.isFinite(nextPageSize) || nextPageSize <= 0 || nextPageSize === pageSize) {
      return
    }

    const nextState = {
      page: 1,
      pageSize: nextPageSize,
    }

    if (!nextState) return
    pageSize = nextState.pageSize
    page = nextState.page
    await loadRows()
  }

  function resolveCellContent(value: unknown): DbBrowserCellContent {
    return getDbBrowserCellContent(value, translate('dbBrowser.noValue'))
  }

  async function handleExportCurrentTable() {
    if (!selectedTable) return

    exportingTable = true
    error = null
    try {
      const exportResponse = await queryAllDbBrowserRowsInChunks({
        table: selectedTable,
        sortColumn,
        sortDirection,
        search: searchTerm || undefined,
      })
      const exportedPath = await exportCollectionToJson(
        {
          version: 1,
          exportedAt: new Date().toISOString(),
          table: selectedTable,
          columns,
          scope: 'full_table',
          total: exportResponse.total,
          sortColumn,
          sortDirection,
          search: searchTerm || null,
          rows: exportResponse.rows,
        },
        `${selectedTable}.json`
      )
      if (exportedPath) {
        setCopyFeedback('success', translate('dbBrowser.exportSuccess'))
      }
    } catch (err) {
      error = err instanceof Error ? err.message : String(err)
      setCopyFeedback('error', translate('dbBrowser.exportError'))
    } finally {
      exportingTable = false
    }
  }
</script>

<div class="db-browser-view page-shell">
  <section class="page-header db-browser-view__header">
    <div class="page-header__content">
      <span class="page-header__eyebrow">{$currentLocale && translate('dbBrowser.eyebrow')}</span>
      <h1>{$currentLocale && translate('dbBrowser.title')}</h1>
      <p>{$currentLocale && translate('dbBrowser.subtitle')}</p>
      {#if selectedTable}
        <span class="page-header__meta"
          >{selectedTable} · {$currentLocale &&
            translate('dbBrowser.columnsCount', { count: columns.length })}</span
        >
      {/if}
    </div>

    <div class="page-toolbar db-browser-toolbar">
      <div class="db-browser-toolbar__field">
        <label for="db-browser-table-select"
          >{$currentLocale && translate('dbBrowser.tableLabel')}</label
        >
        <select
          id="db-browser-table-select"
          class="db-browser-toolbar__input"
          bind:value={selectedTable}
          onchange={handleTableChange}
          disabled={loadingTables || tables.length === 0}
        >
          {#each tables as table (table.name)}
            <option value={table.name}>{table.name}</option>
          {/each}
        </select>
      </div>

      <form class="db-browser-toolbar__search" onsubmit={handleSearchSubmit}>
        <div class="db-browser-toolbar__field">
          <label for="db-browser-search"
            >{$currentLocale && translate('dbBrowser.searchLabel')}</label
          >
          <input
            id="db-browser-search"
            class="db-browser-toolbar__input"
            type="search"
            bind:value={searchDraft}
            placeholder={$currentLocale && translate('dbBrowser.searchPlaceholder')}
          />
        </div>

        <div class="db-browser-toolbar__actions">
          <Button variant="secondary" type="submit" disabled={loadingTables || loadingRows}>
            {$currentLocale && translate('dbBrowser.searchSubmit')}
          </Button>
          <Button
            variant="ghost"
            type="button"
            onclick={clearSearch}
            disabled={loadingTables || loadingRows}
          >
            {$currentLocale && translate('dbBrowser.searchClear')}
          </Button>
          <Button
            variant="ghost"
            type="button"
            onclick={loadRows}
            disabled={!selectedTable || loadingRows}
          >
            {$currentLocale && translate('dbBrowser.refresh')}
          </Button>
        </div>
      </form>
    </div>
  </section>

  {#if error}
    <p class="surface-message surface-message--error">{error}</p>
  {/if}

  {#if copyFeedback}
    <p
      class="surface-message"
      class:surface-message--error={copyFeedback.tone === 'error'}
      class:surface-message--success={copyFeedback.tone === 'success'}
      role={copyFeedback.tone === 'error' ? 'alert' : 'status'}
      aria-live={copyFeedback.tone === 'error' ? 'assertive' : 'polite'}
      aria-atomic="true"
    >
      {copyFeedback.text}
    </p>
  {/if}

  {#if loadingTables}
    <p class="surface-message surface-message--center">
      {$currentLocale && translate('dbBrowser.loadingTables')}
    </p>
  {:else if tables.length === 0}
    <div class="surface-message surface-message--center">
      <p>{$currentLocale && translate('dbBrowser.emptyTables')}</p>
    </div>
  {:else}
    <section class="db-browser-card">
      <div class="db-browser-card__meta">
        <span>{$currentLocale && translate('dbBrowser.totalRows', { count: total })}</span>
        {#if total > 0}
          <span
            >{$currentLocale &&
              translate('dbBrowser.pageSummary', { from: fromRow, to: toRow, total })}</span
          >
        {/if}
        {#if sortColumn}
          <span class="db-browser-sort-summary">
            <span>{sortColumn}</span>
            <ActionIcon name={activeSortIcon} size={14} />
          </span>
        {/if}
        <button
          class="db-browser-export-button"
          type="button"
          onclick={handleExportCurrentTable}
          disabled={!selectedTable || loadingRows || exportingTable}
          aria-label={translate('dbBrowser.exportTable')}
          title={translate('dbBrowser.exportTable')}
        >
          <ActionIcon name="download" size={16} />
          <span>{$currentLocale && translate('dbBrowser.exportTable')}</span>
        </button>
        <div class="db-browser-page-size">
          <label for="db-browser-page-size"
            >{$currentLocale && translate('dbBrowser.pageSizeLabel')}</label
          >
          <select
            id="db-browser-page-size"
            class="db-browser-toolbar__input db-browser-page-size__select"
            bind:value={pageSize}
            onchange={handlePageSizeChange}
            disabled={loadingRows}
          >
            {#each PAGE_SIZE_OPTIONS as option (option)}
              <option value={option}>{option}</option>
            {/each}
          </select>
        </div>
      </div>

      {#if loadingRows}
        <p class="surface-message surface-message--center">
          {$currentLocale && translate('dbBrowser.loadingRows')}
        </p>
      {:else if rows.length === 0}
        <div class="surface-message surface-message--center">
          <p>
            {searchTerm ? translate('dbBrowser.emptyFiltered') : translate('dbBrowser.emptyRows')}
          </p>
        </div>
      {:else}
        <div class="db-browser-table-wrap">
          <table class="db-browser-table">
            <thead>
              <tr>
                {#each columns as column (column.name)}
                  <th>
                    <button
                      type="button"
                      class="db-browser-table__sort"
                      onclick={() => handleSort(column.name)}
                    >
                      <span>{column.name}</span>
                      {#if sortColumn === column.name}
                        <span class="db-browser-table__sort-indicator">
                          <ActionIcon name={activeSortIcon} size={14} />
                        </span>
                      {/if}
                    </button>
                  </th>
                {/each}
              </tr>
            </thead>
            <tbody>
              {#each rows as row, rowIndex (rowIndex)}
                <tr>
                  {#each columns as column (column.name)}
                    {@const cell = resolveCellContent(row[column.name])}
                    {@const copyCellLabel = translate('dbBrowser.copyCellAria', {
                      column: column.name,
                    })}
                    {@const expandCellLabel = translate('dbBrowser.expandCellAria', {
                      column: column.name,
                    })}
                    <td title={cell.rawText}>
                      <div class="db-browser-table__cell-wrap">
                        <span class="db-browser-table__cell">{cell.rawText}</span>
                        <div class="db-browser-table__cell-actions">
                          {#if cell.hasValue}
                            <span class="db-browser-action-shell" data-tooltip={copyCellLabel}>
                              <button
                                type="button"
                                class="db-browser-table__cell-action"
                                aria-label={copyCellLabel}
                                title={copyCellLabel}
                                onclick={() => copyCellValue(cell.rawText)}
                              >
                                <ActionIcon name="copy" size={14} />
                              </button>
                            </span>
                          {/if}
                          {#if cell.canExpand}
                            <span class="db-browser-action-shell" data-tooltip={expandCellLabel}>
                              <button
                                type="button"
                                class="db-browser-table__cell-action"
                                aria-label={expandCellLabel}
                                title={expandCellLabel}
                                onclick={(event) =>
                                  openExpandedCell(
                                    column.name,
                                    cell.expandedText,
                                    cell.isJson,
                                    event.currentTarget as HTMLElement
                                  )}
                              >
                                <ActionIcon name="expand" size={14} />
                              </button>
                            </span>
                          {/if}
                        </div>
                      </div>
                    </td>
                  {/each}
                </tr>
              {/each}
            </tbody>
          </table>
        </div>
      {/if}

      <div class="db-browser-pagination">
        <span>{$currentLocale && translate('dbBrowser.pageStatus', { page, totalPages })}</span>
        <div class="db-browser-pagination__actions">
          <Button
            variant="ghost"
            onclick={() => goToPage(page - 1)}
            disabled={loadingRows || page <= 1}
          >
            {$currentLocale && translate('dbBrowser.previousPage')}
          </Button>
          <Button
            variant="ghost"
            onclick={() => goToPage(page + 1)}
            disabled={loadingRows || page >= totalPages || total === 0}
          >
            {$currentLocale && translate('dbBrowser.nextPage')}
          </Button>
        </div>
      </div>
    </section>
  {/if}

  {#if expandedCell}
    {@const activeExpandedCell = expandedCell}
    {@const copyExpandedLabel = translate('dbBrowser.copyExpandedAria', {
      column: activeExpandedCell.columnName,
    })}
    <div class="modal-overlay" onclick={closeExpandedCell} role="presentation">
      <div
        class="modal db-browser-modal"
        tabindex="-1"
        role="dialog"
        aria-modal="true"
        aria-labelledby="db-browser-modal-title"
        aria-describedby="db-browser-modal-description"
        bind:this={expandedModalElement}
        onclick={(event) => event.stopPropagation()}
        onkeydown={handleExpandedCellKeydown}
      >
        <div class="db-browser-modal__header">
          <div>
            <h3 id="db-browser-modal-title" class="modal-title">
              {translate('dbBrowser.expandDialogTitle', { column: activeExpandedCell.columnName })}
            </h3>
            <p id="db-browser-modal-description" class="db-browser-modal__subtitle">
              {activeExpandedCell.isJson
                ? translate('dbBrowser.expandDialogJson')
                : translate('dbBrowser.expandDialogText')}
            </p>
          </div>
          <div class="db-browser-modal__actions">
            <span class="db-browser-action-shell" data-tooltip={copyExpandedLabel}>
              <button
                type="button"
                class="db-browser-table__cell-action db-browser-modal__icon-action"
                aria-label={copyExpandedLabel}
                title={copyExpandedLabel}
                onclick={() => copyCellValue(activeExpandedCell.text)}
              >
                <ActionIcon name="copy" size={14} />
              </button>
            </span>
            <Button variant="secondary" onclick={closeExpandedCell}>
              {$currentLocale && translate('dbBrowser.expandDialogClose')}
            </Button>
          </div>
        </div>

        <pre class="db-browser-modal__content"><code>{activeExpandedCell.text}</code></pre>
      </div>
    </div>
  {/if}
</div>

<style>
  .db-browser-view {
    min-height: 100%;
  }

  .db-browser-toolbar {
    display: flex;
    flex: 1;
    gap: var(--space-3);
    justify-content: flex-end;
  }

  .db-browser-toolbar__search {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    align-items: flex-end;
    justify-content: flex-end;
    flex: 1;
  }

  .db-browser-toolbar__field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: min(100%, 220px);
  }

  .db-browser-toolbar__field label {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.075em;
    text-transform: uppercase;
    color: var(--color-text-secondary);
  }

  .db-browser-toolbar__input {
    min-height: var(--control-height-md);
    padding: 0 var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
    background: var(--color-surface-sunken);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    transition:
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth),
      background-color var(--transition-smooth);
  }

  .db-browser-toolbar__input:focus {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
    background: var(--color-surface);
  }

  .db-browser-toolbar__actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .db-browser-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-4);
    border: 1px solid var(--color-border-subtle);
    border-radius: var(--radius-surface);
    background: var(--color-surface-glass);
    box-shadow: var(--shadow-surface);
  }

  .db-browser-card__meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    align-items: center;
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  .db-browser-sort-summary {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
  }

  .db-browser-page-size {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    margin-left: auto;
  }

  .db-browser-export-button {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    min-height: var(--control-height-sm);
    padding: 0 var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-control);
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    cursor: pointer;
    transition:
      border-color var(--transition-smooth),
      color var(--transition-smooth),
      background-color var(--transition-smooth);
  }

  .db-browser-export-button :global(svg) {
    flex-shrink: 0;
  }

  .db-browser-export-button:hover:not(:disabled),
  .db-browser-export-button:focus-visible {
    outline: none;
    border-color: var(--color-accent);
    color: var(--color-text-primary);
    background: color-mix(in srgb, var(--color-accent) 10%, var(--color-surface-raised));
  }

  .db-browser-export-button:focus-visible {
    box-shadow: var(--focus-ring);
  }

  .db-browser-export-button:disabled {
    cursor: not-allowed;
    opacity: 0.48;
  }

  .db-browser-page-size label {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.075em;
    text-transform: uppercase;
    color: var(--color-text-secondary);
  }

  .db-browser-page-size__select {
    min-width: 92px;
    padding-right: var(--space-8);
  }

  .db-browser-table-wrap {
    overflow: auto;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
    background: var(--color-surface-sunken);
  }

  .db-browser-table {
    width: 100%;
    border-collapse: collapse;
    min-width: 720px;
  }

  .db-browser-table th,
  .db-browser-table td {
    padding: var(--space-3);
    border-bottom: 1px solid var(--color-hairline);
    text-align: left;
    vertical-align: top;
    font-size: var(--font-size-sm);
  }

  .db-browser-table thead th {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--color-surface);
  }

  .db-browser-table__sort {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--color-text-primary);
    font: inherit;
    font-weight: var(--font-weight-semibold);
    cursor: pointer;
  }

  .db-browser-table__sort:focus-visible {
    outline: none;
    border-radius: var(--radius-xs);
    box-shadow: var(--focus-ring);
  }

  .db-browser-table__sort-indicator {
    display: inline-flex;
    color: var(--color-accent);
  }

  .db-browser-table__cell {
    display: inline-block;
    max-width: 320px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--color-text-secondary);
  }

  .db-browser-table__cell-wrap {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    justify-content: space-between;
  }

  .db-browser-table__cell-actions {
    display: inline-flex;
    flex-shrink: 0;
    gap: var(--space-1);
  }

  .db-browser-action-shell {
    position: relative;
    display: inline-flex;
  }

  .db-browser-action-shell::after {
    content: attr(data-tooltip);
    position: absolute;
    left: 50%;
    bottom: calc(100% + var(--space-2));
    transform: translateX(-50%) translateY(4px);
    opacity: 0;
    pointer-events: none;
    z-index: 3;
    min-width: max-content;
    max-width: 220px;
    padding: var(--space-1) var(--space-2);
    border: 1px solid color-mix(in srgb, var(--color-accent) 24%, var(--color-border-subtle));
    border-radius: var(--radius-sm);
    background: var(--color-surface-elevated);
    box-shadow: var(--shadow-md);
    color: var(--color-text-primary);
    font-size: var(--font-size-2xs);
    line-height: 1.35;
    text-align: center;
    white-space: normal;
    transition:
      opacity 0.14s ease,
      transform 0.14s ease;
  }

  .db-browser-action-shell:has(.db-browser-table__cell-action:hover)::after,
  .db-browser-action-shell:has(.db-browser-table__cell-action:focus-visible)::after {
    opacity: 1;
    transform: translateX(-50%) translateY(0);
  }

  .db-browser-table__cell-action {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm);
    background: var(--color-surface-raised);
    color: var(--color-text-secondary);
    cursor: pointer;
    transition:
      border-color var(--transition-smooth),
      color var(--transition-smooth),
      background-color var(--transition-smooth);
  }

  .db-browser-table__cell-action :global(svg) {
    flex-shrink: 0;
  }

  .db-browser-table__cell-action:hover,
  .db-browser-table__cell-action:focus-visible {
    outline: none;
    border-color: var(--color-accent);
    color: var(--color-text-primary);
    background: color-mix(in srgb, var(--color-accent) 10%, var(--color-surface-raised));
  }

  .db-browser-table__cell-action:focus-visible {
    box-shadow: var(--focus-ring);
  }

  .modal-overlay {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-4);
    background-color: var(--surface-overlay);
    z-index: 1000;
  }

  .modal-title {
    margin: 0;
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-bold);
    color: var(--color-text-primary);
  }

  .db-browser-modal {
    width: min(900px, calc(100vw - 2rem));
    max-height: min(80vh, 720px);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-6);
    background: var(--color-surface-glass);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-dialog);
    box-shadow: var(--shadow-lg);
  }

  .db-browser-modal__header {
    display: flex;
    gap: var(--space-3);
    align-items: flex-start;
    justify-content: space-between;
  }

  .db-browser-modal__subtitle {
    margin: var(--space-1) 0 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .db-browser-modal__content {
    margin: 0;
    padding: var(--space-4);
    overflow: auto;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
    background: var(--color-surface-sunken);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .db-browser-pagination {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .db-browser-pagination__actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  @media (max-width: 900px) {
    .db-browser-toolbar,
    .db-browser-toolbar__search,
    .db-browser-toolbar__field {
      width: 100%;
    }

    .db-browser-toolbar__actions,
    .db-browser-pagination__actions,
    .db-browser-page-size {
      width: 100%;
    }

    .db-browser-page-size {
      margin-left: 0;
      justify-content: space-between;
    }

    .db-browser-table__cell-wrap,
    .db-browser-modal__header {
      flex-direction: column;
    }

    .db-browser-toolbar__actions :global(.btn),
    .db-browser-pagination__actions :global(.btn) {
      flex: 1 1 0;
    }
  }
</style>
