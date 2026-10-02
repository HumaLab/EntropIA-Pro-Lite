<script module lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import type { LibraryEntry } from '$lib/writing-zotero'

  /** One Zotero creator, camelCase exactly as `writing_zotero_item_detail` answers. */
  export interface ZoteroDetailCreator {
    creatorType?: string | null
    firstName?: string | null
    lastName?: string | null
    name?: string | null
  }

  /** Attachment metadata only: names to list, never files to open. */
  export interface ZoteroDetailAttachment {
    attachmentKey: string
    contentType?: string | null
    linkMode?: string | null
    filename?: string | null
    url?: string | null
  }

  /** The last catalog metadata for one work, camelCase as answered. */
  export interface ZoteroDetailItem {
    itemKey: string
    itemType?: string | null
    title?: string | null
    creators?: ZoteroDetailCreator[] | null
    publicationTitle?: string | null
    publisher?: string | null
    date?: string | null
    doi?: string | null
    isbn?: string | null
    /** `abstract` on the wire; the one reserved-looking name kept verbatim. */
    abstract?: string | null
    language?: string | null
    url?: string | null
    itemVersion?: number | null
    collections: string[]
    tags: string[]
    attachments: ZoteroDetailAttachment[]
  }

  export interface ZoteroDetailTombstone {
    observedAt: number
    remoteVersion?: number | null
    reason: string
  }

  /**
   * What the ficha can honestly show, tagged exactly as the command answers.
   * `confirmed` and `lost_link` render from `item`; the offline states render
   * from the held entry instead — never a claim about Zotero itself.
   */
  export type ZoteroItemDetail =
    | { status: 'confirmed'; verifiedAt: number; item: ZoteroDetailItem }
    | { status: 'lost_link'; tombstone: ZoteroDetailTombstone; item: ZoteroDetailItem | null }
    | { status: 'not_in_catalog' }
    | { status: 'catalog_unavailable' }

  /**
   * The one fetch entry point: the catalog DTO for a work, by native Zotero
   * identity. CamelCase on the wire (`libraryType` / `libraryId` / `itemKey`).
   */
  export async function fetchZoteroItemDetail(
    libraryType: string,
    libraryId: string,
    itemKey: string
  ): Promise<ZoteroItemDetail> {
    return await invoke('writing_zotero_item_detail', { libraryType, libraryId, itemKey })
  }

  /**
   * The open-in-Zotero entry point (E1c-4): one select URI launch, by native
   * Zotero identity. CamelCase on the wire, like the detail fetch. Callers
   * pass the held entry's key — never a CSL id — and the backend validates
   * identity before the gate (`invalid_library` / `invalid_item_key` vs
   * `open_item_disabled`). `Ok(())` reports only that the open was launched.
   */
  export async function openZoteroItem(
    libraryType: string,
    libraryId: string,
    itemKey: string
  ): Promise<void> {
    await invoke('writing_zotero_open_item', { libraryType, libraryId, itemKey })
  }

  /** `verifiedAt` (ms epoch) as a plain calendar date, timezone-proof. */
  export function formatDetailDate(ms: number): string {
    return new Date(ms).toISOString().slice(0, 10)
  }
</script>

