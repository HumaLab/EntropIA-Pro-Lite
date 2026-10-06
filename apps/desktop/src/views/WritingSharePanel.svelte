<script lang="ts">
  import { Button, IconButton, ActionIcon, Input, Panel } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import { describeSyncError, syncNow } from '$lib/sync'
  import { openExternalUrl } from '$lib/external-links'
  import { SETTINGS_KEYS, settingsGet, settingsSet } from '$lib/settings'
  import {
    listWritingShares,
    publishToHlab,
    shareWriting,
    unshareWriting,
    type PublishedPost,
    type WritingShare,
  } from '$lib/writing-publish'

  /**
   * Share a manuscript with another account and send it to the hlab.com.ar
   * blog (T-33). Opens under the writing bar, like the export notice.
   *
   * Sharing hides itself when the sync server predates it; publishing asks
   * for the site's key once and keeps it in the system credential store.
   */

  interface Props {
    documentId: string
    title: string
    /** Renders the article body at the moment of sending. */
    articleHtml: () => Promise<string>
    onclose: () => void
  }

  let { documentId, title, articleHtml, onclose }: Props = $props()

  /** `undefined` while loading, `null` when the server cannot share. */
  let share = $state<WritingShare | null | undefined>(undefined)
  let shareSupported = $state(true)
  let shareError = $state('')
  let email = $state('')
  let busy = $state(false)

  let hasKey = $state(false)
  let key = $state('')
  let published = $state<PublishedPost | null>(null)
  let publishError = $state('')

  async function loadShares() {
    try {
      const shares = await listWritingShares()
      shareSupported = shares !== null
      share = shares?.find((entry) => entry.document_id === documentId) ?? null
    } catch (error) {
      share = null
      shareError = describeSyncError(error) || t('writing.shareNeedsSync')
    }
  }

  async function loadKey() {
    try {
      hasKey = Boolean(await settingsGet(SETTINGS_KEYS.HLAB_PUBLISH_KEY))
    } catch {
      hasKey = false
    }
  }

  $effect(() => {
    void documentId
    loadShares()
    loadKey()
  })

  async function run(action: () => Promise<void>, onError: (message: string) => void) {
    if (busy) return
    busy = true
    try {
      await action()
    } catch (error) {
      onError(error instanceof Error ? error.message : String(error))
    } finally {
      busy = false
    }
  }

  function add() {
    const address = email.trim()
    if (!address) return
    shareError = ''
    // A document the server has never seen cannot be shared: sync first.
    return run(
      async () => {
        await syncNow()
        share = await shareWriting(documentId, address)
        email = ''
      },
      (message) => (shareError = describeSyncError(message) || message)
    )
  }

  function remove(address: string) {
    shareError = ''
    return run(
      async () => {
        await unshareWriting(documentId, address)
        await loadShares()
      },
      (message) => (shareError = describeSyncError(message) || message)
    )
  }

  function saveKey() {
    publishError = ''
    return run(
      async () => {
        await settingsSet(SETTINGS_KEYS.HLAB_PUBLISH_KEY, key.trim())
        key = ''
        hasKey = true
      },
      (message) => (publishError = message)
    )
  }

  function publish() {
    publishError = ''
    published = null
    return run(
      async () => {
        published = await publishToHlab(documentId, title, await articleHtml())
      },
      (message) => {
        // A rejected key is asked for again rather than reported forever.
        if (message.includes('hlab_key_rejected') || message.includes('hlab_key_missing')) {
          hasKey = false
        }
        publishError = message.replace(/^hlab_[a-z_]+: /, '')
      }
    )
  }
</script>

<Panel padding="md">
  <div class="share-panel">
    <IconButton size="sm" variant="ghost" label={t('sync.notif.close')} onclick={onclose}>
      <ActionIcon name="close" size={14} />
    </IconButton>

    <section>
      <h3>{t('writing.shareTitle')}</h3>
      {#if !shareSupported}
        <p class="share-panel__hint">{t('writing.shareUnavailable')}</p>
      {:else if share && !share.is_owner}
        <p>{t('writing.shareFrom', { email: share.owner_email })}</p>
        {#each share.members as member (member)}
          <!-- A member sees only themselves to remove: leaving keeps the copy. -->
          <Button variant="ghost" size="sm" disabled={busy} onclick={() => remove(member)}>
            {t('writing.shareLeave')}
          </Button>
        {/each}
      {:else}
        <p class="share-panel__hint">{t('writing.shareHint')}</p>
        {#if share}
          <ul>
            {#each share.members as member (member)}
              <li>
                {member}
                <Button variant="ghost" size="sm" disabled={busy} onclick={() => remove(member)}>
                  {t('writing.shareRemove')}
                </Button>
              </li>
            {/each}
          </ul>
        {/if}
        <div class="share-panel__row">
          <Input type="email" bind:value={email} label={t('writing.shareEmail')} />
          <Button size="sm" disabled={busy || !email.trim()} onclick={add}>
            {t('writing.shareAdd')}
          </Button>
        </div>
      {/if}
      {#if shareError}<p class="share-panel__error" role="alert">{shareError}</p>{/if}
    </section>

    <section>
      <h3>{t('writing.publishTitle')}</h3>
      <p class="share-panel__hint">{t('writing.publishHint')}</p>
      {#if hasKey}
        <Button size="sm" disabled={busy} onclick={publish}>
          <ActionIcon name={busy ? 'loader' : 'send'} size={14} />
          {t('writing.publishSend')}
        </Button>
      {:else}
        <div class="share-panel__row">
          <Input type="password" bind:value={key} label={t('writing.publishKey')} />
          <Button size="sm" disabled={busy || !key.trim()} onclick={saveKey}>
            {t('writing.publishSaveKey')}
          </Button>
        </div>
      {/if}
      {#if published}
        <p role="status">
          {t('writing.publishDone')}
          <Button variant="ghost" size="sm" onclick={() => openExternalUrl(published!.admin_url)}>
            <ActionIcon name="external-link" size={14} />
            {t('writing.publishOpenAdmin')}
          </Button>
        </p>
      {/if}
      {#if publishError}<p class="share-panel__error" role="alert">{publishError}</p>{/if}
    </section>
  </div>
</Panel>

<style>
  .share-panel {
    position: relative;
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-4);
  }

  .share-panel > :global(button:first-child) {
    position: absolute;
    top: 0;
    right: 0;
  }

  h3 {
    margin: 0 0 var(--space-2);
    font-size: var(--font-size-md, 1rem);
  }

  ul {
    margin: 0 0 var(--space-2);
    padding: 0;
    list-style: none;
  }

  .share-panel__row {
    display: flex;
    align-items: flex-end;
    gap: var(--space-2);
  }

  .share-panel__hint {
    color: var(--color-text-secondary);
    font-size: 0.9em;
  }

  .share-panel__error {
    color: var(--color-danger, #c0392b);
  }
</style>
