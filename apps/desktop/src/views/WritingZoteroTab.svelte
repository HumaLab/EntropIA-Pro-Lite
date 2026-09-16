<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import { ActionIcon, Button, SearchBar } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import { writingZotero, type ZoteroState } from '$lib/writing-zotero'

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
  }

  let { oncite }: Props = $props()

  const store = writingZotero
  let snapshot = $state(store.snapshot)
  const unsubscribe = store.subscribe((value) => {
    snapshot = value
  })

  let cited = $state(false)

  onMount(() => {
    void store.probe()
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

  function cite(csl_json: string, key: string) {
    if (!oncite) return
    cited = oncite({ itemKey: key, metadataSnapshot: csl_json }) !== null
  }
</script>

<div class="zotero">
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

  <div class="zotero__actions">
    <Button
      variant="secondary"
      size="sm"
      disabled={snapshot.loading || snapshot.status?.state !== 'available'}
      onclick={() => store.load()}
    >
      <ActionIcon name="refresh" size={14} />
      {t('writing.zoteroLoad')}
    </Button>
    {#if snapshot.loading}
      <p class="zotero__notice" role="status">{t('writing.zoteroLoading')}</p>
    {:else if snapshot.loaded > 0}
      <p class="zotero__notice">
        {t('writing.zoteroLoaded', { count: String(snapshot.loaded) })}
      </p>
    {/if}
  </div>

  {#if snapshot.hasMore}
    <!-- Said out loud rather than silently truncated: a list that stops without
         saying so implies the rest does not exist. -->
    <p class="zotero__notice">
      {t('writing.zoteroTruncated', { count: String(snapshot.loaded) })}
    </p>
  {/if}

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

  {#if snapshot.entries.length > 0}
    <ul class="zotero__list">
      {#each snapshot.entries as entry (entry.key)}
        <li class="zotero__row">
          <span class="zotero__work">
            <span class="zotero__title">{entry.title}</span>
            <span class="zotero__meta">
              {[entry.authors, entry.year].filter(Boolean).join(' · ')}
            </span>
          </span>
          <Button
            variant="ghost"
            size="sm"
            disabled={!oncite}
            onclick={() => cite(entry.csl_json, entry.key)}
          >
            {t('writing.zoteroCite')}
          </Button>
        </li>
      {/each}
    </ul>
  {:else if snapshot.query.trim() && snapshot.loaded > 0}
    <p class="zotero__notice">{t('writing.zoteroEmpty')}</p>
  {:else if snapshot.loaded === 0 && !snapshot.loading && !snapshot.error}
    <p class="zotero__notice">{t('writing.zoteroStart')}</p>
  {/if}

  <p class="zotero__notice" role="status">
    {#if cited}
      {t('writing.zoteroCited')}
    {:else if !oncite && snapshot.loaded > 0}
      {t('writing.zoteroNoDocument')}
    {/if}
  </p>
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

  .zotero__work {
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

  .zotero__notice {
    color: var(--color-text-muted);
  }

  .zotero__error {
    color: var(--color-danger);
  }
</style>
