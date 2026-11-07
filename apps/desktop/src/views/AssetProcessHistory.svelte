<script lang="ts">
  import { Button, tooltip } from '@entropia/ui'
  import { save } from '@tauri-apps/plugin-dialog'
  import { writeFile } from '@tauri-apps/plugin-fs'
  import {
    getAssetProcessHistory,
    serializeAssetProcessHistory,
    type AssetProcessEvent,
  } from '$lib/asset-history'
  import type { I18nKey, I18nParams } from '$lib/i18n'
  import type { Asset } from '@entropia/store'

  let {
    asset,
    refreshKey = '',
    translate,
  }: {
    asset: Pick<Asset, 'id' | 'path' | 'type'> | null
    /** Any change reloads the history (an OCR finished, an edit landed...). */
    refreshKey?: string
    translate: (key: I18nKey, params?: I18nParams) => string
  } = $props()

  let events = $state<AssetProcessEvent[]>([])
  let loading = $state(false)
  let loadError = $state<string | null>(null)
  let message = $state<{ tone: 'success' | 'error'; text: string } | null>(null)
  let loadToken = 0

  async function load(assetId: string) {
    const token = ++loadToken
    loading = true
    loadError = null
    try {
      const loaded = await getAssetProcessHistory(assetId)
      if (token !== loadToken) return
      // Newest first: the latest process is what the person usually checks.
      events = [...loaded].reverse()
    } catch (error) {
      if (token !== loadToken) return
      events = []
      loadError = error instanceof Error ? error.message : String(error)
    } finally {
      if (token === loadToken) loading = false
    }
  }

  $effect(() => {
    const assetId = asset?.id
    void refreshKey
    message = null
    if (!assetId) {
      loadToken++
      events = []
      loading = false
      return
    }
    void load(assetId)
  })

  function processLabel(process: string) {
    const key = `item.process.${process}` as I18nKey
    const label = translate(key)
    return label === key ? translate('item.processHistoryUnknown', { process }) : label
  }

  function formatDate(ms: number) {
    return new Date(ms).toLocaleString()
  }

  function formatParams(params: Record<string, unknown> | null) {
    if (!params) return ''
    return Object.entries(params)
      .map(([key, value]) => `${key}: ${typeof value === 'string' ? value : JSON.stringify(value)}`)
      .join(' · ')
  }

  function shortHash(hash: string | null) {
    return hash ? `${hash.slice(0, 12)}…` : ''
  }

  async function exportHistory() {
    if (!asset) return
    message = null
    try {
      const oldestFirst = [...events].reverse()
      const path = await save({
        defaultPath: `historial-${asset.id}.json`,
        filters: [{ name: 'JSON', extensions: ['json'] }],
      })
      if (!path) return
      await writeFile(
        path,
        new TextEncoder().encode(serializeAssetProcessHistory(asset, oldestFirst))
      )
      message = { tone: 'success', text: translate('item.processHistoryExported') }
    } catch (error) {
      message = {
        tone: 'error',
        text: translate('item.processHistoryExportError', {
          error: error instanceof Error ? error.message : String(error),
        }),
      }
    }
  }
</script>

