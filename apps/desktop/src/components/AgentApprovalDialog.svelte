<script lang="ts">
  /**
   * The owner's answer to paid work an external assistant asked for (D3).
   *
   * An assistant can only ask; this dialog is the one place where the request
   * runs, and only on "Aprobar". The initial focus is on "Rechazar", Escape
   * rejects and a click outside does nothing, so a stray key never spends. The
   * backend binds each request to the job revision the assistant saw and runs
   * it at most once; this component only shows the queue and sends the answer.
   */
  import { onMount } from 'svelte'
  import { listen, type UnlistenFn } from '@tauri-apps/api/event'
  import { ConfirmDialog, portal } from '@entropia/ui'
  import {
    AGENT_APPROVALS_CHANGED_EVENT,
    agentApprovalDecide,
    agentApprovalsPending,
    type AgentApproval,
  } from '$lib/agent-access'

  let pending = $state<AgentApproval[]>([])
  let busy = $state(false)
  let notice = $state<string | null>(null)
  let destroyed = false

  const current = $derived(pending[0] ?? null)

  const OPERATION_LABELS: Record<string, string> = {
    ocr: 'OCR',
    embeddings: 'Embeddings',
    ner: 'Entidades',
    triples: 'Tripletas',
    replace_text: 'Reemplazar texto',
    import_files: 'Importar archivos',
  }

  function operationLabel(operation: string): string {
    if (operation.startsWith('schema:')) return `Esquema ${operation.slice('schema:'.length)}`
    return OPERATION_LABELS[operation] ?? operation
  }

  function actionVerb(approval: AgentApproval): string {
    switch (approval.action.kind) {
      case 'start':
        return 'empezar un procesamiento'
      case 'resume':
        return 'reanudar un procesamiento'
      case 'retry':
        return approval.action.failed_only
          ? 'reintentar las tareas fallidas de un procesamiento'
          : 'reintentar una tarea de un procesamiento'
      case 'replace_text':
        return 'reemplazar el texto de una página'
      case 'import_files':
        return 'importar archivos a una colección'
    }
  }

  /** A file size the way people read it. */
  function sizeLabel(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`
    if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`
  }

  function expiresLabel(approval: AgentApproval): string {
    const time = new Date(approval.expiresAt).toLocaleTimeString([], {
      hour: '2-digit',
      minute: '2-digit',
    })
    return `Si no respondés, el pedido vence a las ${time} y no se ejecuta.`
  }

  /** One sentence for an answer the backend refused. */
  function refusal(error: unknown): string {
    const text = String(error)
    if (text.includes('approval_expired')) {
      return 'El pedido venció antes de que respondieras; no se ejecutó nada.'
    }
    if (text.includes('approval_used')) return 'Ese pedido ya estaba respondido.'
    return `No se pudo responder el pedido: ${text}`
  }

  async function refresh() {
    try {
      const next = await agentApprovalsPending()
      if (!destroyed) pending = Array.isArray(next) ? next : []
    } catch (error) {
      console.error('[AgentApprovalDialog] could not read approvals:', error)
    }
  }

  async function decide(approve: boolean) {
    const approval = current
    if (!approval || busy) return
    busy = true
    notice = null
    try {
      const decided = await agentApprovalDecide(approval.id, approve)
      if (decided?.status === 'failed') {
        notice = decided.failure ?? 'La acción aprobada no se pudo ejecutar; no se ejecutó nada.'
      }
    } catch (error) {
      notice = refusal(error)
    } finally {
      busy = false
      await refresh()
    }
  }

  // A request nobody answers expires in the backend; read again at that moment
  // so the dialog never offers one that can no longer run.
  $effect(() => {
    const approval = current
    if (!approval) return
    const timer = setTimeout(
      () => void refresh(),
      Math.max(0, approval.expiresAt - Date.now()) + 500
    )
    return () => clearTimeout(timer)
  })

  onMount(() => {
    let unlisten: UnlistenFn | null = null
    void listen(AGENT_APPROVALS_CHANGED_EVENT, () => void refresh()).then((stop) => {
      if (destroyed) stop()
      else unlisten = stop
    })
    void refresh()
    return () => {
      destroyed = true
      unlisten?.()
    }
  })
</script>

