<script lang="ts">
  import { invoke } from '@tauri-apps/api/core'
  import { onMount } from 'svelte'
  import { Card, Checkbox } from '@entropia/ui'
  import { t } from '$lib/i18n'

  // How the search index reads OCR text. The setting is saved as soon as the
  // box changes: it rebuilds the index in the background, so it does not wait
  // for the settings form's Save button.
  let rejoin = $state(true)
  let loaded = $state(false)
  let saving = $state(false)
  let notice = $state<string | null>(null)
  let error = $state<string | null>(null)

  function messageOf(cause: unknown): string {
    if (typeof cause === 'object' && cause !== null && 'message' in cause) {
      return String((cause as { message: unknown }).message)
    }
    return String(cause)
  }

  onMount(async () => {
    try {
      rejoin = await invoke<boolean>('fts_get_rejoin_hyphens')
    } catch (cause) {
      error = t('settings.searchIndex.loadFailed', { error: messageOf(cause) })
    } finally {
      loaded = true
    }
  })

  async function save(enabled: boolean) {
    saving = true
    notice = null
    error = null
    try {
      await invoke('fts_set_rejoin_hyphens', { enabled })
      notice = t('settings.searchIndex.rebuilding')
    } catch (cause) {
      rejoin = !enabled
      error = t('settings.searchIndex.saveFailed', { error: messageOf(cause) })
    } finally {
      saving = false
    }
  }
</script>

<Card>
  <section class="search-index">
    <h2 class="search-index__title">{t('settings.searchIndex.title')}</h2>
    <Checkbox bind:checked={rejoin} disabled={!loaded || saving} onchange={save}>
      <strong>{t('settings.searchIndex.rejoinHyphens.label')}</strong>
      <span class="search-index__desc">{t('settings.searchIndex.rejoinHyphens.description')}</span>
    </Checkbox>
    {#if notice}
      <p class="search-index__notice" role="status">{notice}</p>
    {/if}
    {#if error}
      <p class="search-index__error" role="alert">{error}</p>
    {/if}
  </section>
</Card>

<style>
  .search-index {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .search-index__title {
    margin: 0;
    color: var(--color-text-primary);
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-medium);
  }

  .search-index__desc {
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }

  .search-index__notice,
  .search-index__error {
    margin: 0;
    font-size: var(--font-size-sm);
  }

  .search-index__notice {
    color: var(--color-text-secondary);
  }

  .search-index__error {
    color: var(--color-danger);
  }
</style>
