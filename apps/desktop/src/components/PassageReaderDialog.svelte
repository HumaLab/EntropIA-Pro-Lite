<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import { locale, t } from '$lib/i18n'
  import {
    bibliographyOpenPassage,
    bibliographyPassageContext,
    type BibliographyPassageContext,
  } from '$lib/bibliography-search'
  import { passageWindow } from '$lib/rag-scope'
  import { ConfirmDialog } from '@entropia/ui'

  /**
   * Reads one Zotero passage in the app: the page text the catalog holds with
   * the cited range marked. The original PDF/HTML lives in Zotero's storage,
   * outside what the app's asset protocol may serve, so the original is one
   * explicit click away, in the OS viewer. Shared by the research chat and the
   * Writing "Obras" tab: one reader, one look.
   */
  interface Props {
    chunkId: string
    title: string
    /** The line under the title: work, location, library. */
    heading: string
    /** Shown when the passage is not in this device's catalog. */
    fallbackSnippet: string
    onclose: () => void
  }

  let { chunkId, title, heading, fallbackSnippet, onclose }: Props = $props()

  const currentLocale = locale
  const PASSAGE_CONTEXT_RADIUS = 600

  let status = $state<'loading' | 'ready' | 'missing' | 'failed'>('loading')
  let context = $state<BibliographyPassageContext | null>(null)
  let reason = $state('')
  let opening = $state(false)
  let originalError = $state<string | null>(null)
  let alive = true

  function errorText(error: unknown): string {
    if (typeof error === 'string') return error
    if (error instanceof Error) return error.message
    return ''
  }

  onMount(async () => {
    try {
      const loaded = await bibliographyPassageContext(chunkId)
      if (!alive) return
      context = loaded
      status = 'ready'
    } catch (error) {
      if (!alive) return
      reason = errorText(error)
      // The catalog is local-only: a conversation synced from another device
      // names a chunk this one never had.
      status =
        reason.includes('unknown_chunk') || reason.includes('unknown_attachment')
          ? 'missing'
          : 'failed'
    }
  })

  onDestroy(() => {
    alive = false
  })

  async function openOriginal() {
    if (opening) return
    opening = true
    originalError = null
    try {
      const result = await bibliographyOpenPassage(chunkId)
      if (!alive) return
      originalError = result.openError
        ? t('ragChat.passageOriginalError', { reason: result.openError })
        : null
    } catch (error) {
      if (!alive) return
      originalError = t('ragChat.passageOriginalError', { reason: errorText(error) })
    } finally {
      if (alive) opening = false
    }
  }
</script>

<ConfirmDialog
  {title}
  titleId="passage-reader-title"
  message={heading}
  cancelLabel={$currentLocale && t('ragChat.passageClose')}
  confirmLabel={$currentLocale && t('ragChat.passageOpenOriginal')}
  confirming={opening}
  confirmDisabled={status !== 'ready'}
  error={originalError}
  oncancel={onclose}
  onconfirm={() => void openOriginal()}
>
  {#if status === 'loading'}
    <p class="passage-reader__note" role="status">
      {$currentLocale && t('ragChat.passageLoading')}
    </p>
  {:else if status === 'ready' && context}
    <div class="passage-reader">
      {#each context.pages as page (page.pageNumber)}
        {@const view = passageWindow(page.text, page.highlights, PASSAGE_CONTEXT_RADIUS)}
        <p class="passage-reader__page">
          {#if view.truncatedBefore}<span>… </span>{/if}
          {#each view.segments as segment, segmentIndex (segmentIndex)}
            {#if segment.marked}
              <mark class="passage-reader__hit">{segment.text}</mark>
            {:else}
              <span>{segment.text}</span>
            {/if}
          {/each}
          {#if view.truncatedAfter}<span> …</span>{/if}
        </p>
      {:else}
        <p class="passage-reader__page">{context.text}</p>
      {/each}
    </div>
    {#if context.openError}
      <p class="passage-reader__note">
        {$currentLocale && t('ragChat.passageOriginalUnavailable')}
      </p>
    {/if}
  {:else}
    <p class="passage-reader__note" role="status">
      {$currentLocale &&
        (status === 'missing'
          ? t('ragChat.passageMissing')
          : t('ragChat.passageError', { reason }))}
    </p>
    <p class="passage-reader__page">{fallbackSnippet}</p>
  {/if}
</ConfirmDialog>

<style>
  .passage-reader__note {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    overflow-wrap: anywhere;
  }

  .passage-reader {
    max-height: 50vh;
    overflow-y: auto;
  }

  .passage-reader__page {
    margin: 0 0 var(--space-2);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  /* The cited range, marked the way a citation marks it in the item view. */
  .passage-reader__hit {
    border-radius: var(--radius-xs);
    background: var(--color-warning-soft);
    box-shadow: inset 0 -2px 0 var(--color-warning);
    color: inherit;
  }
</style>
