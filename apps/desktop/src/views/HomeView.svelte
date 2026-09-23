<script lang="ts">
  /**
   * Inicio (home): the startup overview — what to continue, what to start,
   * how the corpus stands, and where the main workspaces are.
   * Design: odd/tasks/home-view.md (canvas artboards InicioV3 / InicioV3Nuevo).
   *
   * Data comes from `loadHomeSnapshot()` ($lib/home, T2); this view only knows
   * how to lay it out. The snapshot reloads on every mount (App.svelte renders
   * HomeView behind an `{#if}`, so mounting == becoming the current view).
   */
  import { onMount } from 'svelte'
  import { locale, t, type I18nKey } from '$lib/i18n'
  import { navigation } from '$lib/navigation'
  import {
    loadHomeSnapshot,
    isUntitledWritingTitle,
    type HomeRecentEntry,
    type HomeRecentEntryKind,
    type HomeActivityEntry,
  } from '$lib/home'
  import type { HomeSnapshot } from '$lib/home'
  import { syncStore } from '$lib/sync-store'
  import type { SyncStatus } from '$lib/sync'
  import { batchStore, type BatchGlobalSummary, type BatchSummary } from '$lib/batch-processing'
  import ActiveProcessBand from './ActiveProcessBand.svelte'
  import { ActionIcon, Button, formatRelativeDate, type ActionIconName } from '@entropia/ui'

  const currentLocale = locale

  let snapshot = $state<HomeSnapshot | null>(null)
  let loading = $state(true)
  let error = $state<string | null>(null)

  let syncStatus = $state<SyncStatus>(syncStore.status)
  const unsubscribeSync = syncStore.subscribe((next) => {
    syncStatus = next
  })

  // Backs the active-process band below: the same batch queue the statusbar
  // indicator and Lotes already read, so this adds no new data source.
  let batchSummary = $state<BatchGlobalSummary>(batchStore.snapshot())
  const unsubscribeBatch = batchStore.subscribe((next) => {
    batchSummary = next
  })

  async function loadSnapshot() {
    loading = true
    error = null
    try {
      snapshot = await loadHomeSnapshot()
    } catch (caught) {
      error = caught instanceof Error ? caught.message : String(caught)
    } finally {
      loading = false
    }
  }

  onMount(() => {
    void loadSnapshot()
    // Idempotent (SyncStore/BatchStore memoize the bootstrap): safe even when
    // SyncStatusIndicator/BatchStatusIndicator already initialized the same
    // singletons.
    void syncStore.initialize()
    void batchStore.initialize()
    return () => {
      unsubscribeSync()
      unsubscribeBatch()
    }
  })

  // ─── Actions ────────────────────────────────────────────────────────────
  // One function per action so a later task can swap its body without
  // touching the others (T4 owns the import dialog; T5 owns the rest).

  /** T4: opens the import dialog (choose/create a collection, then pick files). */
  function openImportSources() {
    navigation.navigate({ name: 'collections' })
  }

  function openNewResearch() {
    navigation.openRootSection({ name: 'research' })
  }

  function openWritingList() {
    navigation.openRootSection({ name: 'writing' })
  }

  /** T4: opens the import dialog's "create a collection" step directly. */
  function openCreateCollection() {
    navigation.navigate({ name: 'collections' })
  }

  function openCollections() {
    navigation.navigate({ name: 'collections' })
  }

  function openChat() {
    navigation.openRootSection({ name: 'rag-chat' })
  }

  /** Same deep link the statusbar batch indicator uses: focus, then open Configuración. */
  function openBatchTab(batchId: string | null = null) {
    batchStore.requestFocus(batchId)
    navigation.openRootSection({ name: 'settings' })
  }

  function openEntry(entry: HomeRecentEntry | HomeActivityEntry) {
    navigation.navigate(entry.view)
  }

  // ─── Presentation ───────────────────────────────────────────────────────

  const continuarEntries = $derived(snapshot?.continuar.slice(0, 3) ?? [])
  const activityEntries = $derived(snapshot?.activity.slice(0, 5) ?? [])

  // Bare 'es'/'en' locale tags resolve inconsistently across ICU builds (no
  // thousands grouping on some Node builds); the region-qualified tags format
  // reliably everywhere.
  const NUMBER_LOCALE = { es: 'es-AR', en: 'en-US' } as const
  const numberFormatter = $derived.by(() => new Intl.NumberFormat(NUMBER_LOCALE[$currentLocale]))

  function formatCount(value: number): string {
    return numberFormatter.format(value)
  }

  /** "618 / 2.193" — the sub-count against the corpus total. */
  function ratioLabel(part: number, total: number): string {
    return `${formatCount(part)} / ${formatCount(total)}`
  }

  /** Guards the 0/0 case: an empty corpus reads as 0%, never NaN. */
  function percentValue(part: number, total: number): number {
    return total > 0 ? Math.round((part / total) * 100) : 0
  }

  function percentLabel(part: number, total: number): string {
    return `${percentValue(part, total)} %`
  }

  // ─── Active-process band ────────────────────────────────────────────────
  // Backed by the same batch queue as the statusbar indicator and Lotes
  // (T3d). Only OCR/embedding batches report live progress this way today;
  // imports and sync have no equivalent per-unit progress source yet (see
  // the task report).

  const ACTIVE_PROCESS_KIND_KEY: Record<string, I18nKey> = {
    ocr: 'home.activeProcess.kind.ocr',
    embeddings: 'home.activeProcess.kind.embeddings',
  }

  const activeBatch = $derived<BatchSummary | null>(batchSummary.active[0] ?? null)

  function activeProcessTitle(batch: BatchSummary): string {
    const kind = batch.operations[0]
    const key = kind ? ACTIVE_PROCESS_KIND_KEY[kind] : undefined
    return key ? t(key) : (kind ?? '')
  }

  function activeProcessProgress(batch: BatchSummary): string {
    const total = batch.activeUnits + batch.failedUnits + batch.succeededUnits
    const done = batch.failedUnits + batch.succeededUnits
    return t('home.activeProcess.progress', {
      done: formatCount(done),
      total: formatCount(total),
      percent: percentValue(done, total),
    })
  }

  function itemCountLabel(count: number): string {
    return count === 1
      ? t('home.recent.itemCount.one', { count })
      : t('home.recent.itemCount.other', { count })
  }

  function wordCountLabel(count: number): string {
    const formatted = formatCount(count)
    return count === 1
      ? t('home.continuar.wordCount.one', { count: formatted })
      : t('home.continuar.wordCount.other', { count: formatted })
  }

  /** Display-only: never changes the stored title (T3f). */
  function continuarTitle(entry: HomeRecentEntry): string {
    if (entry.kind === 'writing' && isUntitledWritingTitle(entry.title)) {
      return t('home.continuar.untitled')
    }
    return entry.title
  }

  const ROW_ICON: Record<HomeRecentEntryKind, ActionIconName> = {
    collection: 'folder',
    writing: 'edit',
    research: 'research',
  }

  // Continuar's meta line names the category ("Colecciones", like the nav
  // label), while Reciente's Tipo column names this one entry's type
  // ("Colección") — the design draws that distinction on purpose.
  const CONTINUAR_TYPE_KEY: Record<HomeRecentEntryKind, I18nKey> = {
    collection: 'nav.collections',
    writing: 'writing.title',
    research: 'nav.research',
  }

  /**
   * The contextual datum after the relative time: a document count for a
   * collection, a word count for a writing document, nothing for a research
   * job (sources would need one extra agent call per job — out of scope, T3f).
   */
  function continuarDatum(entry: HomeRecentEntry): string | null {
    if (entry.kind === 'collection' && entry.size != null) return itemCountLabel(entry.size)
    if (entry.kind === 'writing' && entry.wordCount != null) return wordCountLabel(entry.wordCount)
    return null
  }

  function continuarMeta(entry: HomeRecentEntry): string {
    const parts = [t(CONTINUAR_TYPE_KEY[entry.kind])]
    if (entry.updatedAt) parts.push(formatRelativeDate(entry.updatedAt.getTime(), $currentLocale))
    const datum = continuarDatum(entry)
    if (datum) parts.push(datum)
    return parts.join(' · ')
  }

  function activityDateLabel(entry: HomeActivityEntry): string {
    return formatRelativeDate(entry.createdAt.getTime(), $currentLocale)
  }

  function rowKeydown(e: KeyboardEvent, entry: HomeRecentEntry | HomeActivityEntry) {
    if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault()
      openEntry(entry)
    }
  }

  const syncVisible = $derived(syncStatus.state !== 'disabled')
  const syncLabel = $derived.by(() => {
    switch (syncStatus.state) {
      case 'syncing':
        return t('sync.statusbar.syncing')
      case 'offline':
        return t('sync.statusbar.offline')
      case 'error':
        return t('sync.statusbar.error')
      case 'idle':
      default:
        return t('sync.statusbar.idle')
    }
  })
