<!--
  A work's Zotero tags. Manual tags show; automatic ones (Zotero's `type: 1`:
  imported keywords, subject headings) stay hidden behind a toggle, so the
  ficha shows what the person chose. A filter of the view only.
-->
<script lang="ts">
  import { Button } from '@entropia/ui'
  import { t } from '$lib/i18n'

  let { tags, automaticTags = [] }: { tags: string[]; automaticTags?: string[] } = $props()

  let showAutomatic = $state(false)
</script>

<span class="zotero-tags">
  {#if tags.length > 0}
    <span>{tags.join(', ')}</span>
  {/if}
  {#if automaticTags.length > 0}
    <Button
      variant="ghost"
      size="sm"
      aria-expanded={showAutomatic}
      onclick={() => (showAutomatic = !showAutomatic)}
    >
      {showAutomatic
        ? t('writing.zoteroAutomaticTagsHide')
        : t('writing.zoteroAutomaticTagsShow', { count: automaticTags.length })}
    </Button>
    {#if showAutomatic}
      <span class="zotero-tags__automatic">{automaticTags.join(', ')}</span>
    {/if}
  {/if}
</span>

<style>
  .zotero-tags {
    display: inline-flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-1) var(--space-2);
  }

  .zotero-tags__automatic {
    flex-basis: 100%;
    color: var(--color-text-secondary);
  }
</style>
