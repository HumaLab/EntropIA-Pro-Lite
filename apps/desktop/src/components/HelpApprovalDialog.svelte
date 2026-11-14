<script lang="ts">
  import { locale, t } from '$lib/i18n'
  import { helpApprovalAnswer, type HelpApprovalRequest } from '$lib/help'
  import { ConfirmDialog } from '@entropia/ui'

  const currentLocale = locale

  /**
   * An inline LOCAL_WRITE approval (T27, D12) shown as a modal dialog, like
   * the MCP approval dialog: the same ConfirmDialog primitive and pattern —
   * portalled, initial focus on «Rechazar», Escape rejects, a click outside
   * does nothing. Controlled by the parent (HelpView): `request` is the live
   * `help://approval` payload, null when nothing waits. Answering reports
   * through `onanswered` so the parent closes the dialog and adds the
   * conversation line («Aprobaste: …» / «Rechazaste: …»); a failed answer
   * keeps the dialog open with the error instead of closing silently. When
   * the run stops or the request times out the parent clears `request` and
   * the dialog disappears without a line (the «Detenido.» answer covers it).
   */
  let {
    request,
    onanswered,
  }: {
    request: HelpApprovalRequest | null
    onanswered: (decision: { id: string; approved: boolean; summary: string }) => void
  } = $props()

  let busy = $state(false)
  let error = $state<string | null>(null)

  // A new request clears the previous answer state.
  $effect(() => {
    if (request) {
      busy = false
      error = null
    }
  })

  async function answer(approved: boolean): Promise<void> {
    const pending = request
    if (!pending || busy) return
    busy = true
    error = null
    try {
      await helpApprovalAnswer(pending.id, approved)
      onanswered({ id: pending.id, approved, summary: pending.summary })
    } catch {
      error = t('help.approvalAnswerError')
    } finally {
      busy = false
    }
  }
</script>

{#if request}
  <ConfirmDialog
    title={$currentLocale && t('help.approvalTitle')}
    message={request.summary}
    cancelLabel={$currentLocale && t('help.reject')}
    confirmLabel={$currentLocale && t('help.approve')}
    confirming={busy}
    cancelDisabled={busy}
    dismissOnOverlay={false}
    {error}
    oncancel={() => void answer(false)}
    onconfirm={() => void answer(true)}
  />
{/if}
