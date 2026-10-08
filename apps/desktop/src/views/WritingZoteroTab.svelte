<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import {
    ActionIcon,
    Button,
    IconButton,
    SearchBar,
    ToolbarMenu,
    type ToolbarMenuItem,
  } from '@entropia/ui'
  import { locale, t } from '$lib/i18n'
  import {
    bibliographyDerivedProgress,
    formatEtaMs,
    writingZotero,
    type LibraryEntry,
    type ZoteroState,
  } from '$lib/writing-zotero'
  import {
    addLibraryChecked,
    libraryLabel,
    loadLibraries,
    type LibraryOption,
  } from '$lib/writing-zotero-libraries'
  import WritingCitationEditor, {
    type CitationDraft,
    type CitationEditSession,
  } from './WritingCitationEditor.svelte'
  import WritingZoteroDetails from './WritingZoteroDetails.svelte'
  import SearchFuzzyToggle from '../components/SearchFuzzyToggle.svelte'
  import SearchMatchLine from '../components/SearchMatchLine.svelte'

  /**
   * The Zotero tab of the research panel (plan-editor.md §6.3, §11).
   *
   * The state line above the list is the point of §11.3, not decoration. Every
   * sentence it can show is something that was observed: nothing here says
   * Zotero is closed or not installed, because nothing the app can see
   * distinguishes a closed program from a blocked port — and sending someone to
   * reinstall a program that is running is worse than telling them plainly that
   * the port did not answer.
   *
   * A library that could not be read is never shown as an empty one. Those are
   * different answers to someone hunting a reference.
   */

  interface Props {
    /** Inserts a Zotero citation at the caret. Absent when no manuscript is open. */
    oncite?: (attrs: Record<string, unknown>) => string | null
    citationDraft?: CitationEditSession | null
    oncitationdraftchange?: (draft: CitationDraft) => void
    onapplycitation?: (attrs: Record<string, unknown>) => void
    oncancelcitation?: () => void
  }

  let {
    oncite,
    citationDraft = null,
    oncitationdraftchange,
    onapplycitation,
    oncancelcitation,
  }: Props = $props()

  const store = writingZotero
  let snapshot = $state(store.snapshot)
  const unsubscribe = store.subscribe((value) => {
    snapshot = value
  })

  let cited = $state(false)

  /**
   * The work whose ficha is open (E1c-3). Local to the tab: opening it
   * selects nothing in the store, cites nothing and leaves the offered
   * library list exactly as it was.
   */
  let detailsEntry = $state<LibraryEntry | null>(null)

  /**
   * The libraries offered (E1c-2): backend-known merged with hand-added.
   * Personal only until the merge answers, so the untouched tab reads exactly
   * what it always read. Selection itself lives in the store (E1c-1).
   */
  let libraries = $state<LibraryOption[]>([
    { libraryType: 'user', libraryId: '0', name: null, source: 'personal', unverified: false },
  ])
  let showAdd = $state(false)
  let addType = $state<'user' | 'group'>('group')
  let addId = $state('')
  let addError = $state<string | null>(null)
  let adding = $state(false)
  let libraryMenuOpen = $state(false)
  let typeMenuOpen = $state(false)

  const currentLocale = locale

  function nameOf(option: LibraryOption): string {
    const base = libraryLabel(option, t('writing.zoteroLibraryPersonal'))
    return option.unverified ? `${base} ${t('writing.zoteroLibraryUnverified')}` : base
  }

  /**
   * The name the picker shows: the offered name of the selected library, or
   * its raw id while the offered list has not caught up with the selection.
   */
  const libraryHeading = $derived.by(() => {
    $currentLocale
    const picked = libraries.find(
      (option) =>
        option.libraryType === snapshot.selection.libraryType &&
        option.libraryId === snapshot.selection.libraryId
    )
    return picked
      ? nameOf(picked)
      : `${snapshot.selection.libraryType}/${snapshot.selection.libraryId}`
  })

  /** The picker: one radio entry per offered library, checked = selected. */
  const libraryItems = $derived.by<ToolbarMenuItem[]>(() => {
    $currentLocale
    return libraries.map((option) => ({
      kind: 'radio' as const,
      id: `${option.libraryType}/${option.libraryId}`,
      label: nameOf(option),
      checked:
        option.libraryType === snapshot.selection.libraryType &&
        option.libraryId === snapshot.selection.libraryId,
      onselect: () => chooseLibrary(option),
    }))
  })

  const addTypeLabel = $derived.by(() => {
    $currentLocale
    return addType === 'user'
      ? t('writing.zoteroLibraryTypeUser')
      : t('writing.zoteroLibraryTypeGroup')
  })

  const addTypeItems = $derived.by<ToolbarMenuItem[]>(() => {
    $currentLocale
    return (['user', 'group'] as const).map((type) => ({
      kind: 'radio' as const,
      id: type,
      label:
        type === 'user' ? t('writing.zoteroLibraryTypeUser') : t('writing.zoteroLibraryTypeGroup'),
      checked: addType === type,
      onselect: () => {
        typeMenuOpen = false
        addType = type
      },
    }))
  })

  function chooseLibrary(option: LibraryOption): void {
    libraryMenuOpen = false
    store.select(option.libraryType, option.libraryId)
    void store.connect()
  }

  function addErrorText(code: 'invalid' | 'not_found' | 'check_failed'): string {
    switch (code) {
      case 'not_found':
        return t('writing.zoteroLibraryNotFound')
      case 'invalid':
        return t('writing.zoteroLibraryInvalid')
      default:
        return t('writing.zoteroLibraryCheckFailed')
    }
  }

  async function submitAdd(event: Event) {
    event.preventDefault()
    if (adding) return
    const id = addId.trim()
    if (!id) {
      addError = t('writing.zoteroLibraryInvalid')
      return
    }
    adding = true
    addError = null
    try {
      const outcome = await addLibraryChecked(addType, id)
      if (!outcome.ok) {
        addError = addErrorText(outcome.error)
        return
      }
      libraries = outcome.libraries
      addId = ''
      showAdd = false
      store.select(outcome.selected.libraryType, outcome.selected.libraryId)
      void store.connect()
    } finally {
      adding = false
    }
  }

  onMount(() => {
    // Opening the tab is the request: the copy on disk is listed at once, and
    // Zotero is asked only what changed since.
    void store.connect()
    void loadLibraries().then((list) => {
      libraries = list
    })
  })

  onDestroy(unsubscribe)

  /** The sentence for a state, with nothing added that was not observed. */
  function say(status: ZoteroState): string {
    switch (status.state) {
      case 'available':
        return t('writing.zoteroStateAvailable')
      case 'api_disabled':
        return t('writing.zoteroStateDisabled')
      case 'timeout':
        return t('writing.zoteroStateTimeout')
      case 'invalid_response':
        return t('writing.zoteroStateInvalid', { detail: status.detail })
      default:
        return t('writing.zoteroStateUnavailable')
    }
  }

  /**
   * What the requested sync is really doing, in the scheduler's own terms.
   * Null when nothing was requested yet. Never claims background work that
   * is not happening: a paused or failed task says so, with its reason.
   */
  function syncReport(
    progress: typeof snapshot.bibliographyProgress
  ): { text: string; error: boolean } | null {
    if (!progress) return null
    if (!progress.status) {
      return {
        text: t('writing.zoteroBibliographySyncUnreadable', {
          detail: progress.unreadable ?? '',
        }),
        error: true,
      }
    }
    const status = progress.status
    const detail = status.errorMessage ?? status.errorCode ?? ''
    switch (status.state) {
      case 'pending':
        return { text: t('writing.zoteroBibliographySyncQueued'), error: false }
      case 'running':
        return {
          text: status.progressTotal
            ? t('writing.zoteroBibliographySyncRunningOf', {
                done: String(status.progressDone),
                total: String(status.progressTotal),
              })
            : t('writing.zoteroBibliographySyncRunning'),
          error: false,
        }
      case 'retry_wait':
      case 'interrupted':
        return {
          text:
            status.errorCode === 'zotero_unreachable' || status.errorCode === 'zotero_timeout'
              ? t('writing.zoteroBibliographySyncWaiting')
              : t('writing.zoteroBibliographySyncPaused', { detail }),
          error: false,
        }
      case 'blocked':
        return {
          text:
            status.errorCode === 'zotero_api_disabled'
              ? t('writing.zoteroBibliographySyncApiDisabled')
              : t('writing.zoteroBibliographySyncPaused', { detail }),
          error: true,
        }
      case 'succeeded': {
        // The sync's own pages are done; its derived work (fichas and
        // pasajes) may not be. While it runs, the line reports that work
        // with its real counts and honest estimate — never a finished-sync
        // claim over a backlog that is still moving.
        const derived = bibliographyDerivedProgress(status)
        if (derived.remaining > 0) {
          const counts = {
            worksDone: derived.worksDone,
            worksTotal: derived.worksTotal,
            passagesDone: derived.passagesDone,
            passagesTotal: derived.passagesTotal,
          }
          if (derived.blocked > 0) {
            const waiting = {
              ...counts,
              blocked: derived.blocked,
              reason: derived.blockedReason,
            }
            // Parked work that cannot move is an attention line: the counts
            // stop being progress and start naming what only the owner can
            // do about it. While other work still moves, the same note is a
            // plain notice beside the estimate of the actionable part.
            return derived.remainingActive === 0
              ? { text: t('writing.zoteroBibliographySyncBlockedBacklog', waiting), error: true }
              : {
                  text:
                    derived.etaMs == null
                      ? t('writing.zoteroBibliographySyncBlockedBacklog', waiting)
                      : t('writing.zoteroBibliographySyncIndexingWaiting', {
                          ...waiting,
                          eta: formatEtaMs(derived.etaMs),
                        }),
                  error: false,
                }
          }
          return {
            text:
              derived.etaMs == null
                ? t('writing.zoteroBibliographySyncIndexingNoEta', counts)
                : t('writing.zoteroBibliographySyncIndexing', {
                    ...counts,
                    eta: formatEtaMs(derived.etaMs),
                  }),
            error: false,
          }
        }
        return {
          text:
            status.newProfiles + status.newExtractions === 0
              ? t('writing.zoteroBibliographySyncUpToDate')
              : t('writing.zoteroBibliographySyncDone', {
                  works: String(status.newProfiles),
                  attachments: String(status.newExtractions),
                }),
          error: false,
        }
      }
      case 'cancelled':
        return { text: t('writing.zoteroBibliographySyncCancelled'), error: false }
      default:
        return { text: t('writing.zoteroBibliographySyncFailed', { detail }), error: true }
    }
  }

  const syncProgress = $derived(syncReport(snapshot.bibliographyProgress ?? null))
  const syncActive = $derived(
    ['pending', 'running'].includes(snapshot.bibliographyProgress?.status?.state ?? '')
  )

  /**
   * The derived unit running right now, said out loud. A book of 1500 pages
   * OCRs page by page for hours while the counts line stands still; this
   * second line names the work and its page so nothing looks stuck.
   */
  const currentLine = $derived.by(() => {
    $currentLocale
    const status = snapshot.bibliographyProgress?.status ?? null
    const current = status?.current ?? null
    if (!current) return null
    const etaMs = status?.etaMs ?? null
    const eta = etaMs == null ? null : formatEtaMs(etaMs)
    if (current.pagesTotal > 0) {
      return eta == null
        ? t('writing.zoteroBibliographyCurrentPages', {
            title: current.title,
            done: current.pagesDone,
            total: current.pagesTotal,
          })
        : t('writing.zoteroBibliographyCurrentPagesEta', {
            title: current.title,
            done: current.pagesDone,
            total: current.pagesTotal,
            eta,
          })
    }
    return eta == null
      ? t('writing.zoteroBibliographyCurrent', { title: current.title })
      : t('writing.zoteroBibliographyCurrentEta', { title: current.title, eta })
  })

  /**
   * While the derived backlog drains, another press of «Sincronizar
   * biblioteca» only re-reads the Zotero catalog — it does not restart the
   * processing in course — and the button says so where the press happens.
   */
  const backlogDraining = $derived.by(() => {
    const status = snapshot.bibliographyProgress?.status ?? null
    return status != null && bibliographyDerivedProgress(status).remaining > 0
  })
  const syncRereadTitle = $derived.by(() => {
    $currentLocale
    return t('writing.zoteroBibliographySyncReread')
  })
  const zoteroReachable = $derived(snapshot.status?.state === 'available')

  function cite(entry: (typeof snapshot.entries)[number]) {
    if (!oncite) return
    cited =
      oncite({
        sourceOrigin: 'local',
        sourceInstanceId: null,
        itemKey: entry.key,
        itemVersion: entry.itemVersion,
        libraryType: entry.libraryType,
        libraryId: entry.libraryId,
        metadataSnapshot: entry.csl_json,
      }) !== null
  }