</script>

<div class="home-view page-shell">
  <section class="page-header" aria-labelledby="home-title">
    <div class="page-header__content">
      <span class="page-header__eyebrow">{$currentLocale && t('home.eyebrow')}</span>
      <h1 id="home-title">{$currentLocale && t('home.title')}</h1>
      <p class="home-view__description">{$currentLocale && t('home.description')}</p>
    </div>
    <div class="home-view__header-actions">
      {#if !snapshot?.isFirstRun}
        <!-- First run carries this action inside the "Empezá con EntropIA"
             block below, so the header does not offer it twice. -->
        <Button variant="primary" onclick={openImportSources}>
          <ActionIcon name="add" size={16} />
          {$currentLocale && t('home.actions.import')}
        </Button>
      {/if}
      <Button variant="secondary" onclick={openNewResearch}>
        <ActionIcon name="research" size={16} />
        {$currentLocale && t('home.actions.newResearch')}
      </Button>
      <Button variant="secondary" onclick={openWritingList}>
        <ActionIcon name="edit" size={16} />
        {$currentLocale && t('home.actions.newDocument')}
      </Button>
    </div>
  </section>

  {#if error}
    <p class="surface-message surface-message--error" role="alert">{error}</p>
  {/if}

  {#if activeBatch}
    <ActiveProcessBand
      title={activeProcessTitle(activeBatch)}
      progress={activeProcessProgress(activeBatch)}
      openLabel={t('home.activeProcess.open')}
      onOpen={() => openBatchTab(activeBatch!.id)}
    />
  {/if}

  <div class="home-view__top-row" class:home-view__top-row--grow={snapshot?.isFirstRun}>
    <section class="home-panel home-view__continuar" aria-labelledby="home-continuar-title">
      {#if !loading && snapshot?.isFirstRun}
        <div class="home-panel__header">
          <span id="home-continuar-title" class="home-panel__label"
            >{$currentLocale && t('home.firstRun.title')}</span
          >
        </div>
        <div class="home-view__first-run">
          <h2>{$currentLocale && t('home.firstRun.heading')}</h2>
          <p>{$currentLocale && t('home.firstRun.description')}</p>
          <div class="home-view__first-run-actions">
            <Button variant="primary" onclick={openImportSources}>
              {$currentLocale && t('home.actions.import')}
            </Button>
            <Button variant="secondary" onclick={openCreateCollection}>
              {$currentLocale && t('home.firstRun.createCollection')}
            </Button>
          </div>
        </div>
      {:else}
        <div class="home-panel__header">
          <span id="home-continuar-title" class="home-panel__label"
            >{$currentLocale && t('home.continuar.title')}</span
          >
        </div>
        {#if !loading}
          <ul class="home-view__continuar-list">
            {#each continuarEntries as entry (entry.kind + entry.id)}
              <li class="home-view__continuar-item">
                <button
                  type="button"
                  class="home-view__continuar-row"
                  onclick={() => openEntry(entry)}
                >
                  <span class="home-view__continuar-icon">
                    <ActionIcon name={ROW_ICON[entry.kind]} size={20} />
                  </span>
                  <span class="home-view__continuar-copy">
                    <span class="home-view__continuar-row-title">{continuarTitle(entry)}</span>
                    <span class="home-view__continuar-row-meta">{continuarMeta(entry)}</span>
                  </span>
                  <span class="home-view__continuar-resume" aria-hidden="true">
                    {$currentLocale && t('home.continuar.resume')}
                  </span>
                </button>
              </li>
            {/each}
          </ul>
        {/if}
      {/if}
    </section>

    <section class="home-panel home-view__corpus" aria-labelledby="home-corpus-title">
      <div class="home-panel__header">
        <span id="home-corpus-title" class="home-panel__label"
          >{$currentLocale && t('home.corpus.title')}</span
        >
      </div>
      <div class="home-view__corpus-grid">
        <div class="home-view__corpus-line">
          <span class="home-view__corpus-value"
            >{snapshot ? formatCount(snapshot.stats.collections) : '—'}</span
          >
          <span class="home-view__corpus-sep" aria-hidden="true">/</span>
          <span class="home-view__corpus-label"
            >{$currentLocale && t('home.corpus.collections')}</span
          >
        </div>
        <div class="home-view__corpus-line">
          <span class="home-view__corpus-value"
            >{snapshot ? formatCount(snapshot.stats.items) : '—'}</span
          >
          <span class="home-view__corpus-sep" aria-hidden="true">/</span>
          <span class="home-view__corpus-label">{$currentLocale && t('home.corpus.items')}</span>
        </div>
        <div class="home-view__corpus-ratio">
          <div class="home-view__corpus-line">
            <span class="home-view__corpus-value"
              >{snapshot ? ratioLabel(snapshot.stats.ocr, snapshot.stats.items) : '—'}</span
            >
            <span class="home-view__corpus-label"
              >{$currentLocale && t('home.corpus.ocr')}{snapshot
                ? ` · ${percentLabel(snapshot.stats.ocr, snapshot.stats.items)}`
                : ''}</span
            >
          </div>
          <div class="home-view__corpus-bar">
            <div
              class="home-view__corpus-bar-fill"
              style:width="{snapshot ? percentValue(snapshot.stats.ocr, snapshot.stats.items) : 0}%"
            ></div>
          </div>
        </div>
        <div class="home-view__corpus-ratio">
          <div class="home-view__corpus-line">
            <span class="home-view__corpus-value"
              >{snapshot ? ratioLabel(snapshot.stats.embeddings, snapshot.stats.items) : '—'}</span
            >
            <span class="home-view__corpus-label"
              >{$currentLocale && t('home.corpus.embeddings')}{snapshot
                ? ` · ${percentLabel(snapshot.stats.embeddings, snapshot.stats.items)}`
                : ''}</span
            >
          </div>
          <div class="home-view__corpus-bar">
            <div
              class="home-view__corpus-bar-fill"
              style:width="{snapshot
                ? percentValue(snapshot.stats.embeddings, snapshot.stats.items)
                : 0}%"
            ></div>
          </div>
        </div>
      </div>
      <div class="home-view__corpus-footer">
        {#if snapshot && !snapshot.isFirstRun}
          {#if snapshot.stats.pendingOcr > 0}
            <button type="button" class="home-view__corpus-pending" onclick={() => openBatchTab()}>
              <span class="home-view__corpus-dot" aria-hidden="true"></span>
              {$currentLocale && t('home.corpus.pendingOcr', { count: snapshot.stats.pendingOcr })}
            </button>
          {/if}
          {#if snapshot.stats.pendingEmbeddings > 0}
            <button type="button" class="home-view__corpus-pending" onclick={() => openBatchTab()}>
              <span class="home-view__corpus-dot" aria-hidden="true"></span>
              {$currentLocale &&
                t('home.corpus.pendingEmbeddings', { count: snapshot.stats.pendingEmbeddings })}
            </button>
          {/if}
        {/if}
        {#if syncVisible}
          <span class="home-view__corpus-sync">
            <ActionIcon name="check" size={14} />
            {$currentLocale && syncLabel}
          </span>
        {/if}
      </div>
    </section>
  </div>

  <section class="home-view__quick-access" aria-labelledby="home-quick-access-title">
    <span id="home-quick-access-title" class="home-view__section-label"
      >{$currentLocale && t('home.quickAccess.title')}</span
    >
    <div class="home-view__quick-access-grid">
      <button type="button" class="home-view__quick-access-card" onclick={openCollections}>
        <span class="home-view__quick-access-arrow" aria-hidden="true">→</span>
        <ActionIcon name="folder" size={24} />
        <span class="home-view__quick-access-title">{$currentLocale && t('nav.collections')}</span>
        <span class="home-view__quick-access-subtitle"
          >{$currentLocale && t('home.quickAccess.collections.subtitle')}</span
        >
      </button>
      <button type="button" class="home-view__quick-access-card" onclick={openChat}>
        <span class="home-view__quick-access-arrow" aria-hidden="true">→</span>
        <ActionIcon name="message-circle" size={24} />
        <span class="home-view__quick-access-title">{$currentLocale && t('nav.ragChat')}</span>
        <span class="home-view__quick-access-subtitle"
          >{$currentLocale && t('home.quickAccess.chat.subtitle')}</span
        >
      </button>
      <button type="button" class="home-view__quick-access-card" onclick={openNewResearch}>
        <span class="home-view__quick-access-arrow" aria-hidden="true">→</span>
        <ActionIcon name="research" size={24} />
        <span class="home-view__quick-access-title">{$currentLocale && t('nav.research')}</span>
        <span class="home-view__quick-access-subtitle"
          >{$currentLocale && t('home.quickAccess.research.subtitle')}</span
        >
      </button>
      <button type="button" class="home-view__quick-access-card" onclick={openWritingList}>
        <span class="home-view__quick-access-arrow" aria-hidden="true">→</span>
        <ActionIcon name="edit" size={24} />
        <span class="home-view__quick-access-title">{$currentLocale && t('writing.title')}</span>
        <span class="home-view__quick-access-subtitle"
          >{$currentLocale && t('home.quickAccess.writing.subtitle')}</span
        >
      </button>
    </div>
  </section>

  {#if snapshot && !snapshot.isFirstRun && activityEntries.length > 0}
    <section
      class="home-panel home-view__recent"
      aria-labelledby="home-activity-title"
      role="table"
    >
      <div class="home-view__recent-row home-view__recent-row--header" role="row">
        <span role="columnheader" id="home-activity-title"
          >{$currentLocale && t('home.activity.title')}</span
        >
        <span role="columnheader">{$currentLocale && t('home.activity.columnCollection')}</span>
        <span role="columnheader" class="home-view__recent-cell--end"
          >{$currentLocale && t('home.recent.columnModified')}</span
        >
      </div>
      {#each activityEntries as entry (entry.id)}
        <div
          class="home-view__recent-row"
          role="row"
          tabindex="0"
          onclick={() => openEntry(entry)}
          onkeydown={(e) => rowKeydown(e, entry)}
        >
          <span role="cell" class="home-view__recent-name">{entry.title}</span>
          <span role="cell">{entry.collectionName}</span>
          <span role="cell" class="home-view__recent-cell--end">{activityDateLabel(entry)}</span>
        </div>
      {/each}
    </section>
  {/if}
</div>

<style>
  .home-view__description {
    max-width: 60ch;
  }

  .home-view h1 {
    font-family: var(--font-reading);
    font-weight: var(--font-weight-medium);
    font-size: 30px;
    letter-spacing: normal;
  }

  .home-view__header-actions {
    display: flex;
    align-items: flex-end;
    gap: var(--space-2);
    flex-shrink: 0;
  }

  .home-view__top-row {
    display: grid;
    grid-template-columns: 3fr 2fr;
    gap: var(--space-4);
    height: 250px;
  }

  .home-view__top-row--grow {
    height: auto;
    flex: 1;
  }

  @media (max-width: 720px) {
    .home-view__top-row {
      grid-template-columns: 1fr;
      height: auto;
    }
  }

  .home-panel {
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--color-surface-raised);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    overflow: hidden;
  }

  .home-panel__header {
    display: flex;
    align-items: center;
    flex: 0 0 auto;
    height: 40px;
    padding: 0 var(--space-4);
    border-bottom: 1px solid var(--color-hairline);
  }

  .home-panel__label,
  .home-view__section-label {
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.075em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }

  /* ─── Continuar ─── */
  .home-view__continuar-list {
    list-style: none;
    display: flex;
    flex-direction: column;
    flex: 1;
  }

  .home-view__continuar-item {
    flex: 1;
    min-height: 0;
    border-bottom: 1px solid var(--color-hairline);
  }

  .home-view__continuar-item:last-child {
    border-bottom: none;
  }

  .home-view__continuar-row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    height: 100%;
    padding: 0 var(--space-4);
    background: none;
    border: none;
    cursor: pointer;
    text-align: left;
    font: inherit;
    color: inherit;
  }

  .home-view__continuar-row:hover,
  .home-view__continuar-row:focus-visible {
    background: var(--color-accent-faint);
  }

  .home-view__continuar-row:focus-visible {
    outline: none;
  }

  .home-view__continuar-icon {
    display: flex;
    align-items: center;
    justify-content: center;
    flex: 0 0 auto;
    width: 36px;
    height: 36px;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-control);
    color: var(--color-text-secondary);
  }

  .home-view__continuar-copy {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }

  .home-view__continuar-row-title {
    font-family: var(--font-reading);
    font-size: 17px;
    font-weight: var(--font-weight-medium);
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .home-view__continuar-row-meta {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .home-view__continuar-resume {
    flex: 0 0 auto;
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  /* ─── Inicio rápido (first run) ─── */
  .home-view__first-run {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-4);
    flex: 1;
    justify-content: center;
  }

  .home-view__first-run h2 {
    font-family: var(--font-reading);
    font-size: 26px;
    font-weight: var(--font-weight-medium);
  }

  .home-view__first-run-actions {
    display: flex;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }

  /* ─── Estado del corpus ─── */
  .home-view__corpus-grid {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: var(--space-3);
    padding: var(--space-4);
    flex: 1;
  }

  .home-view__corpus-line {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
  }

  .home-view__corpus-value {
    font-family: var(--font-reading);
    font-size: 20px;
    font-variant-numeric: tabular-nums;
  }

  .home-view__corpus-sep {
    color: var(--color-text-muted);
  }

  .home-view__corpus-label {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .home-view__corpus-ratio {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  /* Extremely subtle by design: a neutral hairline track and a slightly
     brighter neutral fill — never a saturated "progress" color. */
  .home-view__corpus-bar {
    height: 3px;
    border-radius: var(--radius-full);
    background: var(--color-hairline);
    overflow: hidden;
  }

  .home-view__corpus-bar-fill {
    height: 100%;
    background: var(--color-text-muted);
    opacity: 0.6;
  }

  .home-view__corpus-footer {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    flex: 0 0 auto;
    padding: var(--space-3) var(--space-4);
    border-top: 1px solid var(--color-hairline);
    font-size: var(--font-size-xs);
  }

  .home-view__corpus-pending,
  .home-view__corpus-sync {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .home-view__corpus-pending {
    background: none;
    border: none;
    padding: 0;
    font: inherit;
    color: var(--color-text-secondary);
    cursor: pointer;
    text-align: left;
  }

  .home-view__corpus-pending:hover,
  .home-view__corpus-pending:focus-visible {
    color: var(--color-text-primary);
  }

  .home-view__corpus-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--color-warning);
    flex-shrink: 0;
  }

  .home-view__corpus-sync {
    color: var(--color-success);
  }

  /* ─── Acceso rápido ─── */
  .home-view__quick-access {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .home-view__quick-access-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--space-3);
  }

  .home-view__quick-access-card {
    position: relative;
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: var(--space-1);
    height: 64px;
    padding: 0 var(--space-4);
    background: var(--color-surface-raised);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    color: var(--color-accent);
    cursor: pointer;
    text-align: left;
  }

  .home-view__quick-access-card:hover,
  .home-view__quick-access-card:focus-visible {
    border-color: var(--color-border-hover);
    background: var(--color-accent-faint);
  }

  .home-view__quick-access-arrow {
    position: absolute;
    top: var(--space-2);
    right: var(--space-3);
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    opacity: 0.55;
    transition: opacity var(--transition-smooth);
  }

  .home-view__quick-access-card:hover .home-view__quick-access-arrow,
  .home-view__quick-access-card:focus-visible .home-view__quick-access-arrow {
    opacity: 1;
    color: var(--color-text-secondary);
  }

  .home-view__quick-access-title {
    font-family: var(--font-reading);
    font-size: 17px;
    color: var(--color-text-primary);
  }

  .home-view__quick-access-subtitle {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  /* ─── Reciente ─── */
  .home-view__recent {
    flex: 1;
  }

  .home-view__recent-row {
    display: grid;
    grid-template-columns: 1fr 200px 130px;
    align-items: center;
    gap: var(--space-3);
    padding: 0 var(--space-4);
    min-height: 36px;
    border-bottom: 1px solid var(--color-hairline);
    cursor: pointer;
  }

  .home-view__recent-row:last-child {
    border-bottom: none;
  }

  .home-view__recent-row:hover,
  .home-view__recent-row:focus-visible {
    background: var(--color-accent-faint);
    outline: none;
  }

  .home-view__recent-row--header {
    cursor: default;
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.075em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }

  .home-view__recent-row--header:hover {
    background: none;
  }

  .home-view__recent-name {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .home-view__recent-cell--end {
    text-align: right;
  }
</style>
