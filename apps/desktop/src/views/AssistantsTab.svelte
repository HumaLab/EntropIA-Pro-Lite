<script lang="ts">
  /**
   * Configuración → Asistentes (D11): let an external assistant (Claude Code,
   * Codex, OpenCode, Pi) read this archive over the loopback MCP endpoint.
   *
   * The tab only shows what the backend reports and asks it to change state:
   * pairing, tokens and the endpoint itself live in
   * `src-tauri/src/automation`. The token never crosses the IPC boundary, so
   * nothing here can display or copy one (S4); the client id is the only thing
   * an assistant's configuration names.
   */
  import { onMount } from 'svelte'
  import { ActionIcon, Button, Card, ConfirmDialog, Input } from '@entropia/ui'
  import {
    agentAccessSetEnabled,
    agentAccessStatus,
    agentClientCreate,
    agentClientRevoke,
    connectorInvocation,
    connectorRecipes,
    type AgentAccessStatus,
    type AgentClient,
    type CreatedAgentClient,
  } from '$lib/agent-access'

  let status = $state<AgentAccessStatus | null>(null)
  /** The client created in this visit; its connection lines stay until revoked. */
  let created = $state<CreatedAgentClient | null>(null)
  let name = $state('')
  let busy = $state(false)
  let errorMessage = $state<string | null>(null)
  let pendingRevoke = $state<AgentClient | null>(null)
  let copiedLabel = $state<string | null>(null)
  let destroyed = false

  const recipes = $derived(created ? connectorRecipes(created.client.id) : [])
  const invocation = $derived(created ? connectorInvocation(created.client.id) : '')

  const endpointLabel = $derived.by(() => {
    if (!status) return 'Verificando…'
    if (!status.enabled) return 'Deshabilitado'
    if (status.running && status.port !== null) return `Activo en 127.0.0.1:${status.port}`
    return 'Habilitado, pero el endpoint no está activo'
  })

  onMount(() => {
    void refresh()
    return () => {
      destroyed = true
    }
  })

  async function refresh() {
    try {
      const next = await agentAccessStatus()
      if (!destroyed) status = next
    } catch (error) {
      if (!destroyed) errorMessage = String(error)
    }
  }

  async function toggleAccess() {
    if (!status || busy) return
    const enabled = !status.enabled
    busy = true
    errorMessage = null
    copiedLabel = null
    try {
      status = await agentAccessSetEnabled(enabled)
    } catch (error) {
      errorMessage = String(error)
      // The switch may have been persisted even when the endpoint failed to
      // start: show what the backend really holds instead of a guess.
      await refresh()
    } finally {
      busy = false
    }
  }

  async function connect() {
    const trimmed = name.trim()
    if (!trimmed || busy) return
    busy = true
    errorMessage = null
    copiedLabel = null
    try {
      const result = await agentClientCreate(trimmed)
      if (destroyed) return
      created = result
      status = result.status
      name = ''
    } catch (error) {
      if (!destroyed) errorMessage = String(error)
    } finally {
      busy = false
    }
  }

  async function confirmRevoke() {
    const client = pendingRevoke
    if (!client || busy) return
    busy = true
    errorMessage = null
    copiedLabel = null
    try {
      status = await agentClientRevoke(client.id)
      if (created?.client.id === client.id) created = null
    } catch (error) {
      errorMessage = String(error)
      // A failed keyring delete still leaves the client revoked: re-read it.
      await refresh()
    } finally {
      pendingRevoke = null
      busy = false
    }
  }

  async function copy(label: string, text: string) {
    errorMessage = null
    try {
      await navigator.clipboard.writeText(text)
      copiedLabel = label
    } catch (error) {
      errorMessage = `No se pudo copiar: ${String(error)}`
    }
  }

  /** `YYYY-MM-DD`: stable, locale-free and enough to recognize a date. */
  function formatDate(seconds: number): string {
    return new Date(seconds * 1000).toISOString().slice(0, 10)
  }

  /** Where the backend keeps this client's token (D14). */
  function storageLabel(store: AgentClient['tokenStore']): string {
    return store === 'file' ? 'Archivo protegido' : 'Llavero del sistema'
  }

  function copyActionLabel(recipe: (typeof recipes)[number]): string {
    return recipe.kind === 'command'
      ? `Copiar comando para ${recipe.label}`
      : `Copiar configuración para ${recipe.label}`
  }