</script>

<div class="zotero">
  {#if citationDraft}
    {#key citationDraft.id}
      <WritingCitationEditor
        items={citationDraft.items}
        affixes={citationDraft.affixes}
        ondraftchange={oncitationdraftchange}
        onapply={(attrs) => onapplycitation?.(attrs)}
        onclose={() => oncancelcitation?.()}
      />
    {/key}
  {:else}
    <div class="zotero__library">
      <span class="zotero__library-label" id="zotero-library-label">
        {t('writing.zoteroLibrary')}
      </span>
      <!-- The same radio menu BibliotecaView paints for its library picker:
        the project rule forbids a native select. The checked entry and the
        trigger name derive from the store selection live, so — unlike the
        select this replaces — nothing is remounted when the offered list
        arrives after the mount. -->
      <ToolbarMenu
        label={t('writing.zoteroLibrary')}
        items={libraryItems}
        bind:open={libraryMenuOpen}
      >
        {#snippet trigger(props, { open })}
          <button
            type="button"
            class="zotero__menu-trigger"
            class:zotero__menu-trigger--open={open}
            aria-labelledby="zotero-library-label zotero-library-value"
            {...props}
          >
            <span class="zotero__menu-trigger-label" id="zotero-library-value">
              {libraryHeading}
            </span>
            <ActionIcon name="chevron-down" size={12} />
          </button>
        {/snippet}
      </ToolbarMenu>
      <Button
        variant="ghost"
        size="sm"
        onclick={() => {
          showAdd = !showAdd
          addError = null
        }}
      >
        <ActionIcon name="add" size={14} />
        {t('writing.zoteroAddLibrary')}
      </Button>
    </div>

    {#if showAdd}
      <form class="zotero__add" onsubmit={submitAdd}>
        <span class="zotero__add-label" id="zotero-add-type-label">
          {t('writing.zoteroLibraryType')}
        </span>
        <ToolbarMenu
          label={t('writing.zoteroLibraryType')}
          items={addTypeItems}
          bind:open={typeMenuOpen}
        >
          {#snippet trigger(props, { open })}
            <button
              type="button"
              class="zotero__menu-trigger"
              class:zotero__menu-trigger--open={open}
              aria-labelledby="zotero-add-type-label zotero-add-type-value"
              {...props}
            >
              <span class="zotero__menu-trigger-label" id="zotero-add-type-value">
                {addTypeLabel}
              </span>
              <ActionIcon name="chevron-down" size={12} />
            </button>
          {/snippet}
        </ToolbarMenu>
        <label class="zotero__add-label" for="zotero-add-id">
          {t('writing.zoteroLibraryId')}
        </label>
        <input
          id="zotero-add-id"
          class="zotero__input"
          type="text"
          inputmode="numeric"
          autocomplete="off"
          placeholder={t('writing.zoteroLibraryIdPlaceholder')}
          bind:value={addId}
        />
        <div class="zotero__add-actions">
          <Button variant="secondary" size="sm" type="submit" loading={adding}>
            {t('writing.zoteroLibraryAdd')}
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onclick={() => {
              showAdd = false
              addError = null
            }}
          >
            {t('writing.zoteroLibraryCancel')}
          </Button>
        </div>
        {#if addError}
          <p class="zotero__error" role="alert">{addError}</p>
        {/if}
      </form>
    {/if}

    <div class="zotero__bibliography-sync">
      <p class="zotero__bibliography-sync-help">
        {t('writing.zoteroBibliographySyncHelp')}
      </p>
      <div class="zotero__bibliography-sync-action">
        <Button
          variant="primary"
          size="sm"
          loading={snapshot.bibliographySync.loading}
          disabled={!zoteroReachable || syncActive}
          title={backlogDraining ? syncRereadTitle : undefined}
          onclick={() => void store.requestBibliographySync()}
        >
          {t('writing.zoteroBibliographySync')}
        </Button>
        {#if snapshot.bibliographySync.loading}
          <p class="zotero__notice" role="status">
            {t('writing.zoteroBibliographySyncRequesting')}
          </p>
        {:else if snapshot.bibliographySync.error}
          <p class="zotero__error" role="alert">
            {t('writing.zoteroBibliographySyncError', {
              detail: snapshot.bibliographySync.error,
            })}
          </p>
        {:else if syncProgress}
          {#if syncProgress.error}
            <p class="zotero__error" role="alert">{syncProgress.text}</p>
          {:else}
            <p class="zotero__notice" role="status">{syncProgress.text}</p>
          {/if}
        {:else if snapshot.bibliographySync.requested}
          <p class="zotero__notice" role="status">
            {t('writing.zoteroBibliographySyncRequested')}
          </p>
        {:else if !zoteroReachable}
          <p class="zotero__notice">{t('writing.zoteroBibliographySyncNeedsZotero')}</p>
        {/if}
        {#if currentLine}
          <!-- The second line, beside the status it qualifies: the work the
             scheduler is running and the page it is on. -->
          <p class="zotero__notice" role="status">{currentLine}</p>
        {/if}
      </div>
    </div>

    {#if snapshot.status}
      <!-- Observed, never inferred. §11.3 forbids claiming Zotero is closed or
         absent without evidence, and this line is where that promise is kept
         or broken. -->
      <p
        class="zotero__state"
        class:zotero__state--ok={snapshot.status.state === 'available'}
        role="status"
      >
        {say(snapshot.status)}
      </p>
    {/if}

    {#if snapshot.error}
      <p class="zotero__error" role="alert">{snapshot.error}</p>
    {/if}

    {#if detailsEntry}
      <!-- The ficha reads one work; the list underneath stays exactly as it
         was. Opening it selects nothing and cites nothing. -->
      <WritingZoteroDetails entry={detailsEntry} onclose={() => (detailsEntry = null)} />
    {:else}
      <div class="zotero__actions">
        <Button
          variant="secondary"
          size="sm"
          disabled={snapshot.loading || snapshot.status?.state !== 'available'}
          onclick={() => store.sync()}
        >
          <ActionIcon name="refresh" size={14} />
          {t('writing.zoteroReload')}
        </Button>
        {#if snapshot.loading}
          <p class="zotero__notice" role="status">
            {t(snapshot.loaded > 0 ? 'writing.zoteroSyncing' : 'writing.zoteroLoading')}
          </p>
        {:else if snapshot.loaded > 0}
          <p class="zotero__notice">
            {t('writing.zoteroLoaded', { count: String(snapshot.loaded) })}
          </p>
        {/if}
      </div>

      {#if snapshot.loaded > 0}
        <SearchBar
          value={snapshot.query}
          debounceMs={350}
          ariaLabel={t('writing.zoteroSearch')}
          placeholder={t('writing.zoteroSearch')}
          onvaluechange={(query) => store.search(query)}
          onsearch={(query) => void store.searchLibrary(query)}
          emitSearch={true}
        />
        <!-- One switch for every search in the app (search-preferences.ts). -->
        <SearchFuzzyToggle
          checked={snapshot.fuzzy}
          onchange={(checked) => void store.setFuzzy(checked)}
        />
      {/if}

      {#if snapshot.query.trim() && snapshot.semanticStatus !== 'idle' && snapshot.semanticStatus !== 'ok'}
        <!-- Said, never implied: a missing meaning search is not "no similar works". -->
        <p class="zotero__notice" role="status">
          {t(
            snapshot.semanticStatus === 'not_synced'
              ? 'writing.zoteroSemanticNotSynced'
              : snapshot.semanticStatus === 'lexical_only'
                ? 'writing.zoteroSemanticLexicalOnly'
                : 'writing.zoteroSemanticFailed'
          )}
        </p>
      {/if}

      {#if snapshot.entries.length > 0}
        <ul class="zotero__list">
          <!-- Keyed by the whole item, not its citation key: two works can share
           a key, and a keyed list with a repeated key does not render. -->
          {#each snapshot.entries as entry (entry.csl_json)}
            <li class="zotero__row">
              <span class="zotero__work">
                <span class="zotero__title">{entry.title}</span>
                <span class="zotero__meta">
                  {[entry.authors, entry.year].filter(Boolean).join(' · ')}
                  {#if entry.semantic}
                    <span class="zotero__semantic">{t('writing.zoteroSemanticTag')}</span>
                  {/if}
                  {#if entry.content}
                    <span class="zotero__semantic">{t('writing.zoteroContentTag')}</span>
                  {/if}
                </span>
                {#if entry.content}
                  <SearchMatchLine kind={entry.content.kind} terms={entry.content.terms} />
                {/if}
              </span>
              <span class="zotero__row-actions">
                <IconButton
                  size="sm"
                  label={t('writing.zoteroDetails')}
                  onclick={() => (detailsEntry = entry)}
                >
                  <ActionIcon name="eye" size={14} />
                </IconButton>
                <IconButton
                  size="sm"
                  label={t('writing.zoteroCite')}
                  disabled={!oncite || !entry.csl_json.trim()}
                  onclick={() => cite(entry)}
                >
                  <ActionIcon name="text-quote" size={14} />
                </IconButton>
              </span>
            </li>
          {/each}
        </ul>
      {:else if snapshot.query.trim() && snapshot.loaded > 0}
        <p class="zotero__notice">{t('writing.zoteroEmpty')}</p>
      {:else if snapshot.loaded === 0 && !snapshot.loading && !snapshot.error}
        <p class="zotero__notice">{t('writing.zoteroStart')}</p>
      {/if}
    {/if}

    <p class="zotero__notice" role="status">
      {#if cited}
        {t('writing.zoteroCited')}
      {:else if !oncite && snapshot.loaded > 0}
        {t('writing.zoteroNoDocument')}
      {/if}
    </p>
  {/if}
</div>

<style>
  .zotero {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 0;
  }

  .zotero__state {
    margin: 0;
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--surface-input);
    color: var(--color-text-secondary);
    font-size: var(--font-size-2xs);
    line-height: var(--line-height-base);
  }

  .zotero__state--ok {
    color: var(--color-text-muted);
  }

  .zotero__actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .zotero__bibliography-sync {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--color-accent-faint);
  }

  .zotero__bibliography-sync-help {
    flex: 1 1 14rem;
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-2xs);
    line-height: var(--line-height-base);
  }

  .zotero__bibliography-sync-action {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-2);
    flex: 1 1 12rem;
    flex-wrap: wrap;
  }

  .zotero__library {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .zotero__library-label,
  .zotero__add-label {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .zotero__input {
    min-height: var(--control-height-sm);
    padding: 0 var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-control);
    background: var(--surface-input);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    max-width: 100%;
  }

  /* The picker trigger wears this tab's own control look (the select it
     replaced dressed exactly like the input above), shaped like the library
     pickers of BibliotecaView and BatchProcessingTab: label, name, chevron. */
  .zotero__menu-trigger {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    max-width: 260px;
    min-height: var(--control-height-sm);
    padding: 0 var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-control);
    background: var(--surface-input);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    cursor: pointer;
  }

  .zotero__menu-trigger:hover,
  .zotero__menu-trigger--open {
    border-color: color-mix(in srgb, var(--color-accent) 40%, var(--border-subtle));
  }

  .zotero__menu-trigger:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .zotero__menu-trigger-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .zotero__add {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }

  .zotero__add-actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }

  .zotero__list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
    overflow-y: auto;
  }

  .zotero__row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-control);
  }

  .zotero__row:hover {
    background: var(--color-accent-faint);
  }

  .zotero__row-actions {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex-shrink: 0;
  }

  .zotero__work {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .zotero__title {
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .zotero__meta {
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
  }

  .zotero__notice,
  .zotero__error {
    margin: 0;
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }

  .zotero__semantic {
    margin-left: var(--space-1);
    padding: 0 var(--space-1);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-control);
    color: var(--color-text-secondary);
  }

  .zotero__notice {
    color: var(--color-text-muted);
  }

  .zotero__error {
    color: var(--color-danger);
  }
</style>
