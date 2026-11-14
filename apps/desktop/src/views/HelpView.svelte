<script lang="ts">
  import { onMount } from 'svelte'
  import { getNavigation, getPaneId } from '$lib/pane-context'
  import { locale, t } from '$lib/i18n'
  import { renderMarkdown } from '$lib/markdown'
  import {
    HELP_HISTORY_MAX_TURNS,
    describeHelpStep,
    formatHistoryDate,
    helpAsk,
    helpDiagnosticsPreview,
    helpFaqList,
    helpHistoryAppend,
    helpHistoryClear,
    helpHistoryDelete,
    helpHistoryGet,
    helpHistoryList,
    helpStatus,
    helpStop,
    isNoKeyError,
    listenHelpApproval,
    listenHelpApprovalOutcome,
    listenHelpSteps,
    newHelpConversationId,
    newHelpRequestId,
    openHelpManual,
    type HelpApprovalOutcome,
    type HelpApprovalRequest,
    type HelpDocument,
    type HelpHistoryItem,
    type HelpHistoryMessage,
    type HelpSource,
    type HelpStep,
    type HelpTurn,
  } from '$lib/help'
  import { ActionIcon, Button, Checkbox, ConfirmDialog, tooltip } from '@entropia/ui'
  import HelpApprovalDialog from '../components/HelpApprovalDialog.svelte'

  const navigation = getNavigation()
  const paneId = getPaneId()
  const currentLocale = locale

  /**
   * Compact mode for the floating panel (T15): the panel header replaces the
   * full-page header card, and the view fills the panel body with its own
   * scrolling middle and fixed footer. Standalone use keeps the header card.
   * `maximized` (T28) drives the single-line opt-in row: the panel owns the
   * state (HelpBubble) and the view only renders the contract.
   */
  let { embedded = false, maximized = false }: { embedded?: boolean; maximized?: boolean } =
    $props()

  /** One decided MCP approval following an answer (T26): how it ended
   * and the backend's clear Spanish message naming the target. */
  type HelpOutcomeNotice = {
    approvalId: string
    status: 'approved' | 'rejected' | 'expired' | 'failed'
    message: string
  }

  type HelpMessage = {
    role: 'user' | 'assistant'
    content: string
    sources?: HelpSource[]
    /** True when the backend answered from the bundled FAQ, with no network call (S10). */
    local?: boolean
    /** Titles of the user's documents the agent read for this question (T22). */
    documents?: HelpDocument[]
    /** Every tool call the agent made, in order (T22, T26). */
    steps?: HelpStep[]
    /** True when the round cap forced the final answer (T22). */
    roundLimitReached?: boolean
    /** True when the provider rejected tool use and the answer is plain (T22). */
    toolsUnavailable?: boolean
    /** Decided MCP approvals that followed this answer (T26). */
    notices?: HelpOutcomeNotice[]
  }

  /** The backend's group name for the first-steps starter buttons (S10). */
  const FIRST_STEPS_GROUP = 'Primeros pasos'

  // The conversation lives in component memory only: nothing is stored (D4).
  let status = $state<'loading' | 'no-key' | 'ready'>('loading')
  let messages = $state<HelpMessage[]>([])
  let draft = $state('')
  let sending = $state(false)
  // The run in flight (T24): its frontend-generated id keys the live
  // `help://step` events and `helpStop`. Steps accumulate here as they
  // arrive and are replaced by the answer's final list at the end.
  let liveSteps = $state<HelpStep[]>([])
  let currentRequestId = $state<string | null>(null)
  let stopping = $state(false)
  let error = $state<string | null>(null)
  // Opt-in per question, off by default (S5, D3).
  let includeDiagnostics = $state(false)
  // Opt-in per question, off by default (S12); same reset-after-send pattern.
  let includeDocuments = $state(false)
  let preview = $state<string | null>(null)
  let previewOpen = $state(false)
  let previewLoading = $state(false)
  // Starter buttons from the bundled FAQ (S10); built-in suggestions while
  // the list loads or when it fails.
  let starters = $state<string[]>([])

  const canSend = $derived(status !== 'loading' && !sending && draft.trim().length > 0)

  /** The built-in suggestions, used until the faq list loads or when it fails. */
  function fallbackStarters(): string[] {
    return [t('help.suggestion1'), t('help.suggestion2'), t('help.suggestion3')]
  }

  /** One stored turn back into a panel message (S19): user turns stay
   * plain; assistant turns recover the sources, documents, steps and
   * flags stored in their meta. Notices (T26) and approval lines (T27)
   * were stored as plain assistant text and render as such. */
  function historyMessageToMessage(entry: HelpHistoryMessage): HelpMessage {
    if (entry.role === 'user') return { role: 'user', content: entry.content }
    const meta = (entry.meta ?? {}) as Record<string, unknown>
    const sources = Array.isArray(meta['sources']) ? (meta['sources'] as HelpSource[]) : []
    const documents = Array.isArray(meta['documents']) ? (meta['documents'] as HelpDocument[]) : []
    const steps = Array.isArray(meta['steps']) ? (meta['steps'] as HelpStep[]) : []
    return {
      role: 'assistant',
      content: entry.content,
      sources,
      documents,
      steps,
      local: meta['local'] === true,
      roundLimitReached: meta['roundLimitReached'] === true,
      toolsUnavailable: meta['toolsUnavailable'] === true,
    }
  }

  // The live inline approval (T27, D12): the `help://approval` payload the
  // backend waits on. Rendered as a modal HelpApprovalDialog, never as a
  // card in the conversation; answering adds a short line instead.
  let pendingApproval = $state<HelpApprovalRequest | null>(null)
  let destroyed = false

  /** One decided inline approval (T27): the dialog closes and a short line
   * stays in the conversation («Aprobaste: …» / «Rechazaste: …»). */
  function handleApprovalDecided(decision: {
    id: string
    approved: boolean
    summary: string
  }): void {
    pendingApproval = null
    const line = decision.approved
      ? t('help.approvalApproved', { summary: decision.summary })
      : t('help.approvalRejected', { summary: decision.summary })
    messages = [...messages, { role: 'assistant', content: line }]
    // T27: the line joins the stored conversation while the run is still
    // in flight, so a reopened conversation keeps it (S19).
    if (conversationId) {
      helpHistoryAppend(conversationId, 'assistant', line).catch(() => {
        error = t('help.historyError')
      })
    }
  }

  /** The mark shown before an outcome notice, in the UI language. */
  function outcomeMark(status: HelpOutcomeNotice['status']): string {
    if (status === 'approved') return $currentLocale && t('help.approvalOutcomeApproved')
    if (status === 'rejected') return $currentLocale && t('help.approvalOutcomeRejected')
    if (status === 'expired') return $currentLocale && t('help.approvalOutcomeExpired')
    return $currentLocale && t('help.approvalOutcomeFailed')
  }

  /** Applies one decided MCP approval (T26): the notice goes right after
   * the answer that requested it — found by the step carrying its
   * approval id — and that step flips from pending to its final state.
   * The original answer text stays (it was true when written); the notice
   * makes the final state clear. An outcome for no known answer is
   * ignored, never pinned to the wrong one. */
  function applyApprovalOutcome(outcome: HelpApprovalOutcome): void {
    const stepStatus: HelpStep['status'] =
      outcome.status === 'approved'
        ? 'ok'
        : outcome.status === 'rejected'
          ? 'denied'
          : outcome.status === 'expired'
            ? 'expired'
            : 'failed'
    let changed = false
    messages = messages.map((message) => {
      if (message.role !== 'assistant' || !message.steps) return message
      const index = message.steps.findIndex((step) => step.approvalId === outcome.approvalId)
      if (index === -1) return message
      if (message.notices?.some((notice) => notice.approvalId === outcome.approvalId)) {
        return message
      }
      const steps = message.steps.map((step, at) =>
        at === index ? { ...step, status: stepStatus } : step
      )
      changed = true
      return {
        ...message,
        steps,
        notices: [
          ...(message.notices ?? []),
          { approvalId: outcome.approvalId, status: outcome.status, message: outcome.message },
        ],
      }
    })
    if (!changed) return
  }

  // The stored conversation (S19, D17): created with the first
  // question, continued by every following one. Null until the first
  // send; «Nueva conversación» clears it back to null.
  let conversationId = $state<string | null>(null)
  // «Historial» (S19): collapsed by default, never persisted open —
  // every mount starts collapsed.
  let historyOpen = $state(false)
  let conversations = $state<HelpHistoryItem[]>([])
  let historyLoading = $state(false)
  let historyError = $state<string | null>(null)
  let clearingHistory = $state(false)

  /** Reloads the «Historial» list, silently: the count on the toggle
   * stays current after every answer and every deletion. */
  async function refreshHistory(): Promise<void> {
    try {
      conversations = (await helpHistoryList()) ?? []
    } catch {
      conversations = []
      historyError = t('help.historyError')
    }
  }

  /** Loads a stored conversation into the panel (S19): following
   * questions continue it. The list collapses once it is open. */
  async function openConversation(id: string): Promise<void> {
    if (sending) return
    historyLoading = true
    historyError = null
    try {
      const detail = await helpHistoryGet(id)
      conversationId = detail.id
      messages = (detail.messages ?? []).map(historyMessageToMessage)
      error = null
      historyOpen = false
    } catch {
      historyError = t('help.historyError')
    } finally {
      historyLoading = false
    }
  }

  /** Starts an empty conversation (S19): the next question creates it. */
  function startNewConversation(): void {
    if (sending) return
    messages = []
    conversationId = null
    error = null
  }

  /** Soft-deletes one stored conversation (S19); deleting the open one
   * clears the panel. */
  async function deleteConversation(id: string): Promise<void> {
    if (sending) return
    try {
      await helpHistoryDelete(id)
      if (id === conversationId) {
        messages = []
        conversationId = null
      }
      await refreshHistory()
    } catch {
      historyError = t('help.historyError')
    }
  }

  /** Soft-deletes every stored conversation (S19), after confirmation. */
  async function confirmClearHistory(): Promise<void> {
    clearingHistory = false
    try {
      await helpHistoryClear()
      messages = []
      conversationId = null
      error = null
      await refreshHistory()
    } catch {
      historyError = t('help.historyError')
    }
  }

  onMount(() => {
    let unlisten: (() => void) | null = null
    let unlistenSteps: (() => void) | null = null
    let unlistenOutcomes: (() => void) | null = null
    void listenHelpApproval((request) => {
      pendingApproval = request
    }).then((stop) => {
      if (destroyed) stop()
      else unlisten = stop
    })
    // Live agent steps (T24, T26): only the run in flight renders them;
    // a late step from a settled run is ignored. A pending step already
    // carries its human target and approval id.
    void listenHelpSteps((step) => {
      if (currentRequestId && step.requestId === currentRequestId) {
        liveSteps = [
          ...liveSteps,
          {
            tool: step.tool,
            args: step.args,
            status: step.status,
            ...(step.target === undefined ? {} : { target: step.target }),
            ...(step.approvalId === undefined ? {} : { approvalId: step.approvalId }),
          },
        ]
      }
    }).then((stopSteps) => {
      if (destroyed) stopSteps()
      else unlistenSteps = stopSteps
    })
    // Decided MCP approvals (T26): the backend follows them after the
    // answer and reports each outcome once.
    void listenHelpApprovalOutcome((outcome) => {
      applyApprovalOutcome(outcome)
    }).then((stopOutcomes) => {
      if (destroyed) stopOutcomes()
      else unlistenOutcomes = stopOutcomes
    })
    void helpStatus().then(
      (result) => {
        status = result.hasOpenrouterKey ? 'ready' : 'no-key'
      },
      () => {
        // A failed check is not a missing key: let the user ask, and let
        // help_ask report the real cause (it switches to the no-key page on
        // help_no_openrouter_key).
        status = 'ready'
      }
    )
    // Starter buttons need no key: the bundled FAQ answers locally (S10).
    starters = fallbackStarters()
    void refreshHistory()
    void helpFaqList().then(
      (items) => {
        const firstSteps = items
          .filter((item) => item.group === FIRST_STEPS_GROUP)
          .map((item) => item.question)
        if (firstSteps.length > 0) starters = firstSteps
      },
      () => {
        starters = fallbackStarters()
      }
    )
    return () => {
      destroyed = true
      unlisten?.()
      unlistenSteps?.()
      unlistenOutcomes?.()
    }
  })

  /**
   * Sends the composer's draft, or `direct` when a starter button asks its
   * question at once (S18): then the draft the user was typing is left alone.
   */
  async function send(direct?: string): Promise<void> {
    const fromDraft = direct === undefined
    const question = (direct ?? draft).trim()
    // The input stays usable with no key (S6, S10): local answers still
    // work, and help_ask reports the key rejection for the rest.
    if (!question || sending || status === 'loading') return
    sending = true
    error = null
    // S19: the conversation is created with the first question; every
    // following question continues it.
    if (conversationId === null) conversationId = newHelpConversationId()
    // One id per question (T24): the live steps and `helpStop` belong to it.
    const requestId = newHelpRequestId()
    currentRequestId = requestId
    liveSteps = []
    stopping = false
    let diagnostics: string | null = null
    if (includeDiagnostics) {
      // Read once, then back off: every send starts unsubscribed again (S5).
      includeDiagnostics = false
      try {
        diagnostics = preview ?? (await helpDiagnosticsPreview())
        preview = diagnostics
      } catch {
        error = t('help.diagnosticsError')
        sending = false
        return
      }
    }
    // An answer built from the user's documents (S12) never travels again as
    // history: its consent covered that one question only.
    const history: HelpTurn[] = messages
      .filter((message) => !(message.documents && message.documents.length > 0))
      .slice(-HELP_HISTORY_MAX_TURNS)
      .map((message) => ({ role: message.role, content: message.content }))
    messages = [...messages, { role: 'user', content: question }]
    if (fromDraft) draft = ''
    // Same pattern as diagnostics (S5): every send starts unsubscribed again.
    const withDocuments = includeDocuments
    includeDocuments = false
    try {
      const answer = await helpAsk(question, history, diagnostics, {
        ...(withDocuments ? { documents: true as const } : {}),
        requestId,
        conversationId,
      })
      if (answer.conversationId) conversationId = answer.conversationId
      messages = [
        ...messages,
        {
          role: 'assistant',
          content: answer.answer,
          sources: answer.sources,
          local: answer.local,
          documents: answer.documents ?? [],
          steps: answer.steps ?? [],
          roundLimitReached: answer.roundLimitReached ?? false,
          toolsUnavailable: answer.toolsUnavailable ?? false,
        },
      ]
      void refreshHistory()
    } catch (sendError) {
      if (isNoKeyError(sendError)) {
        // No local answer and no key: show the key message (S6) instead of
        // a generic error. The input stays for a local question.
        error = t('help.noKeyBody')
        if (fromDraft) draft = question
        return
      }
      error = t('help.sendError')
      if (fromDraft) draft = question
    } finally {
      sending = false
      stopping = false
      // The answer's final steps replace the live list (same content).
      currentRequestId = null
      liveSteps = []
      // The run settled while an inline approval was still open («Detener»
      // or the request timing out): the dialog disappears with the run, and
      // the answer above («Detenido.» or the denial) says what happened.
      pendingApproval = null
    }
  }

  /** «Detener» (T24): ends the run in flight; its answer keeps the steps
   * done so far. The button stays put until the run settles. */
  async function stop(): Promise<void> {
    const requestId = currentRequestId
    if (!requestId || !sending || stopping) return
    stopping = true
    try {
      await helpStop(requestId)
    } catch {
      stopping = false
      error = t('help.sendError')
    }
  }

  async function togglePreview(): Promise<void> {
    previewOpen = !previewOpen
    if (previewOpen && preview === null && !previewLoading) {
      previewLoading = true
      try {
        preview = await helpDiagnosticsPreview()
      } catch {
        previewOpen = false
        error = t('help.diagnosticsError')
      } finally {
        previewLoading = false
      }
    }
  }

  function handleComposerKeydown(event: KeyboardEvent) {
    // keyCode 229 cubre WKWebView, donde isComposing puede no reportarse durante IME.
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing && event.keyCode !== 229) {
      event.preventDefault()
      void send()
    }
  }