{#if current}
  <ConfirmDialog
    title="{current.clientName} pide {actionVerb(current)}"
    titleId="agent-approval-title"
    message="Lo pidió un asistente externo conectado a EntropIA. Revisá qué va a procesar antes de aprobar."
    cancelLabel="Rechazar"
    confirmLabel="Aprobar"
    confirming={busy}
    cancelDisabled={busy}
    dismissOnOverlay={false}
    oncancel={() => void decide(false)}
    onconfirm={() => void decide(true)}
  >
    <dl class="agent-approval__facts">
      <dt>Operaciones</dt>
      <dd>{current.operations.map(operationLabel).join(', ')}</dd>
      <dt>Colecciones</dt>
      <dd>{current.collections.length > 0 ? current.collections.join(', ') : '—'}</dd>
      <dt>Alcance</dt>
      <dd>{current.files} {current.files === 1 ? 'archivo' : 'archivos'}</dd>
    </dl>
    {#if current.action.kind === 'import_files'}
      {#if current.importFiles && current.importFiles.length > 0}
        <p class="agent-approval__label">Archivos</p>
        <ul class="agent-approval__files">
          {#each current.importFiles as file (file.name)}
            <li>
              {file.name} <span class="agent-approval__size">{sizeLabel(file.sizeBytes)}</span>
            </li>
          {/each}
        </ul>
      {/if}
      <p class="agent-approval__note">
        No gasta. Cada archivo pasa a ser un documento de la colección; los PDF se dividen en
        páginas. Un archivo que cambie o que ya esté en la colección se saltea.
      </p>
    {:else if current.action.kind === 'replace_text'}
      {#if current.preview}
        <p class="agent-approval__label">Texto propuesto (comienzo)</p>
        <blockquote class="agent-approval__preview">{current.preview}</blockquote>
      {/if}
      <p class="agent-approval__note">
        No gasta. El texto actual queda guardado y «Restaurar original», en la vista del documento,
        lo deshace. Si la página cambia antes de que respondas, no se aplica.
      </p>
    {:else}
      <p class="agent-approval__note">
        Este procesamiento puede usar proveedores pagos. Lo que ya está procesado, o compartido
        desde otro equipo, no se vuelve a pagar.
      </p>
      {#if current.operations.includes('ocr') && !current.operations.includes('embeddings')}
        <!-- Every committed OCR admits a repair embedding (lib.rs processing_commit_observer). -->
        <p class="agent-approval__note">
          Después del OCR, EntropIA calcula los embeddings de cada página reconocida con el
          proveedor configurado, lo que también puede tener costo.
        </p>
      {/if}
    {/if}
    <p class="agent-approval__note">{expiresLabel(current)}</p>
    {#if pending.length > 1}
      <p class="agent-approval__note">
        {pending.length - 1}
        {pending.length - 1 === 1 ? 'pedido más esperando' : 'pedidos más esperando'}.
      </p>
    {/if}
  </ConfirmDialog>
{/if}

{#if notice}
  <div class="agent-approval__notice" role="alert" {@attach portal}>
    <span>{notice}</span>
    <button type="button" class="agent-approval__dismiss" onclick={() => (notice = null)}>
      Cerrar
    </button>
  </div>
{/if}

<style>
  .agent-approval__facts {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    gap: var(--space-1) var(--space-3);
    margin: var(--space-2) 0;
    font-size: var(--font-size-sm);
  }

  .agent-approval__facts dt {
    color: var(--color-text-secondary);
  }

  .agent-approval__facts dd {
    margin: 0;
    color: var(--color-text-primary);
    overflow-wrap: anywhere;
  }

  .agent-approval__note {
    margin: var(--space-1) 0 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .agent-approval__label {
    margin: var(--space-2) 0 var(--space-1);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .agent-approval__preview {
    max-height: 9em;
    margin: 0;
    padding: var(--space-2) var(--space-3);
    overflow: auto;
    border-left: 2px solid var(--color-hairline);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .agent-approval__files {
    max-height: 9em;
    margin: 0;
    padding-left: var(--space-4);
    overflow: auto;
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    overflow-wrap: anywhere;
  }

  .agent-approval__size {
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .agent-approval__notice {
    position: fixed;
    right: var(--space-4);
    bottom: var(--space-4);
    z-index: 1100;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    max-width: 420px;
    padding: var(--space-2) var(--space-3);
    border: 1px solid color-mix(in srgb, var(--color-danger) 32%, var(--color-hairline));
    border-radius: var(--radius-md);
    background: var(--color-surface-elevated);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    box-shadow: var(--shadow-surface);
  }

  .agent-approval__dismiss {
    flex: 0 0 auto;
    border: none;
    background: none;
    color: var(--color-accent);
    font: inherit;
    cursor: pointer;
  }
</style>