<script lang="ts">
  import { ActionIcon, Button } from '@entropia/ui'
  import { t } from '$lib/i18n'

  interface Props {
    /** The held list entry: identity for the fetch, CSL fallback while offline. */
    entry: LibraryEntry
    /** When given, the ficha renders presentationally and never fetches. */
    detail?: ZoteroItemDetail
    onclose?: () => void
  }

  let { entry, detail = undefined, onclose }: Props = $props()

  let remote = $state<ZoteroItemDetail | null>(null)
  let failed = $state(false)
  // Opening in Zotero never changes the ficha: success reports nothing,
  // failures surface as a message beside the button, keyed by error code.
  let opening = $state(false)
  let openError = $state<string | null>(null)

  // Fetch mode is the absence of a held DTO, not a flag: presentational
  // mounts never fetch, fetching mounts never claim a state they lack.
  const fetchMode = $derived(detail === undefined)
  const loading = $derived(fetchMode && remote === null && !failed)
  const shown = $derived(detail !== undefined ? detail : remote)
  const verifiedAt = $derived(shown?.status === 'confirmed' ? shown.verifiedAt : null)
  const confirmedItem = $derived(shown?.status === 'confirmed' ? shown.item : null)
  const tombstone = $derived(shown?.status === 'lost_link' ? shown.tombstone : null)
  const lostItem = $derived(shown?.status === 'lost_link' ? shown.item : null)
  const offline = $derived(
    shown?.status === 'not_in_catalog' || shown?.status === 'catalog_unavailable'
  )

  async function load() {
    failed = false
    try {
      remote = await fetchZoteroItemDetail(entry.libraryType, entry.libraryId, entry.key)
    } catch {
      // An invoke failure is a failed read, not a state: the retry below
      // says exactly that instead of inventing a chip.
      failed = true
    }
  }

  /** Branch on the wire `code`, never on message text. */
  function openErrorCode(error: unknown): string {
    if (typeof error === 'object' && error !== null && 'code' in error) {
      return String((error as { code: unknown }).code)
    }
    return 'unknown'
  }

  function openErrorText(code: string): string {
    switch (code) {
      case 'open_item_disabled':
        return t('writing.zoteroDetailOpenPending')
      case 'invalid_item_key':
        return t('writing.zoteroDetailOpenInvalidKey')
      case 'invalid_library':
        return t('writing.zoteroDetailOpenInvalidLibrary')
      default:
        return t('writing.zoteroDetailOpenFailed')
    }
  }

  async function openInZotero() {
    if (opening) return
    opening = true
    openError = null
    try {
      // The held entry's native key, never the CSL id and never the
      // snapshot's copy: a lost-link ficha may show stale metadata, and a
      // local copy has only this identity to attempt.
      await openZoteroItem(entry.libraryType, entry.libraryId, entry.key)
    } catch (error) {
      openError = openErrorText(openErrorCode(error))
    } finally {
      opening = false
    }
  }

  $effect(() => {
    if (fetchMode) void load()
  })

  function creatorName(creator: ZoteroDetailCreator): string {
    if (creator.name?.trim()) return creator.name.trim()
    return [creator.firstName, creator.lastName]
      .map((part) => part?.trim() ?? '')
      .filter(Boolean)
      .join(' ')
  }
</script>

