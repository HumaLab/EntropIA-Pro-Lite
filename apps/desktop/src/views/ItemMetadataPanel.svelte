<script lang="ts">
  import { MetadataEditor, type MetadataEditorProps } from '@entropia/ui'
  import type { I18nKey, I18nParams } from '$lib/i18n'
  import type { TechnicalMetadataEntry } from '$lib/item-metadata'

  let {
    savingMetadata,
    fileMetadataEntries,
    metadataValue,
    metadataEditorLabels,
    translate,
    onMetadataChange,
  }: {
    savingMetadata: boolean
    fileMetadataEntries: TechnicalMetadataEntry[]
    metadataValue: Record<string, string>
    metadataEditorLabels: MetadataEditorProps['labels']
    translate: (key: I18nKey, params?: I18nParams) => string
    onMetadataChange: (metadata: Record<string, string>) => void
  } = $props()
</script>

<section class="section">
  <h3>
    {translate('item.metadata')}
    {#if savingMetadata}<span class="saving">{translate('item.saving')}</span>{/if}
  </h3>

  <div class="metadata-sections">
    <section class="metadata-subsection" data-testid="item-file-metadata">
      <h4>{translate('item.fileMetadata')}</h4>

      {#if fileMetadataEntries.length > 0}
        <dl class="metadata-list">
          {#each fileMetadataEntries as entry (entry.label)}
            <div class="metadata-list__row">
              <dt>{entry.label}</dt>
              <dd>{entry.value}</dd>
            </div>
          {/each}
        </dl>
      {/if}
    </section>

    <section class="metadata-subsection" data-testid="item-custom-metadata">
      <h4>{translate('item.customMetadata')}</h4>
      <MetadataEditor
        value={metadataValue}
        onchange={onMetadataChange}
        labels={metadataEditorLabels}
      />
    </section>
  </div>
</section>

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: var(--color-surface);
    box-shadow: var(--shadow-surface);
  }

  .section h3 {
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
    margin-bottom: var(--space-1);
  }

  .saving {
    margin-left: var(--space-2);
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
    font-weight: var(--font-weight-normal);
  }

  .metadata-sections {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }

  .metadata-subsection {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .metadata-subsection h4 {
    margin: 0;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
    color: var(--color-text-primary);
  }

  .metadata-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
  }

  .metadata-list__row {
    display: grid;
    grid-template-columns: minmax(0, 0.45fr) minmax(0, 0.55fr);
    gap: var(--space-3);
    padding: var(--space-2) 0;
    border-bottom: 1px solid var(--color-border-subtle);
  }

  .metadata-list__row:last-child {
    border-bottom: none;
  }

  .metadata-list dt {
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: 0.02em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }

  .metadata-list dd {
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
    overflow-wrap: anywhere;
  }
</style>
