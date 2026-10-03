<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import { ActionIcon, Button, IconButton, SearchBar } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import { writingZotero, type LibraryEntry, type ZoteroState } from '$lib/writing-zotero'
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

  function nameOf(option: LibraryOption): string {
    const base = libraryLabel(option, t('writing.zoteroLibraryPersonal'))
    return option.unverified ? `${base} ${t('writing.zoteroLibraryUnverified')}` : base
  }

  function chooseLibrary(event: Event) {
    const value = (event.currentTarget as HTMLSelectElement).value
    const slash = value.indexOf('/')
    if (slash < 0) return
    const type = value.slice(0, slash)
    const id = value.slice(slash + 1)
    if ((type !== 'user' && type !== 'group') || !id) return
    store.select(type, id)
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
      <label class="zotero__library-label" for="zotero-library">
        {t('writing.zoteroLibrary')}
      </label>
      <!-- Keyed on the offered set: the list arrives after the mount, and a
        select keeps the value it was mounted with when its options change
        underneath it. Remounting applies the store selection together with
        the full options, so a selection outside the initial personal-only
        option still shows. Reloads happen on mount and after an add, never
        mid-interaction with the select itself. -->
      {#key libraries.map((option) => `${option.libraryType}/${option.libraryId}`).join(',')}
        <select
          id="zotero-library"
          class="zotero__select"
          value={`${snapshot.selection.libraryType}/${snapshot.selection.libraryId}`}
          onchange={chooseLibrary}
        >
          {#each libraries as option (`${option.libraryType}/${option.libraryId}`)}
            <option value={`${option.libraryType}/${option.libraryId}`}>
              {nameOf(option)}
            </option>
          {/each}
        </select>
      {/key}
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
        <label class="zotero__add-label" for="zotero-add-type">
          {t('writing.zoteroLibraryType')}
        </label>
        <select
          id="zotero-add-type"
          class="zotero__select"
          value={addType}
          onchange={(event) => {
            const next = (event.currentTarget as HTMLSelectElement).value
            if (next === 'user' || next === 'group') addType = next
          }}
        >
          <option value="user">{t('writing.zoteroLibraryTypeUser')}</option>
          <option value="group">{t('writing.zoteroLibraryTypeGroup')}</option>
        </select>
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
        {:else if snapshot.bibliographySync.requested}
          <p class="zotero__notice" role="status">
            {t('writing.zoteroBibliographySyncRequested')}
          </p>
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
                </span>
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
                  disabled={!oncite}
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

  .zotero__select,
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

  .zotero__select {
    flex: 1 1 10rem;
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