{#snippet ficha(item: ZoteroDetailItem)}
  <h3 class="details__title">{item.title ?? entry.title}</h3>
  <dl class="details__fields">
    {#if item.itemType}
      <div class="details__field">
        <dt>{t('writing.zoteroDetailType')}</dt>
        <dd>{item.itemType}</dd>
      </div>
    {/if}
    {#if (item.creators ?? []).map(creatorName).filter(Boolean).length > 0}
      <div class="details__field">
        <dt>{t('writing.zoteroDetailCreators')}</dt>
        <dd>{(item.creators ?? []).map(creatorName).filter(Boolean).join(', ')}</dd>
      </div>
    {/if}
    {#if item.publicationTitle}
      <div class="details__field">
        <dt>{t('writing.zoteroDetailPublication')}</dt>
        <dd>{item.publicationTitle}</dd>
      </div>
    {/if}
    {#if item.publisher}
      <div class="details__field">
        <dt>{t('writing.zoteroDetailPublisher')}</dt>
        <dd>{item.publisher}</dd>
      </div>
    {/if}
    {#if item.date}
      <div class="details__field">
        <dt>{t('writing.zoteroDetailDate')}</dt>
        <dd>{item.date}</dd>
      </div>
    {/if}
    {#if item.doi}
      <div class="details__field">
        <dt>{t('writing.zoteroDetailDoi')}</dt>
        <dd>{item.doi}</dd>
      </div>
    {/if}
    {#if item.isbn}
      <div class="details__field">
        <dt>{t('writing.zoteroDetailIsbn')}</dt>
        <dd>{item.isbn}</dd>
      </div>
    {/if}
    {#if item.abstract}
      <div class="details__field">
        <dt>{t('writing.zoteroDetailAbstract')}</dt>
        <dd>{item.abstract}</dd>
      </div>
    {/if}
    {#if item.language}
      <div class="details__field">
        <dt>{t('writing.zoteroDetailLanguage')}</dt>
        <dd>{item.language}</dd>
      </div>
    {/if}
    {#if item.url}
      <div class="details__field">
        <dt>{t('writing.zoteroDetailUrl')}</dt>
        <dd>{item.url}</dd>
      </div>
    {/if}
    {#if item.collections.length > 0}
      <div class="details__field">
        <dt>{t('writing.zoteroDetailCollections')}</dt>
        <dd>{item.collections.join(', ')}</dd>
      </div>
    {/if}
    {#if item.tags.length > 0}
      <div class="details__field">
        <dt>{t('writing.zoteroDetailTags')}</dt>
        <dd>{item.tags.join(', ')}</dd>
      </div>
    {/if}
    {#if item.attachments.length > 0}
      <div class="details__field">
        <dt>{t('writing.zoteroDetailAttachments')}</dt>
        <dd>
          <ul class="details__attachments">
            {#each item.attachments as attachment (attachment.attachmentKey)}
              <!-- Names only: the ficha never resolves or opens a file. -->
              <li>{attachment.filename ?? attachment.attachmentKey}</li>
            {/each}
          </ul>
        </dd>
      </div>
    {/if}
  </dl>
{/snippet}

<div class="details">
  <div class="details__head">
    <Button variant="ghost" size="sm" onclick={() => onclose?.()}>
      <ActionIcon name="chevron-left" size={14} />
      {t('writing.zoteroBack')}
    </Button>
    <h2 class="details__heading">{t('writing.zoteroDetailTitle')}</h2>
  </div>

  {#if loading}
    <p class="details__notice" role="status">{t('writing.zoteroDetailLoading')}</p>
  {:else if failed}
    <p class="details__error" role="alert">{t('writing.zoteroDetailError')}</p>
    <Button variant="secondary" size="sm" onclick={() => void load()}>
      {t('writing.zoteroDetailRetry')}
    </Button>
  {:else if shown}
    <!-- Opening from any ficha is allowed: a lost link may exist again in
       Zotero, and a local copy attempts with the entry's native identity.
       Success closes nothing; failures surface below by error code. -->
    <div class="details__actions">
      <Button variant="secondary" size="sm" onclick={() => void openInZotero()} disabled={opening}>
        <ActionIcon name="external-link" size={14} />
        {t('writing.zoteroDetailOpenInZotero')}
      </Button>
    </div>
    {#if openError}
      <p class="details__error" role="alert">{openError}</p>
    {/if}
    {#if confirmedItem && verifiedAt !== null}
      <p class="details__chip details__chip--ok" role="status">
        {t('writing.zoteroDetailSynced', { date: formatDetailDate(verifiedAt) })}
      </p>
      {@render ficha(confirmedItem)}
    {:else if tombstone}
      <p class="details__chip details__chip--lost" role="status">
        {t('writing.zoteroDetailLost')}
      </p>
      {#if tombstone.reason.trim()}
        <p class="details__notice">
          {t('writing.zoteroDetailReason', { reason: tombstone.reason })}
        </p>
      {/if}
      {#if lostItem}
        {@render ficha(lostItem)}
      {:else}
        <p class="details__notice">{t('writing.zoteroDetailNoSnapshot')}</p>
      {/if}
    {:else if offline}
      <!-- The held CSL, already parsed by the entry: what the list showed, with
       a chip that says only that it is unverified right now. Nothing here
       claims Zotero is closed or absent — nothing observed says that. -->
      <p class="details__chip details__chip--local" role="status">
        {t('writing.zoteroDetailLocalCopy')}
      </p>
      <h3 class="details__title">{entry.title}</h3>
      {#if entry.authors || entry.year}
        <p class="details__meta">{[entry.authors, entry.year].filter(Boolean).join(' · ')}</p>
      {/if}
    {/if}
  {/if}
</div>

<style>
  .details {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 0;
    overflow-y: auto;
  }

  .details__head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .details__actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .details__heading {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    font-weight: 600;
  }

  .details__chip {
    margin: 0;
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-surface);
    background: var(--surface-input);
    color: var(--color-text-secondary);
    font-size: var(--font-size-2xs);
    line-height: var(--line-height-base);
  }

  .details__chip--lost {
    color: var(--color-danger);
  }

  .details__chip--local {
    color: var(--color-text-muted);
  }

  .details__title {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
    font-weight: 600;
    line-height: var(--line-height-base);
  }

  .details__meta {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .details__fields {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
  }

  .details__field {
    display: grid;
    grid-template-columns: 7rem 1fr;
    gap: var(--space-2);
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }

  .details__field dt {
    margin: 0;
    color: var(--color-text-muted);
  }

  .details__field dd {
    margin: 0;
    color: var(--color-text-secondary);
    overflow-wrap: anywhere;
  }

  .details__attachments {
    margin: 0;
    padding-left: var(--space-4);
  }

  .details__notice {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }

  .details__error {
    margin: 0;
    color: var(--color-danger);
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }
</style>