<section class="section" data-testid="asset-process-history">
  <div class="history-header">
    <h3>{translate('item.processHistory')}</h3>
    <div class="history-actions">
      <Button
        variant="secondary"
        size="sm"
        type="button"
        data-testid="asset-process-history-reload"
        disabled={!asset || loading}
        onclick={() => asset && load(asset.id)}
      >
        {translate('item.processHistoryReload')}
      </Button>
      <Button
        variant="secondary"
        size="sm"
        type="button"
        data-testid="asset-process-history-export"
        disabled={!asset || events.length === 0}
        onclick={exportHistory}
      >
        {translate('item.processHistoryExport')}
      </Button>
    </div>
  </div>
  <p class="history-help">{translate('item.processHistoryHelp')}</p>

  {#if loading && events.length === 0}
    <p class="history-empty">{translate('item.processHistoryLoading')}</p>
  {:else if loadError}
    <p class="history-error">{translate('item.processHistoryError', { error: loadError })}</p>
  {:else if events.length === 0}
    <p class="history-empty" data-testid="asset-process-history-empty">
      {translate('item.processHistoryEmpty')}
    </p>
  {:else}
    <ol class="history-list">
      {#each events as event (event.id)}
        <li class="history-event" data-testid={`asset-process-event-${event.process}`}>
          <div class="history-event__head">
            <strong>{processLabel(event.process)}</strong>
            <time datetime={new Date(event.createdAt).toISOString()}
              >{formatDate(event.createdAt)}</time
            >
          </div>
          <dl class="history-event__details">
            <div>
              <dt>{translate('item.processHistoryTool')}</dt>
              <dd>
                {event.tool}{#if event.toolVersion}&nbsp;<span class="version"
                    >{event.toolVersion}</span
                  >{/if}
              </dd>
            </div>
            {#if event.model}
              <div>
                <dt>{translate('item.processHistoryModel')}</dt>
                <dd>
                  {event.model}{#if event.modelVersion}&nbsp;<span class="version"
                      >{event.modelVersion}</span
                    >{/if}
                </dd>
              </div>
            {/if}
            <div>
              <dt>{translate('item.processHistoryApp')}</dt>
              <dd>{event.appVersion}</dd>
            </div>
            {#if event.params}
              <div>
                <dt>{translate('item.processHistoryParams')}</dt>
                <dd class="params">{formatParams(event.params)}</dd>
              </div>
            {/if}
            {#if event.inputSha256}
              <div>
                <dt>{translate('item.processHistoryInput')}</dt>
                <dd><code use:tooltip={event.inputSha256}>{shortHash(event.inputSha256)}</code></dd>
              </div>
            {/if}
            {#if event.outputSha256}
              <div>
                <dt>{translate('item.processHistoryOutput')}</dt>
                <dd>
                  <code use:tooltip={event.outputSha256}>{shortHash(event.outputSha256)}</code>
                </dd>
              </div>
            {/if}
          </dl>
        </li>
      {/each}
    </ol>
  {/if}

  {#if message}
    <p
      class="history-message"
      class:history-message--error={message.tone === 'error'}
      role="status"
      data-testid="asset-process-history-message"
    >
      {message.text}
    </p>
  {/if}
</section>

<style>
  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: var(--color-surface);
    box-shadow: var(--shadow-surface);
    container-type: inline-size;
  }

  .history-header {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .history-header h3 {
    margin: 0;
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
  }

  .history-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .history-help,
  .history-empty {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    line-height: 1.4;
  }

  .history-error {
    margin: 0;
    color: var(--color-danger);
    font-size: var(--font-size-sm);
  }

  .history-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .history-event {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    background: var(--surface-card);
  }

  .history-event__head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
  }

  .history-event__head time {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    font-variant-numeric: tabular-nums;
  }

  .history-event__details {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
  }

  .history-event__details > div {
    display: grid;
    grid-template-columns: minmax(0, 0.35fr) minmax(0, 0.65fr);
    gap: var(--space-2);
  }

  @container (max-width: 240px) {
    .history-event__details > div {
      grid-template-columns: 1fr;
    }
  }

  .history-event__details dt {
    font-size: var(--font-size-2xs);
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--color-text-muted);
  }

  .history-event__details dd {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-primary);
    overflow-wrap: anywhere;
  }

  .history-event__details code {
    font-family: var(--font-mono);
  }

  .version {
    color: var(--color-text-muted);
  }

  .history-message {
    width: fit-content;
    margin: 0;
    padding: var(--space-1) var(--space-2);
    border: 1px solid color-mix(in srgb, var(--color-success) 28%, transparent);
    border-radius: var(--radius-sm);
    background: var(--color-success-soft);
    color: var(--color-success);
    font-size: var(--font-size-sm);
  }

  .history-message--error {
    border-color: color-mix(in srgb, var(--color-danger) 30%, transparent);
    background: var(--color-danger-soft);
    color: var(--color-danger);
  }
</style>