</script>

<div class="help-view page-shell" class:help-view--embedded={embedded}>
  {#if !embedded}
    <section class="page-header help-view__header" aria-labelledby="help-title-{paneId}">
      <div class="page-header__content">
        <h1 id="help-title-{paneId}">{$currentLocale && t('help.title')}</h1>
        <p>{$currentLocale && t('help.subtitle')}</p>
      </div>
    </section>
  {/if}

  {#if status === 'loading'}
    <p class="help-view__state" role="status">{$currentLocale && t('help.statusLoading')}</p>
  {:else}
    {#if status === 'no-key'}
      <section class="help-view__no-key" aria-label={$currentLocale && t('help.noKeyTitle')}>
        <p class="help-view__no-key-title">{$currentLocale && t('help.noKeyTitle')}</p>
        <p class="help-view__no-key-body">{$currentLocale && t('help.noKeyBody')}</p>
        <div class="help-view__no-key-actions">
          <Button
            variant="primary"
            onclick={() => navigation.openRootSection({ name: 'settings' })}
          >
            {$currentLocale && t('help.openSettings')}
          </Button>
          <Button variant="secondary" onclick={() => void openHelpManual()}>
            <ActionIcon name="external-link" size={16} />
            {$currentLocale && t('help.openManual')}
          </Button>
        </div>
      </section>
    {/if}
    <!-- Stored conversations (S19, D17): collapsed by default, never
      persisted open; opening one loads it into the panel and the next
      questions continue it. -->
    <div class="help-view__history">
      <div class="help-view__history-row">
        <Button
          variant="secondary"
          size="sm"
          aria-expanded={historyOpen}
          onclick={() => (historyOpen = !historyOpen)}
        >
          <ActionIcon name="history" size={14} />
          {$currentLocale && t('help.historyCount', { count: conversations.length })}
        </Button>
        <Button variant="secondary" size="sm" onclick={() => startNewConversation()}>
          {$currentLocale && t('help.newConversation')}
        </Button>
      </div>
      {#if historyOpen}
        {#if historyLoading}
          <p class="help-view__state" role="status">
            {$currentLocale && t('help.historyLoading')}
          </p>
        {:else if historyError}
          <p class="help-view__error" role="alert">{historyError}</p>
        {:else if conversations.length === 0}
          <p class="help-view__state">{$currentLocale && t('help.historyEmpty')}</p>
        {:else}
          <ul class="help-view__history-list">
            {#each conversations as conv (conv.id)}
              <li class="help-view__history-item">
                <button
                  type="button"
                  class="help-view__history-open"
                  class:help-view__history-open--current={conv.id === conversationId}
                  aria-current={conv.id === conversationId ? 'true' : undefined}
                  onclick={() => void openConversation(conv.id)}
                >
                  <span class="help-view__history-title">{conv.title}</span>
                  <span class="help-view__history-date">
                    {formatHistoryDate(
                      conv.updatedAt,
                      $currentLocale,
                      $currentLocale && t('help.historyNow')
                    )}
                  </span>
                </button>
                <Button
                  variant="secondary"
                  iconOnly
                  size="sm"
                  aria-label={$currentLocale && t('help.deleteConversation')}
                  title={$currentLocale && t('help.deleteConversation')}
                  onclick={() => void deleteConversation(conv.id)}
                >
                  <ActionIcon name="delete" size={20} />
                </Button>
              </li>
            {/each}
          </ul>
          <Button variant="secondary" size="sm" onclick={() => (clearingHistory = true)}>
            {$currentLocale && t('help.clearHistory')}
          </Button>
        {/if}
      {/if}
    </div>
    <div
      class="help-view__messages"
      role="log"
      aria-live="polite"
      aria-label={$currentLocale && t('help.title')}
    >
      {#if messages.length === 0 && !sending}
        <p class="help-view__empty">{$currentLocale && t('help.emptyState')}</p>
        <div class="help-view__suggestions">
          <p class="help-view__suggestions-label">{$currentLocale && t('help.suggestionsLabel')}</p>
          <div class="help-view__suggestions-list">
            {#each starters as starter (starter)}
              <Button variant="secondary" size="sm" onclick={() => void send(starter)}>
                {starter}
              </Button>
            {/each}
          </div>
        </div>
      {/if}

      {#each messages as message, index (index)}
        {#if message.role === 'user'}
          <p class="help-view__bubble help-view__bubble--user">{message.content}</p>
        {:else}
          <article class="help-view__bubble help-view__bubble--assistant">
            <div class="help-view__markdown">
              <!-- markdown: renderMarkdown escapes all HTML before emitting tags -->
              <!-- eslint-disable-next-line svelte/no-at-html-tags -->
              {@html renderMarkdown(message.content)}
            </div>
            {#if message.local}
              <p class="help-view__local">{$currentLocale && t('help.localAnswer')}</p>
            {/if}
            {#if message.sources && message.sources.length > 0}
              <ul class="help-view__sources" aria-label={$currentLocale && t('help.sourcesLabel')}>
                {#each message.sources as source (source.chapter + source.title)}
                  <li>
                    {$currentLocale &&
                      (source.kind === 'screenMap'
                        ? t('help.sourceScreenMap')
                        : t('help.sourceManual'))}: {source.chapter} › {source.title}
                  </li>
                {/each}
              </ul>
            {/if}
            {#if message.documents && message.documents.length > 0}
              <p class="help-view__doc-titles">
                {$currentLocale && t('help.documentsLabel')}: {message.documents
                  .map((document) => document.title)
                  .join('; ')}
              </p>
            {/if}
            {#if message.steps && message.steps.length > 0}
              <p class="help-view__steps">
                {$currentLocale && t('help.stepsUsed')}: {message.steps
                  .map((step) => describeHelpStep(step, $currentLocale))
                  .join(', ')}
              </p>
            {/if}
            {#if message.notices && message.notices.length > 0}
              <ul class="help-view__notices" aria-live="polite">
                {#each message.notices as notice (notice.approvalId)}
                  <li class="help-view__notice help-view__notice--{notice.status}">
                    <span class="help-view__notice-mark">{outcomeMark(notice.status)}</span>
                    <span>{notice.message}</span>
                  </li>
                {/each}
              </ul>
            {/if}
            {#if message.roundLimitReached}
              <p class="help-view__note">{$currentLocale && t('help.roundLimitReached')}</p>
            {/if}
            {#if message.toolsUnavailable}
              <p class="help-view__note">{$currentLocale && t('help.toolsUnavailable')}</p>
            {/if}
          </article>
        {/if}
      {/each}

      {#if sending}
        <p class="help-view__state" role="status">{$currentLocale && t('help.thinking')}</p>
        {#if liveSteps.length > 0}
          <ul class="help-view__live-steps" aria-label={$currentLocale && t('help.stepsUsed')}>
            {#each liveSteps as liveStep, index (index)}
              <li>{describeHelpStep(liveStep, $currentLocale)}</li>
            {/each}
          </ul>
        {/if}
      {/if}

      {#if previewOpen}
        {#if previewLoading}
          <p class="help-view__state" role="status">
            {$currentLocale && t('help.diagnosticsLoading')}
          </p>
        {:else if preview !== null}
          <pre class="help-view__preview">{preview}</pre>
        {/if}
      {/if}
    </div>

    {#if error}
      <p class="help-view__error" role="alert">{error}</p>
    {/if}

    <form
      class="help-view__composer"
      onsubmit={(event) => {
        event.preventDefault()
        void send()
      }}
    >
      <div class="help-view__optins" class:help-view__optins--single={maximized}>
        <div class="help-view__optin">
          <Checkbox
            bind:checked={includeDiagnostics}
            disabled={sending}
            label={$currentLocale && t('help.includeDiagnostics')}
          >
            {$currentLocale && t('help.includeDiagnostics')}
          </Checkbox>
          <button
            type="button"
            class="help-view__info"
            aria-label={$currentLocale && t('help.diagnosticsInfo')}
            aria-describedby="help-diagnostics-hint-{paneId}"
            use:tooltip={$currentLocale && t('help.diagnosticsHint')}
          >
            <ActionIcon name="info" size={12} />
          </button>
          <span id="help-diagnostics-hint-{paneId}" class="help-view__visually-hidden">
            {$currentLocale && t('help.diagnosticsHint')}
          </span>
          <button
            type="button"
            class="help-view__preview-toggle"
            onclick={() => void togglePreview()}
          >
            {$currentLocale && (previewOpen ? t('help.previewHide') : t('help.previewToggle'))}
          </button>
        </div>
        <div class="help-view__optin">
          <Checkbox
            bind:checked={includeDocuments}
            disabled={sending}
            label={$currentLocale && t('help.includeDocuments')}
          >
            {$currentLocale && t('help.includeDocuments')}
          </Checkbox>
          <button
            type="button"
            class="help-view__info"
            aria-label={$currentLocale && t('help.documentsInfo')}
            aria-describedby="help-documents-hint-{paneId}"
            use:tooltip={$currentLocale && t('help.documentsHint')}
          >
            <ActionIcon name="info" size={12} />
          </button>
          <span id="help-documents-hint-{paneId}" class="help-view__visually-hidden">
            {$currentLocale && t('help.documentsHint')}
          </span>
        </div>
      </div>
      <div class="help-view__input-row">
        <textarea
          class="help-view__input"
          rows="2"
          bind:value={draft}
          placeholder={$currentLocale && t('help.placeholder')}
          aria-label={$currentLocale && t('help.placeholder')}
          onkeydown={handleComposerKeydown}
          disabled={sending}></textarea>
        {#if sending}
          <Button
            variant="primary"
            type="button"
            aria-label={$currentLocale && t('help.stop')}
            title={$currentLocale && t('help.stop')}
            disabled={stopping}
            onclick={() => void stop()}
          >
            {$currentLocale && t('help.stop')}
          </Button>
        {:else}
          <Button
            variant="primary"
            iconOnly
            type="submit"
            aria-label={$currentLocale && t('help.send')}
            title={$currentLocale && t('help.send')}
            disabled={!canSend}
          >
            <ActionIcon name="send" size={20} />
          </Button>
        {/if}
      </div>
    </form>

    <!-- Inline LOCAL_WRITE approval (T27, D12): a modal dialog like the MCP
      one, portalled out of the panel so it shows even maximized. Answering
      adds a short line to the conversation above. -->
    <HelpApprovalDialog request={pendingApproval} onanswered={handleApprovalDecided} />

    <!-- «Borrar historial» (S19): soft-deletes every stored conversation,
      after confirmation. -->
    {#if clearingHistory}
      <ConfirmDialog
        title={$currentLocale && t('help.clearHistoryTitle')}
        message={$currentLocale && t('help.clearHistoryMessage')}
        cancelLabel={$currentLocale && t('help.cancel')}
        confirmLabel={$currentLocale && t('help.confirmDelete')}
        variant="destructive"
        oncancel={() => (clearingHistory = false)}
        onconfirm={() => void confirmClearHistory()}
      />
    {/if}
  {/if}
</div>

<style>
  .help-view {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }

  /* Compact mode inside the floating panel (T15): the panel header replaces
     the full-page header card, so the view only needs a tighter gap here. */
  .help-view--embedded {
    gap: var(--space-2);
    padding-block-end: 0;
  }

  .help-view__header {
    flex-shrink: 0;
  }

  .help-view__state {
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .help-view__no-key {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    max-width: 36rem;
  }

  .help-view__no-key-title {
    margin: 0;
    color: var(--color-text-primary);
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
  }

  .help-view__no-key-body {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .help-view__no-key-actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .help-view__messages {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-2) var(--space-1);
  }

  .help-view__empty {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .help-view__suggestions {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .help-view__suggestions-label {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .help-view__suggestions-list {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .help-view__bubble {
    max-width: 75%;
    min-width: 0;
    box-sizing: border-box;
    margin: 0;
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-dialog);
    overflow-wrap: anywhere;
  }

  .help-view__bubble--user {
    align-self: flex-end;
    background: color-mix(in srgb, var(--color-accent) 14%, var(--color-surface-glass));
    border-color: color-mix(in srgb, var(--color-accent) 24%, var(--border-subtle));
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    white-space: pre-wrap;
  }

  .help-view__bubble--assistant {
    align-self: flex-start;
    background: var(--surface-panel);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
  }

  .help-view__markdown {
    white-space: normal;
  }

  .help-view__markdown :global(p) {
    margin: 0;
  }

  .help-view__markdown :global(p + p),
  .help-view__markdown :global(p + ul),
  .help-view__markdown :global(p + ol),
  .help-view__markdown :global(ul + p),
  .help-view__markdown :global(ol + p) {
    margin-top: var(--space-2);
  }

  .help-view__markdown :global(ul),
  .help-view__markdown :global(ol) {
    margin: 0;
    padding-left: var(--space-4);
  }

  .help-view__markdown :global(li) {
    margin-block: var(--space-1);
  }

  .help-view__markdown :global(strong) {
    font-weight: var(--font-weight-semibold);
  }

  .help-view__markdown :global(em) {
    font-style: italic;
  }

  .help-view__markdown :global(code) {
    padding: 0.1em 0.35em;
    border-radius: var(--radius-sm);
    background: var(--surface-toolbar);
    font-family: var(--font-mono);
    font-size: 0.9em;
  }

  .help-view__markdown :global(a) {
    color: var(--color-accent);
    text-decoration: underline;
    text-underline-offset: 0.15em;
  }

  .help-view__sources {
    margin: var(--space-2) 0 0;
    padding: 0;
    list-style: none;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .help-view__local {
    margin: var(--space-2) 0 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    font-style: italic;
  }

  .help-view__doc-titles {
    margin: var(--space-2) 0 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .help-view__steps {
    margin: var(--space-2) 0 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .help-view__live-steps {
    margin: 0;
    padding-left: var(--space-4);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .help-view__live-steps li {
    margin-block: var(--space-1);
  }

  .help-view__note {
    margin: var(--space-2) 0 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    font-style: italic;
  }

  /* Decided MCP approvals (T26): one short assistant-side notice right
     after the answer that requested it, with a success/rejected/failed
     mark. The original answer text stays above. */
  .help-view__notices {
    margin: var(--space-2) 0 0;
    padding: 0;
    list-style: none;
  }

  .help-view__notices li + li {
    margin-top: var(--space-1);
  }

  .help-view__notice {
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .help-view__notice-mark {
    font-weight: var(--font-weight-semibold);
  }

  .help-view__notice--approved .help-view__notice-mark {
    color: var(--color-success, var(--color-accent));
  }

  .help-view__notice--rejected .help-view__notice-mark,
  .help-view__notice--failed .help-view__notice-mark {
    color: var(--color-danger);
  }

  .help-view__sources li + li {
    margin-top: var(--space-1);
  }

  .help-view__error {
    margin: 0;
    color: var(--color-danger);
    font-size: var(--font-size-sm);
  }

  .help-view__composer {
    display: flex;
    flex-shrink: 0;
    flex-direction: column;
    gap: var(--space-2);
    padding-top: var(--space-2);
  }

  /* Opt-ins (T28): one wrapping row by default; a single line only with the
     maximized contract below. The maximized panel is wide enough for both. */
  .help-view__optins {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-1) var(--space-3);
  }

  .help-view__optins--single {
    flex-wrap: nowrap;
    align-items: center;
    overflow-x: auto;
  }

  .help-view__optin {
    display: flex;
    align-items: baseline;
    gap: var(--space-1);
    min-width: 0;
  }

  /* The small superscript hint (T28): the hint paragraph is gone and the
     sentence lives on this button's tooltip, its accessible name and the
     description below. Inline, so it never forces the row to wrap. A button
     (no action on press) so keyboard focus reaches it and shows the
     tooltip; the label beside it keeps the checkbox meaning. */
  .help-view__info {
    display: inline-flex;
    flex-shrink: 0;
    padding: 0;
    border: none;
    background: none;
    font: inherit;
    vertical-align: super;
    color: var(--color-text-muted);
    cursor: help;
  }

  .help-view__info:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .help-view__visually-hidden {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
    border: 0;
  }

  .help-view__preview-toggle {
    padding: 0;
    border: none;
    background: none;
    color: var(--color-accent);
    font: inherit;
    font-size: var(--font-size-sm);
    text-decoration: underline;
    text-underline-offset: 0.15em;
    cursor: pointer;
  }

  /* Bounded (T15): its own scroll, so a long dump scrolls inside the
     messages middle instead of pushing the footer out of the panel. */
  .help-view__preview {
    flex-shrink: 0;
    max-height: 12rem;
    overflow: auto;
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    background: var(--surface-input);
    color: var(--color-text-secondary);
    font-family: var(--font-mono);
    font-size: var(--font-size-xs);
    white-space: pre-wrap;
  }

  /* Stored conversations (S19): collapsed by default, above the
     conversation; the list scrolls on its own. */
  .help-view__history {
    display: flex;
    flex-shrink: 0;
    flex-direction: column;
    gap: var(--space-2);
  }

  .help-view__history-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .help-view__history-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-height: 12rem;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }

  .help-view__history-item {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }

  .help-view__history-open {
    display: flex;
    flex: 1;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
    min-width: 0;
    padding: var(--space-1) var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-input);
    background: none;
    color: var(--color-text-primary);
    font: inherit;
    font-size: var(--font-size-sm);
    text-align: left;
    cursor: pointer;
  }

  .help-view__history-open--current {
    border-color: var(--color-accent);
  }

  .help-view__history-open:focus-visible {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
  }

  .help-view__history-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .help-view__history-date {
    flex-shrink: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .help-view__input-row {
    display: flex;
    align-items: flex-end;
    gap: var(--space-2);
  }

  .help-view__input {
    flex: 1;
    min-width: 0;
    min-height: var(--control-height-md);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-input);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font: inherit;
    font-size: var(--font-size-sm);
    resize: vertical;
  }

  .help-view__input:focus {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
  }
</style>
