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
  import { open as openFolderDialog } from '@tauri-apps/plugin-dialog'
  import { ActionIcon, Button, Card, Checkbox, ConfirmDialog, Input } from '@entropia/ui'
  import {
    agentAccessSetEnabled,
    agentAccessStatus,
    agentClientCreate,
    agentClientRevoke,
    agentClientSetGrants,
    agentConnectorLocation,
    agentImportFolderAdd,
    agentImportFolderRemove,
    connectorInvocation,
    isWindowsProgram,
    connectorRecipes,
    grantsOf,
    type AgentAccessStatus,
    type AgentClient,
    type AgentGrants,
    type ConnectorLocation,
    type CreatedAgentClient,
  } from '$lib/agent-access'

  let status = $state<AgentAccessStatus | null>(null)
  /** Where this build installed the connector; `null` until the backend answers. */
  let connector = $state<ConnectorLocation | null>(null)
  /** The client created in this visit; its connection lines stay until revoked. */
  let created = $state<CreatedAgentClient | null>(null)
  let name = $state('')
  let busy = $state(false)
  let errorMessage = $state<string | null>(null)
  let pendingRevoke = $state<AgentClient | null>(null)
  /** A grant waiting for the owner's confirmation; withdrawing never waits. */
  let pendingGrant = $state<{ client: AgentClient; grant: keyof AgentGrants } | null>(null)
  /**
   * Bumped after every grant decision. The shared Checkbox keeps the state of
   * its own click; re-creating it makes it show what the backend holds, also
   * when the owner cancels the confirmation.
   */
  let grantRender = $state(0)
  let copiedLabel = $state<string | null>(null)
  let destroyed = false

  /** What each grantable scope lets an assistant do, as the panel words it. */
  const GRANT_COPY: Record<
    keyof AgentGrants,
    { label: string; meta: string; confirm: (name: string) => string }
  > = {
    localWrite: {
      label: 'Preparar procesamiento y editar el corpus',
      meta: 'Arma, pausa y cancela lotes; crea y renombra colecciones; edita títulos y metadatos; propone importaciones. No gasta.',
      confirm: (name) =>
        `${name} va a poder armar lotes de procesamiento y pausarlos o cancelarlos, crear y renombrar colecciones, editar títulos y metadatos de tus ítems, y proponer importar archivos de tus carpetas de entrada (cada importación la aprobás vos). Nada de esto gasta: empezar a procesar sigue pidiendo tu aprobación.`,
    },
    remoteSpend: {
      label: 'Pedir procesamiento pago',
      meta: 'Cada pedido se aprueba en EntropIA antes de gastar.',
      confirm: (name) =>
        `${name} va a poder pedir que empiece procesamiento con proveedores pagos (OCR, embeddings, entidades). Cada pedido abre un diálogo en EntropIA y no gasta nada hasta que lo apruebes.`,
    },
  }
  const GRANT_KEYS: (keyof AgentGrants)[] = ['localWrite', 'remoteSpend']

  const recipes = $derived(created ? connectorRecipes(created.client.id, connector) : [])
  const invocation = $derived(created ? connectorInvocation(created.client.id, connector) : '')

  const endpointLabel = $derived.by(() => {
    if (!status) return 'Verificando…'
    if (!status.enabled) return 'Deshabilitado'
    if (status.running && status.port !== null) return `Activo en 127.0.0.1:${status.port}`
    return 'Habilitado, pero el endpoint no está activo'
  })

  onMount(() => {
    void refresh()
    void refreshConnector()
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

  /** Where the connector lives, once: it cannot change while the app runs. */
  async function refreshConnector() {
    try {
      const next = await agentConnectorLocation()
      if (!destroyed) connector = next
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

  /**
   * Granting asks first; withdrawing applies at once, because it only takes a
   * permission away.
   */
  function toggleGrant(client: AgentClient, grant: keyof AgentGrants) {
    if (busy) return
    const current = grantsOf(client)
    if (current[grant]) {
      void applyGrants(client, { ...current, [grant]: false })
    } else {
      pendingGrant = { client, grant }
    }
  }

  function cancelGrant() {
    pendingGrant = null
    grantRender += 1
  }

  async function confirmGrant() {
    const pending = pendingGrant
    if (!pending || busy) return
    await applyGrants(pending.client, { ...grantsOf(pending.client), [pending.grant]: true })
    pendingGrant = null
  }

  async function applyGrants(client: AgentClient, grants: AgentGrants) {
    busy = true
    errorMessage = null
    copiedLabel = null
    try {
      status = await agentClientSetGrants(client.id, grants)
    } catch (error) {
      errorMessage = String(error)
      await refresh()
    } finally {
      busy = false
      grantRender += 1
    }
  }

  /** The owner picks a folder with the system dialog; assistants never name one. */
  async function addImportFolder() {
    if (busy) return
    errorMessage = null
    let picked: string | string[] | null
    try {
      picked = await openFolderDialog({
        directory: true,
        multiple: false,
        title: 'Carpeta de entrada para asistentes',
      })
    } catch (error) {
      errorMessage = String(error)
      return
    }
    const path = Array.isArray(picked) ? picked[0] : picked
    if (!path) return
    busy = true
    try {
      status = await agentImportFolderAdd(path)
    } catch (error) {
      errorMessage = String(error)
    } finally {
      busy = false
    }
  }

  /** Removing only takes a permission away, so it applies at once. */
  async function removeImportFolder(path: string) {
    if (busy) return
    busy = true
    errorMessage = null
    try {
      status = await agentImportFolderRemove(path)
    } catch (error) {
      errorMessage = String(error)
      await refresh()
    } finally {
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
      Permití que un asistente externo consulte este archivo desde esta computadora: el endpoint
      escucha en 127.0.0.1 y nunca en la red local. Un asistente nuevo solo puede leer; abajo podés
      dejarle preparar procesamiento y pedir procesamiento pago, que siempre aprobás acá.
    </p>

    {#if errorMessage}
      <p class="assistants-tab__error" role="alert" data-testid="assistants-error">
        {errorMessage}
      </p>
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
                <div class="assistants-tab__grants">
                  {#each GRANT_KEYS as grant (grant)}
                    <!-- The backend owns the grant: the box shows the status it
                         returns, never the click itself. -->
                    {#key `${grantRender}`}
                      <Checkbox
                        checked={grantsOf(client)[grant]}
                        label="{GRANT_COPY[grant].label}: {client.name}"
                        disabled={busy}
                        onchange={() => toggleGrant(client, grant)}
                      >
                        <span class="assistants-tab__copy">
                          <span class="assistants-tab__value">{GRANT_COPY[grant].label}</span>
                          <span class="assistants-tab__meta">{GRANT_COPY[grant].meta}</span>
                        </span>
                      </Checkbox>
                    {/key}
                  {/each}
                </div>
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
      <span class="assistants-tab__label">Carpetas de entrada</span>
      <p class="assistants-tab__meta">
        Un asistente con permiso para editar el corpus puede proponer importar archivos de estas
        carpetas (no de sus subcarpetas). Cada importación la aprobás vos, viendo la lista de
        archivos.
      </p>
      {#if status && status.importFolders.length > 0}
        <ul class="assistants-tab__folders">
          {#each status.importFolders as folder (folder)}
            <li class="assistants-tab__folder">
              <code class="assistants-tab__code">{folder}</code>
              <Button
                variant="ghost"
                size="sm"
                aria-label="Quitar carpeta {folder}"
                disabled={busy}
                onclick={() => removeImportFolder(folder)}
              >
                Quitar
              </Button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="assistants-tab__value">Ninguna carpeta habilitada.</p>
      {/if}
      <div>
        <Button variant="secondary" size="sm" disabled={!status || busy} onclick={addImportFolder}>
          Agregar carpeta
        </Button>
      </div>
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
          El conector de EntropIA lee la credencial del llavero del sistema y estas líneas no la
          contienen, así que se pueden pegar en la configuración del asistente.
        </p>
        {#if connector?.installed && connector.path}
          <p class="assistants-tab__meta" data-testid="assistants-connector-path">
            Conector instalado en <code>{connector.path}</code>
          </p>
          {#if isWindowsProgram(connector.path)}
            <p class="assistants-tab__meta" data-testid="assistants-connector-powershell">
              Las líneas de Windows son para PowerShell.
            </p>
          {/if}
        {:else}
          <p class="assistants-tab__meta" data-testid="assistants-connector-missing">
            El conector no está instalado en esta compilación: las líneas usan el comando sin ruta.
          </p>
        {/if}
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

{#if pendingGrant}
  <ConfirmDialog
    title="Permitir a {pendingGrant.client.name}: {GRANT_COPY[pendingGrant.grant].label}"
    titleId="assistants-grant-title"
    message={GRANT_COPY[pendingGrant.grant].confirm(pendingGrant.client.name)}
    cancelLabel="Cancelar"
    confirmLabel="Permitir"
    confirming={busy}
    oncancel={cancelGrant}
    onconfirm={confirmGrant}
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

  .assistants-tab__grants {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding-top: var(--space-2);
  }

  .assistants-tab__folders {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .assistants-tab__folder {
    display: flex;
    align-items: center;
    gap: var(--space-2);
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
