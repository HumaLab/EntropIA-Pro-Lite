<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import { Button } from '@entropia/ui'
  import {
    clearLogs,
    formatLogEntry,
    getLogs,
    onLogEntry,
    openLogsDir,
    type AppLogEntry,
  } from '$lib/logs'

  let entries = $state<AppLogEntry[]>([])
  let loading = $state(false)
  let feedback = $state<{ tone: 'success' | 'error'; text: string } | null>(null)
  let unlisten: (() => void) | null = null
  const LOG_WINDOW_SIZE = 20

  let renderedLogs = $derived(entries.map(formatLogEntry).join('\n'))

  onMount(async () => {
    await refreshLogs()
    unlisten = await onLogEntry((entry) => {
      entries = [...entries, entry].slice(-LOG_WINDOW_SIZE)
    })
  })

  onDestroy(() => {
    unlisten?.()
  })

  async function refreshLogs() {
    loading = true
    feedback = null
    try {
      entries = (await getLogs()).slice(-LOG_WINDOW_SIZE)
    } catch (error) {
      feedback = { tone: 'error', text: `No se pudieron cargar los logs: ${String(error)}` }
    } finally {
      loading = false
    }
  }

  async function copyLogs() {
    try {
      await navigator.clipboard.writeText(renderedLogs)
      feedback = { tone: 'success', text: 'Logs copiados al portapapeles.' }
    } catch (error) {
      feedback = { tone: 'error', text: `No se pudieron copiar los logs: ${String(error)}` }
    }
  }

  async function handleClearLogs() {
    try {
      await clearLogs()
      entries = []
      feedback = { tone: 'success', text: 'Logs limpiados.' }
    } catch (error) {
      feedback = { tone: 'error', text: `No se pudieron limpiar los logs: ${String(error)}` }
    }
  }

  async function handleOpenLogsDir() {
    try {
      await openLogsDir()
    } catch (error) {
      feedback = { tone: 'error', text: `No se pudo abrir la carpeta de logs: ${String(error)}` }
    }
  }
</script>

<section class="logs-tab">
  <div class="logs-tab__header">
    <div>
      <h2>Logs</h2>
      <p>
        Diagnóstico local de instalación, runtime y procesos de IA. Copialos cuando necesitemos
        analizar una falla real.
      </p>
    </div>
    <div class="logs-tab__actions">
      <Button variant="secondary" size="sm" onclick={refreshLogs} disabled={loading}>
        {loading ? 'Actualizando…' : 'Refrescar'}
      </Button>
      <Button variant="secondary" size="sm" onclick={copyLogs} disabled={entries.length === 0}>
        Copiar
      </Button>
      <Button variant="secondary" size="sm" onclick={handleOpenLogsDir}>Abrir carpeta</Button>
      <Button
        variant="secondary"
        size="sm"
        onclick={handleClearLogs}
        disabled={entries.length === 0}
      >
        Limpiar
      </Button>
    </div>
  </div>

  {#if feedback}
    <p
      class="surface-message logs-tab__feedback"
      class:surface-message--success={feedback.tone === 'success'}
      class:surface-message--error={feedback.tone === 'error'}
    >
      {feedback.text}
    </p>
  {/if}

  <div class="logs-tab__body" aria-live="polite">
    {#if loading && entries.length === 0}
      <p class="logs-tab__empty">Cargando logs…</p>
    {:else if entries.length === 0}
      <p class="logs-tab__empty">Todavía no hay logs capturados en esta sesión.</p>
    {:else}
      {#each entries as entry (entry.id)}
        <article class="logs-tab__entry logs-tab__entry--{entry.level}">
          <time>{new Date(entry.timestamp_ms).toLocaleTimeString('es-AR')}</time>
          <span class="logs-tab__level">{entry.level.toUpperCase()}</span>
          <span class="logs-tab__source">{entry.source}</span>
          <p>{entry.message}</p>
        </article>
      {/each}
    {/if}
  </div>
</section>

<style>
  .logs-tab {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .logs-tab__header {
    display: flex;
    justify-content: space-between;
    gap: var(--space-4);
    align-items: flex-start;
    padding: var(--space-4);
    border: 1px solid color-mix(in srgb, var(--color-hairline) 78%, transparent);
    border-radius: var(--radius-lg);
    background: color-mix(in srgb, var(--color-surface-glass) 76%, transparent);
  }

  .logs-tab__header h2,
  .logs-tab__header p,
  .logs-tab__entry p,
  .logs-tab__empty,
  .logs-tab__feedback {
    margin: 0;
  }

  .logs-tab__header h2 {
    font-size: var(--font-size-base);
    margin-bottom: var(--space-2);
  }

  .logs-tab__header p,
  .logs-tab__empty {
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
    line-height: 1.55;
  }

  .logs-tab__actions {
    display: flex;
    flex-wrap: wrap;
    justify-content: flex-end;
    gap: var(--space-2);
  }

  .logs-tab__body {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-height: min(62vh, 720px);
    overflow: auto;
    padding: var(--space-3);
    border: 1px solid color-mix(in srgb, var(--color-hairline) 78%, transparent);
    border-radius: var(--radius-lg);
    background: color-mix(in srgb, var(--color-surface-glass) 64%, transparent);
  }

  .logs-tab__entry {
    display: grid;
    grid-template-columns: auto auto minmax(96px, auto) 1fr;
    gap: var(--space-2);
    align-items: baseline;
    padding: var(--space-2) var(--space-3);
    border: 1px solid color-mix(in srgb, var(--color-hairline) 68%, transparent);
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--color-surface-glass) 78%, transparent);
    font-family: var(--font-mono, monospace);
    font-size: var(--font-size-xs);
  }

  .logs-tab__entry time,
  .logs-tab__source {
    color: var(--color-text-secondary);
  }

  .logs-tab__level {
    font-weight: var(--font-weight-semibold);
  }

  .logs-tab__entry--warn .logs-tab__level {
    color: var(--color-warning);
  }

  .logs-tab__entry--error .logs-tab__level {
    color: var(--color-error);
  }

  .logs-tab__entry p {
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: var(--color-text-primary);
  }

  @media (max-width: 760px) {
    .logs-tab__header {
      flex-direction: column;
    }

    .logs-tab__actions {
      justify-content: flex-start;
    }

    .logs-tab__entry {
      grid-template-columns: 1fr;
    }
  }
</style>
