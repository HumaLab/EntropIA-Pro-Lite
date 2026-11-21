<script lang="ts">
  import { Button } from '@entropia/ui'
  import type { I18nKey, I18nParams } from '$lib/i18n'
  import { listTranscriptionVersions, type TranscriptVersion } from '$lib/transcription'

  // The transcripts a re-transcription replaced (finding 14). Loaded when the
  // list opens, so editing the current transcript never queries them.
  let {
    assetId,
    translate,
    onRestore,
  }: {
    assetId: string
    translate: (key: I18nKey, params?: I18nParams) => string
    /** Restores the version and reloads the current transcript. */
    onRestore: (version: TranscriptVersion) => Promise<void>
  } = $props()

  let open = $state(false)
  let versions = $state<TranscriptVersion[] | null>(null)
  let error = $state<string | null>(null)
  let notice = $state<string | null>(null)
  let restoringId = $state<string | null>(null)

  async function load() {
    error = null
    try {
      const forAsset = assetId
      const loaded = await listTranscriptionVersions(forAsset)
      if (forAsset === assetId) versions = loaded
    } catch (cause) {
      error = translate('item.transcriptVersionsFailed', { error: String(cause) })
    }
  }

  async function toggle() {
    open = !open
    notice = null
    if (open) await load()
  }

  async function restore(version: TranscriptVersion) {
    if (restoringId) return
    restoringId = version.id
    error = null
    notice = null
    try {
      await onRestore(version)
      notice = translate('item.transcriptVersionRestored')
      await load()
    } catch (cause) {
      error = translate('item.transcriptVersionRestoreFailed', { error: String(cause) })
    } finally {
      restoringId = null
    }
  }

  function preview(text: string): string {
    const firstLine = text.trim().split('\n')[0] ?? ''
    return firstLine.length > 120 ? `${firstLine.slice(0, 120)}…` : firstLine
  }
</script>

<div class="transcript-versions">
  <Button variant="secondary" size="sm" aria-expanded={open} onclick={toggle}>
    {open ? translate('item.transcriptVersionsHide') : translate('item.transcriptVersions')}
  </Button>
  {#if open}
    {#if error}
      <p class="transcript-versions__error" role="alert">{error}</p>
    {/if}
    {#if notice}
      <p class="transcript-versions__notice" role="status">{notice}</p>
    {/if}
    {#if versions && versions.length === 0}
      <p class="transcript-versions__empty">{translate('item.transcriptVersionsEmpty')}</p>
    {:else if versions}
      <ul class="transcript-versions__list">
        {#each versions as version (version.id)}
          <li class="transcript-versions__item">
            <div class="transcript-versions__text">
              <span class="transcript-versions__meta">
                {translate('item.transcriptVersionMeta', {
                  date: new Date(version.replacedAt).toLocaleString(),
                  model: version.model ?? '—',
                  count: version.textContent.length,
                })}
              </span>
              <span class="transcript-versions__preview">{preview(version.textContent)}</span>
            </div>
            <Button
              variant="secondary"
              size="sm"
              disabled={restoringId !== null}
              onclick={() => restore(version)}
            >
              {translate('item.transcriptVersionRestore')}
            </Button>
          </li>
        {/each}
      </ul>
    {/if}
  {/if}
</div>

<style>
  .transcript-versions {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-start;
  }

  .transcript-versions__list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    width: 100%;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .transcript-versions__item {
    display: flex;
    gap: var(--space-2);
    align-items: center;
    justify-content: space-between;
  }

  .transcript-versions__text {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .transcript-versions__meta,
  .transcript-versions__empty,
  .transcript-versions__notice {
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .transcript-versions__preview {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .transcript-versions__error {
    color: var(--color-danger);
    font-size: var(--font-size-sm);
  }
</style>