</script>

<Card>
  <section class="assistants-tab">
    <p class="assistants-tab__hint">
      Permití que un asistente externo consulte este archivo en modo lectura. En esta versión solo
      se puede leer, y solo desde esta computadora: el endpoint escucha en 127.0.0.1 y nunca en la
      red local.
    </p>

    {#if errorMessage}
      <p class="assistants-tab__error" role="alert" data-testid="assistants-error">{errorMessage}</p>
    {/if}

    <div class="assistants-tab__row">
      <div class="assistants-tab__copy">
        <span class="assistants-tab__label">Permitir asistentes externos</span>
        <span class="assistants-tab__value" data-testid="assistants-endpoint-state"
          >{endpointLabel}</span
        >
      </div>
      <button
        type="button"
        class="assistants-tab__switch"
        class:assistants-tab__switch--on={status?.enabled ?? false}
        role="switch"
        aria-checked={status?.enabled ?? false}
        aria-label="Permitir asistentes externos"
        disabled={!status || busy}
        onclick={toggleAccess}
      >
        <span class="assistants-tab__switch-knob"></span>
      </button>
    </div>

    <div class="assistants-tab__field">
      <span class="assistants-tab__label">Asistentes emparejados</span>
      {#if status && status.clients.length > 0}
        <ul class="assistants-tab__clients">
          {#each status.clients as client (client.id)}
            <li class="assistants-tab__client">
              <div class="assistants-tab__copy">
                <span class="assistants-tab__value">{client.name}</span>
                <span
                  class="assistants-tab__meta"
                  data-testid="assistants-client-created-{client.id}"
                >
                  Creado el {formatDate(client.createdAt)}
                </span>
                <span
                  class="assistants-tab__meta"
                  data-testid="assistants-client-last-used-{client.id}"
                >
                  {client.lastUsedAt === null
                    ? 'Sin uso todavía'
                    : `Último uso: ${formatDate(client.lastUsedAt)}`}
                </span>
                <span
                  class="assistants-tab__meta"
                  data-testid="assistants-client-storage-{client.id}"
                >
                  {storageLabel(client.tokenStore)}
                </span>
                {#if client.tokenStore === 'file'}
                  <span
                    class="assistants-tab__meta"
                    data-testid="assistants-client-storage-hint-{client.id}"
                  >
                    Este equipo no tiene llavero del sistema: el token está en un archivo que solo
                    puede leer tu usuario.
                  </span>
                {/if}
              </div>
              <Button
                variant="danger"
                size="sm"
                aria-label="Revocar {client.name}"
                disabled={busy}
                onclick={() => (pendingRevoke = client)}
              >
                Revocar
              </Button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="assistants-tab__value">Todavía no conectaste ningún asistente.</p>
      {/if}
    </div>

    <div class="assistants-tab__field">
      <span class="assistants-tab__label">Conectar un asistente</span>
      <div class="assistants-tab__row">
        <div class="assistants-tab__grow">
          <Input
            bind:value={name}
            label="Nombre del asistente"
            placeholder="Claude Code"
            hint="Un nombre para reconocerlo después en la lista."
            disabled={busy}
          />
        </div>
        <Button
          variant="primary"
          size="sm"
          disabled={busy || !name.trim()}
          loading={busy}
          onclick={connect}
        >
          Conectar
        </Button>
      </div>
    </div>

    {#if created}
      <div class="assistants-tab__connection">
        <span class="assistants-tab__label">Cómo conectar {created.client.name}</span>
        <p class="assistants-tab__value">
          Identificador del cliente: <code data-testid="assistants-client-id"
            >{created.client.id}</code
          >
        </p>
        <p class="assistants-tab__hint">
          El conector de EntropIA tiene que estar instalado: esta versión todavía no lo incluye. La
          credencial queda en el llavero del sistema y estas líneas no la contienen, así que se
          pueden pegar en la configuración del asistente.
        </p>
        <ul class="assistants-tab__lines">
          <li class="assistants-tab__line">
            <span class="assistants-tab__line-name">Comando del conector</span>
            <div class="assistants-tab__line-row">
              <code class="assistants-tab__code" data-testid="assistants-connector-command"
                >{invocation}</code
              >
              <Button
                variant="ghost"
                size="sm"
                aria-label="Copiar comando del conector"
                onclick={() => copy('Comando del conector', invocation)}
              >
                <ActionIcon name="copy" size={14} /> Copiar
              </Button>
            </div>
          </li>
          {#each recipes as recipe (recipe.id)}
            <li class="assistants-tab__line">
              <span class="assistants-tab__line-name">{recipe.label}</span>
              <div class="assistants-tab__line-row">
                <code class="assistants-tab__code" data-testid="assistants-line-{recipe.id}"
                  >{recipe.line}</code
                >
                <Button
                  variant="ghost"
                  size="sm"
                  aria-label={copyActionLabel(recipe)}
                  onclick={() => copy(recipe.label, recipe.line)}
                >
                  <ActionIcon name="copy" size={14} /> Copiar
                </Button>
              </div>
            </li>
          {/each}
        </ul>
        {#if copiedLabel}
          <p class="assistants-tab__copied" role="status">Copiado: {copiedLabel}</p>
        {/if}
      </div>
    {/if}
  </section>
</Card>

{#if pendingRevoke}
  <ConfirmDialog
    title="Revocar {pendingRevoke.name}"
    titleId="assistants-revoke-title"
    message="La credencial de {pendingRevoke.name} deja de funcionar en la próxima petición, incluso si alguien guardó una copia. Para volver a conectarlo hay que emparejarlo de nuevo."
    cancelLabel="Cancelar"
    confirmLabel="Revocar asistente"
    variant="destructive"
    confirming={busy}
    oncancel={() => (pendingRevoke = null)}
    onconfirm={confirmRevoke}
  />
{/if}

<style>
  .assistants-tab {
    display: flex;
    flex-direction: column;
  }

  .assistants-tab__hint {
    margin: 0;
    padding-bottom: var(--space-3);
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .assistants-tab__error {
    margin: 0 0 var(--space-3);
    padding: var(--space-2) var(--space-3);
    border: 1px solid rgba(239, 68, 68, 0.3);
    border-radius: var(--radius-md);
    background: var(--color-danger-soft);
    color: var(--color-danger);
    font-size: var(--font-size-sm);
  }

  .assistants-tab__row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .assistants-tab__copy {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .assistants-tab__grow {
    flex: 1;
    min-width: 0;
  }

  .assistants-tab__label {
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
  }

  .assistants-tab__value {
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
    overflow-wrap: anywhere;
  }

  .assistants-tab__meta {
    color: var(--color-text-muted, #6b7280);
    font-size: var(--font-size-xs);
  }

  .assistants-tab__field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) 0;
    border-top: 1px solid var(--color-hairline);
  }

  .assistants-tab__switch {
    position: relative;
    flex: 0 0 auto;
    width: 36px;
    height: 20px;
    padding: 0;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-full, 9999px);
    background: var(--color-surface-sunken);
    cursor: pointer;
    transition: background var(--transition-base);
  }

  .assistants-tab__switch--on {
    border-color: transparent;
    background: var(--color-success);
  }

  .assistants-tab__switch:disabled {
    opacity: 0.5;
    cursor: default;
  }

  .assistants-tab__switch-knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--color-surface-elevated);
    transition: transform var(--transition-base);
  }

  .assistants-tab__switch--on .assistants-tab__switch-knob {
    transform: translateX(16px);
  }

  .assistants-tab__clients {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .assistants-tab__client {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
  }

  .assistants-tab__connection {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-md);
    background: var(--color-surface-sunken);
  }

  .assistants-tab__lines {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .assistants-tab__line {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .assistants-tab__line-name {
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .assistants-tab__line-row {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
  }

  .assistants-tab__code {
    flex: 1;
    min-width: 0;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-sm, 4px);
    background: var(--color-surface);
    color: var(--color-text-primary);
    font-family: var(--font-mono);
    font-size: var(--font-size-xs);
    overflow-wrap: anywhere;
  }

  .assistants-tab__copied {
    margin: 0;
    color: var(--color-success);
    font-size: var(--font-size-xs);
  }
</style>
