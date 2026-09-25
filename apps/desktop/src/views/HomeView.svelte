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
  import { getNavigation, getPaneId } from '$lib/pane-context'
  import { workspace } from '$lib/workspace'
  import {
    loadHomeSnapshot,
    CONTINUAR_LIMIT,
    isUntitledWritingTitle,
    type HomeRecentEntry,
    type HomeRecentEntryKind,
    type HomeActivityEntry,
  } from '$lib/home'
  import type { HomeSnapshot } from '$lib/home'
  import { batchStore, type BatchGlobalSummary, type BatchSummary } from '$lib/batch-processing'
  import { writing, isReusableBlankDocument } from '$lib/writing'
  import { ragChat } from '$lib/rag-chat'
  import { requestCreateCollection } from '$lib/document-explorer'
  import ActiveProcessBand from './ActiveProcessBand.svelte'
  import ImportSourcesDialog from './ImportSourcesDialog.svelte'
  import { ActionIcon, Button, formatRelativeDate, type ActionIconName } from '@entropia/ui'

  const navigation = getNavigation()
  const paneId = getPaneId()

  const currentLocale = locale

  let snapshot = $state<HomeSnapshot | null>(null)
  let showImportDialog = $state(false)
  let loading = $state(true)
  let error = $state<string | null>(null)
  // A header-action failure (e.g. "Nuevo documento") never blanks the page
  // the way a snapshot-load failure does — it is its own inline message, and
  // the user stays exactly where they were (T5).
  let actionError = $state<string | null>(null)

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
    // Idempotent (BatchStore memoizes the bootstrap): safe even when
    // BatchStatusIndicator already initialized the same singleton.
    void batchStore.initialize()
    return () => {
      unsubscribeBatch()
    }
  })

  // ─── Actions ────────────────────────────────────────────────────────────
  // One function per action so a later task can swap its body without
  // touching the others (T4 owns the import dialog; T5 owns the rest).

  /** Opens the "Importar fuentes" dialog: choose/create a collection, then pick files. */
  function openImportSources() {
    showImportDialog = true
  }

  function closeImportDialog() {
    showImportDialog = false
  }

  function openNewResearch() {
    // ResearchView's create-research form is not behind a toggle: it is
    // always rendered beside the job list (research-form-title), so opening
    // the section already lands the investigator on it directly (T5).
    navigation.openRootSection({ name: 'research' })
  }

  /**
   * Opens Chat on a fresh conversation — the same `startNew()` Chat's own
   * "new conversation" button calls. Initialize first: RagChatView's mount
   * initializes too, and a first initialize restores the last conversation,
   * which would otherwise land after startNew() and undo it.
   */
  async function startNewChat() {
    await ragChat.initialize()
    ragChat.startNew()
    navigation.openRootSection({ name: 'rag-chat' })
  }

  function openWritingList() {
    workspace.navigateActive({ name: 'writing' })
  }

  /**
   * Creates a new writing document and opens it directly — the same call
   * WritingView's own "new document" action makes (`store.createDocument`,
   * default title `writing.newDocumentTitle`), reused here instead of
   * duplicated (T5). Failure stays inline and on Inicio, never navigates.
   *
   * Chrome-like reuse of a blank new tab (visual polish round, split view):
   * if the currently open writing document is already an untouched blank
   * one (`isReusableBlankDocument`), this shows that document instead of
   * creating another — the module-singleton store's `open`/`content` are
   * read directly rather than through a subscription, since only their
   * value at click time matters here.
   */
  async function createNewDocument() {
    actionError = null
    const current = writing.snapshot
    if (isReusableBlankDocument(current)) {
      workspace.navigateActive({
        name: 'writing',
        documentId: current.open!.id,
        documentTitle: current.open!.title,
      })
      return
    }
    const title = t('writing.newDocumentTitle')
    const id = await writing.createDocument(title)
    if (id) {
      workspace.navigateActive({ name: 'writing', documentId: id, documentTitle: title })
    } else {
      actionError = t('home.actions.newDocumentError')
    }
  }

  /**
   * Opens Colecciones with its own create-collection form already open — the
   * exact flow the sidebar's "new collection" button uses (AppShell), reused
   * here instead of a second, divergent create form (T5). Home is never
   * itself the Colecciones view, so the section always needs navigating to.
   */
  function openCreateCollection() {
    requestCreateCollection(false)
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
    workspace.navigateActive(entry.view)
  }

  // ─── Presentation ───────────────────────────────────────────────────────

  const continuarEntries = $derived(snapshot?.continuar.slice(0, CONTINUAR_LIMIT) ?? [])
  const activityEntries = $derived(snapshot?.activity.slice(0, 5) ?? [])

  // Per-panel degradation (T3l): a failing source must never blank the whole
  // page. `stats` is `null` exactly when the corpus-stats query failed, and
  // `errors.continuar`/`errors.activity` mark that Continuar/Actividad have
  // nothing because their source(s) failed, not because the archive is
  // genuinely empty (isFirstRun already accounts for that distinction).
  const stats = $derived(snapshot?.stats ?? null)
  const continuarHasError = $derived(
    Boolean(snapshot?.errors.continuar) && continuarEntries.length === 0
  )
  const activityHasError = $derived(
    Boolean(snapshot?.errors.activity) && activityEntries.length === 0
  )

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

  /**
   * "28 %", or an em dash when the denominator is 0 — an empty universe (no
   * documents to OCR, no audio to transcribe, an empty corpus) reads as
   * "nothing to measure yet", never as a misleading "0 %" (T3i).
   */
  function percentLabel(part: number, total: number): string {
    return total > 0 ? `${percentValue(part, total)} %` : '—'
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

  // Continuar names each entry's own type in the singular ("Colección"),
  // matching "Escritura"/"Investigación" — never the plural nav label
  // ("Colecciones"), which names the whole section instead of this one entry.
  const CONTINUAR_TYPE_KEY: Record<HomeRecentEntryKind, I18nKey> = {
    collection: 'home.continuar.type.collection',
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
</script>

<div class="home-view page-shell">
  <section class="page-header home-view__header" aria-labelledby="home-title-{paneId}">
    <div class="page-header__content">
      <span class="page-header__eyebrow">{$currentLocale && t('home.title')}</span>
      <h1 id="home-title-{paneId}">{$currentLocale && t('home.heading')}</h1>
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
      <Button variant="secondary" onclick={startNewChat}>
        <ActionIcon name="message-circle-plus" size={16} />
        {$currentLocale && t('home.actions.newChat')}
      </Button>
      <Button variant="secondary" onclick={createNewDocument}>
        <ActionIcon name="edit" size={16} />
        {$currentLocale && t('home.actions.newDocument')}
      </Button>
    </div>
  </section>

  {#if error}
    <p class="surface-message surface-message--error" role="alert">{error}</p>
  {/if}

  {#if actionError}
    <p class="surface-message surface-message--error" role="alert">{actionError}</p>
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
    <section
      class="home-panel home-view__continuar"
      aria-labelledby="home-continuar-title-{paneId}"
    >
      {#if !loading && snapshot?.isFirstRun}
        <div class="home-panel__header">
          <span id="home-continuar-title-{paneId}" class="home-panel__label"
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
          <span id="home-continuar-title-{paneId}" class="home-panel__label"
            >{$currentLocale && t('home.continuar.title')}</span
          >
        </div>
        {#if !loading}
          {#if continuarHasError}
            <p class="surface-message surface-message--error home-view__panel-error" role="alert">
              {snapshot?.errors.continuar}
            </p>
          {:else}
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
      {/if}
    </section>

    <section class="home-panel home-view__corpus" aria-labelledby="home-corpus-title-{paneId}">
      <div class="home-panel__header">
        <span id="home-corpus-title-{paneId}" class="home-panel__label"
          >{$currentLocale && t('home.corpus.title')}</span
        >
      </div>

      {#snippet corpusCell(icon: ActionIconName, value: string, labelKey: I18nKey)}
        <div class="home-view__corpus-cell">
          <span class="home-view__corpus-cell-icon"><ActionIcon name={icon} size={20} /></span>
          <span class="home-view__corpus-cell-copy">
            <span class="home-view__corpus-cell-value">{value}</span>
            <span class="home-view__corpus-cell-label">{$currentLocale && t(labelKey)}</span>
          </span>
        </div>
      {/snippet}

      {#snippet corpusStage(
        icon: ActionIconName,
        labelKey: I18nKey,
        metaKey: I18nKey,
        part: number | null,
        total: number | null,
        full: boolean = false
      )}
        <div class="home-view__corpus-stage" class:home-view__corpus-stage--full={full}>
          <span class="home-view__corpus-stage-icon"><ActionIcon name={icon} size={16} /></span>
          <div class="home-view__corpus-stage-body">
            <div class="home-view__corpus-stage-head">
              <span class="home-view__corpus-stage-label">{$currentLocale && t(labelKey)}</span>
              <span class="home-view__corpus-stage-value"
                >{part !== null && total !== null ? ratioLabel(part, total) : '—'}</span
              >
            </div>
            <span class="home-view__corpus-stage-meta"
              >{$currentLocale && t(metaKey)}{part !== null && total !== null
                ? ` · ${percentLabel(part, total)}`
                : ''}</span
            >
            <div class="home-view__corpus-bar">
              <div
                class="home-view__corpus-bar-fill"
                style:width="{part !== null && total !== null ? percentValue(part, total) : 0}%"
              ></div>
            </div>
          </div>
        </div>
      {/snippet}

      {#if !loading && snapshot && !stats}
        <p class="surface-message surface-message--error home-view__panel-error" role="alert">
          {snapshot.errors.stats}
        </p>
      {:else}
        <div class="home-view__corpus-grid">
          <div class="home-view__corpus-top">
            {@render corpusCell(
              'folder',
              stats ? formatCount(stats.collections) : '—',
              'home.corpus.collections'
            )}
            {@render corpusCell(
              'file',
              stats ? formatCount(stats.items) : '—',
              'home.corpus.items'
            )}
          </div>
          <!-- OCR and STT are each a ratio of their OWN universe of applicable
               documents (images/scanned PDFs; audio), not of every document —
               mixing incompatible document types under one denominator (T3i). -->
          <div class="home-view__corpus-pair">
            {@render corpusStage(
              'scan',
              'home.corpus.ocr',
              'home.corpus.meta.ocr',
              stats ? stats.ocr : null,
              stats ? stats.ocrUniverse : null
            )}
            {@render corpusStage(
              'mic',
              'home.corpus.stt',
              'home.corpus.meta.stt',
              stats ? stats.stt : null,
              stats ? stats.sttUniverse : null
            )}
          </div>
          <!-- Texto is a ratio of every viewable file in the corpus
               (textUniverse: pages, images and audios together, T7), not the
               document count. Embeddings is a ratio of files WITH TEXT
               (denominator = text, not textUniverse), so a file can never
               show more embeddings than it has text. -->
          {@render corpusStage(
            'file-text',
            'home.corpus.text',
            'home.corpus.meta.text',
            stats ? stats.text : null,
            stats ? stats.textUniverse : null,
            true
          )}
          {@render corpusStage(
            'nodes',
            'home.corpus.embeddings',
            'home.corpus.meta.embeddings',
            stats ? stats.embeddings : null,
            stats ? stats.text : null,
            true
          )}
        </div>
        {#if stats && snapshot && !snapshot.isFirstRun && (stats.pendingOcr > 0 || stats.pendingEmbeddings > 0)}
          <div class="home-view__corpus-footer">
            {#if stats.pendingOcr > 0}
              <button
                type="button"
                class="home-view__corpus-pending"
                onclick={() => openBatchTab()}
              >
                <span class="home-view__corpus-dot" aria-hidden="true"></span>
                {$currentLocale && t('home.corpus.pendingOcr', { count: stats.pendingOcr })}
              </button>
            {/if}
            {#if stats.pendingEmbeddings > 0}
              <button
                type="button"
                class="home-view__corpus-pending"
                onclick={() => openBatchTab()}
              >
                <span class="home-view__corpus-dot" aria-hidden="true"></span>
                {$currentLocale &&
                  t('home.corpus.pendingEmbeddings', { count: stats.pendingEmbeddings })}
              </button>
            {/if}
          </div>
        {/if}
      {/if}
    </section>
  </div>

  <section class="home-view__quick-access" aria-labelledby="home-quick-access-title-{paneId}">
    <span id="home-quick-access-title-{paneId}" class="home-view__section-label"
      >{$currentLocale && t('home.quickAccess.title')}</span
    >
    {#snippet quickAccessCard(
      icon: ActionIconName,
      titleKey: I18nKey,
      subtitleKey: I18nKey,
      onclick: () => void
    )}
      <button type="button" class="home-view__quick-access-card" {onclick}>
        <ActionIcon name={icon} size={24} />
        <span class="home-view__quick-access-copy">
          <span class="home-view__quick-access-title">{$currentLocale && t(titleKey)}</span>
          <span class="home-view__quick-access-subtitle">{$currentLocale && t(subtitleKey)}</span>
        </span>
        <span class="home-view__quick-access-arrow" aria-hidden="true">→</span>
      </button>
    {/snippet}

    <div class="home-view__quick-access-grid">
      {@render quickAccessCard(
        'folder',
        'nav.collections',
        'home.quickAccess.collections.subtitle',
        openCollections
      )}
      {@render quickAccessCard(
        'message-circle',
        'nav.ragChat',
        'home.quickAccess.chat.subtitle',
        openChat
      )}
      {@render quickAccessCard(
        'research',
        'nav.research',
        'home.quickAccess.research.subtitle',
        openNewResearch
      )}
      {@render quickAccessCard(
        'edit',
        'writing.title',
        'home.quickAccess.writing.subtitle',
        openWritingList
      )}
    </div>
  </section>

  {#if snapshot && !snapshot.isFirstRun && (activityEntries.length > 0 || activityHasError)}
    <section class="home-view__activity" aria-labelledby="home-activity-title-{paneId}">
      <span id="home-activity-title-{paneId}" class="home-view__section-label"
        >{$currentLocale && t('home.activity.title')}</span
      >
      {#if activityHasError}
        <div class="home-panel home-view__recent">
          <p class="surface-message surface-message--error home-view__panel-error" role="alert">
            {snapshot.errors.activity}
          </p>
        </div>
      {:else}
        <div
          class="home-panel home-view__recent"
          role="table"
          aria-labelledby="home-activity-title-{paneId}"
        >
          <div class="home-view__recent-row home-view__recent-row--header" role="row">
            <span role="columnheader">{$currentLocale && t('home.activity.columnDocument')}</span>
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
              <span role="cell" class="home-view__recent-name">
                <ActionIcon name="file-text" size={14} />
                <span class="home-view__recent-name-text">{entry.title}</span>
              </span>
              <span role="cell">{entry.collectionName}</span>
              <span role="cell" class="home-view__recent-cell--end">{activityDateLabel(entry)}</span
              >
            </div>
          {/each}
        </div>
      {/if}
    </section>
  {/if}

  {#if showImportDialog}
    <ImportSourcesDialog onClose={closeImportDialog} />
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
    /* Three labeled buttons never fit alongside the title AND stay
       unbroken at the split floor (320px pane): `.page-header`'s own
       `flex-wrap` only decides whether this group shares a line with the
       title, never whether the group's own children wrap among
       themselves. Without this, the group overflowed the pane on its own
       (final visual check, split view). */
    flex-wrap: wrap;
    align-items: flex-end;
    gap: var(--space-2);
    flex-shrink: 0;
    /* Unshrinkable, the group sits at its one-line width; capped at the
       header's width, the wrap above can actually fire in a 320px pane. */
    max-width: 100%;
  }

  /* Content-sized on purpose (T3i): a fixed height clipped the Embeddings
     row once the corpus panel grew to six indicator lines. Grid items
     stretch to the row's own height by default, so Continuar still matches
     the taller Estado del corpus panel without a hardcoded number. */
  .home-view__top-row {
    display: grid;
    grid-template-columns: 3fr 2fr;
    gap: var(--space-4);
  }

  .home-view__top-row--grow {
    height: auto;
    flex: 1;
  }

  /* Keyed off this pane's own rendered width (WorkPane.svelte's `pane`
     container), not the window: a split pane is rarely the window's width,
     so a plain `@media` query never fired there and the two-column row just
     overflowed instead of stacking (columns-adapt-to-pane-width fix). Same
     720px breakpoint as before, so a single pane — as wide as the window —
     behaves exactly as it did. */
  @container pane (max-width: 720px) {
    .home-view__top-row {
      grid-template-columns: 1fr;
      height: auto;
    }
  }

  /* The shared sticky page header is opaque; on Inicio it lets the animated
     constellation through, at the panels' 78 %. */
  .home-view__header {
    background: color-mix(in srgb, var(--surface-app) 78%, transparent);
  }

  .home-panel {
    display: flex;
    flex-direction: column;
    min-height: 0;
    /* A grid item's min-width defaults to its own content's min-content
       size, refusing to shrink below it — this kept each panel (a grid
       item of `.home-view__top-row`) from shrinking to the single-column
       track the `pane` container query already switches to below 720px,
       overflowing the pane instead of reflowing (final visual check,
       split view). */
    min-width: 0;
    /* 78 % over the animated constellation, as in the approved design. */
    background: color-mix(in srgb, var(--color-surface-raised) 78%, transparent);
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

  /* Content-sized on purpose (T3k): the row used to stretch (flex: 1) to
     fill whatever height the taller Estado del corpus panel imposed on the
     grid row, reading as a ~100px oversized button. Rows now keep their own
     compact height and stack at the top; the list's own flex: 1 still
     absorbs the spare height, left empty below the last row. */
  .home-view__continuar-item {
    flex: 0 0 auto;
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
    padding: var(--space-3) var(--space-4);
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

  /* ─── Estado del corpus: OCR/STT -> Texto -> Embeddings pipeline ─── */
  /* Overrides .home-panel's overflow: hidden (used elsewhere for rounded
     corners): with six content-sized indicator lines, this panel must never
     clip a real row (T3i) — nothing here overflows its own bounds. */
  .home-view__corpus {
    overflow: visible;
  }

  .home-view__corpus-grid {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: var(--space-5);
    padding: var(--space-4);
    flex: 1;
  }

  /* Colecciones / Documentos: a bordered icon square, a big number and a
     muted label — the corpus totals, not a pipeline stage. */
  .home-view__corpus-top {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
  }

  .home-view__corpus-cell {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }

  .home-view__corpus-cell-icon {
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

  .home-view__corpus-cell-copy {
    display: flex;
    flex-direction: column;
  }

  .home-view__corpus-cell-value {
    font-family: var(--font-reading);
    font-size: 22px;
    font-weight: var(--font-weight-medium);
    font-variant-numeric: tabular-nums;
  }

  .home-view__corpus-cell-label {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  /* OCR / STT sit side by side; Texto and Embeddings each take the full
     width — the pipeline reads top to bottom, general to derived. */
  .home-view__corpus-pair {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-3);
  }

  .home-view__corpus-stage {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    min-width: 0;
  }

  .home-view__corpus-stage-icon {
    display: flex;
    align-items: center;
    flex: 0 0 auto;
    height: 20px;
    color: var(--color-text-muted);
  }

  .home-view__corpus-stage-body {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    flex: 1;
  }

  .home-view__corpus-stage-head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .home-view__corpus-stage-label {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
  }

  .home-view__corpus-stage-value {
    font-family: var(--font-reading);
    font-size: 15px;
    font-variant-numeric: tabular-nums;
  }

  .home-view__corpus-stage--full .home-view__corpus-stage-value {
    font-size: 17px;
  }

  .home-view__corpus-stage-meta {
    font-size: var(--font-size-2xs);
    color: var(--color-text-muted);
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

  /* ─── Acceso rápido ─── */
  .home-view__quick-access,
  .home-view__activity {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .home-view__quick-access-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--space-3);
  }

  /* Four fixed 1fr tracks never yielded on their own: each card's own
     content (icon + two-line copy) set a floor the track could not shrink
     below, so at a split pane's narrower widths the row overflowed instead
     of reflowing. Keyed to this pane's own width via the `pane` container
     (WorkPane.svelte), not the viewport, since a split pane is rarely the
     window's width (columns-adapt-to-pane-width fix). */
  @container pane (max-width: 680px) {
    .home-view__quick-access-grid {
      grid-template-columns: repeat(2, 1fr);
    }
  }

  @container pane (max-width: 380px) {
    .home-view__quick-access-grid {
      grid-template-columns: 1fr;
    }
  }

  /* Icon, copy and arrow in one row so the content is vertically centered
     inside a comfortably tall card, with real breathing room on every side
     (T3h — the previous 64px/16px box crowded the icon against the edges). */
  .home-view__quick-access-card {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    /* A grid item's min-width defaults to its own content's min-content
       size, refusing to shrink below it — this kept the row from actually
       reaching the 2- or 1-column tracks the `pane` container query below
       switches to, overflowing the pane instead of reflowing (final visual
       check, split view). */
    min-width: 0;
    min-height: 72px;
    padding: var(--space-4) var(--space-5);
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

  .home-view__quick-access-copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    flex: 1;
    min-width: 0;
  }

  /* Muted-but-readable at rest, not dimmed by an extra opacity on top of an
     already-muted token (T3k): the previous opacity: 0.55 made the arrow
     nearly invisible. It brightens to secondary text on hover/focus. */
  .home-view__quick-access-arrow {
    flex: 0 0 auto;
    font-size: var(--font-size-lg);
    line-height: 1;
    color: var(--color-text-muted);
    transition: color var(--transition-smooth);
  }

  .home-view__quick-access-card:hover .home-view__quick-access-arrow,
  .home-view__quick-access-card:focus-visible .home-view__quick-access-arrow {
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
  /* The activity section is the page column's flex child now (its title sits
     above the table), so it carries the grow the table used to have. */
  .home-view__activity {
    flex: 1;
  }

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

  /* The two fixed columns (200px + 130px) left too little of a narrow split
     pane for the document name — the name column would clip or the row
     would overflow past the pane. The collection column (always the row's
     2nd cell, header included) drops first; date stays, since "when" reads
     as more useful than "where" once space is this tight. Keyed to this
     pane's own width via the `pane` container (WorkPane.svelte), not the
     viewport (columns-adapt-to-pane-width fix). */
  @container pane (max-width: 560px) {
    .home-view__recent-row {
      grid-template-columns: minmax(0, 1fr) auto;
    }

    .home-view__recent-row > :nth-child(2) {
      display: none;
    }
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
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    color: var(--color-text-muted);
  }

  .home-view__recent-name-text {
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    color: var(--color-text-primary);
  }

  .home-view__recent-cell--end {
    text-align: right;
  }
</style>
