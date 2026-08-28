<script lang="ts">
  import { onDestroy, tick } from 'svelte'
  import { navigation } from '$lib/navigation'
  import { locale, t, type Locale } from '$lib/i18n'
  import { ragSearchConversations, type RagConversationSummary, type RagSource } from '$lib/rag'
  import { downloadRagConversationPdf } from '$lib/rag-chat-export'
  import { ragChat, type UiMessage } from '$lib/rag-chat'
  import { renderMarkdown } from '$lib/markdown'
  import { ActionIcon, Button, ConfirmDialog, IconButton, Panel, SearchClearButton } from '@entropia/ui'

  let messagesEl = $state<HTMLDivElement | undefined>()
  let conversationSearchInput = $state<HTMLInputElement | undefined>()
  let pendingDeleteId = $state<string | null>(null)
  type ActionFeedback = 'success' | 'error'
  type DownloadFeedback = 'loading' | 'success' | 'error'

  let copyFeedback = $state<{ messageIndex: number; tone: ActionFeedback } | null>(null)
  let copyFeedbackTimer: ReturnType<typeof setTimeout> | null = null
  let copyAttemptGeneration = 0
  let conversationSearchOpen = $state(false)
  let conversationQuery = $state('')
  let conversationSearchResults = $state<RagConversationSummary[] | null>(null)
  let conversationSearchError = $state<string | null>(null)
  let conversationSearchLoading = $state(false)
  let conversationSearchTimer: ReturnType<typeof setTimeout> | null = null
  let conversationSearchRequest = 0
  let downloadFeedback = $state<Record<string, DownloadFeedback>>({})
  let downloadFeedbackTimers = new Map<string, ReturnType<typeof setTimeout>>()

  let visibleConversations = $derived(
    conversationQuery.trim()
      ? (conversationSearchResults ?? $ragChat.conversations).filter((conversation) =>
          $ragChat.conversations.some((canonical) => canonical.id === conversation.id),
        )
      : $ragChat.conversations,
  )

  const currentLocale = locale
  const canSend = $derived(!$ragChat.loading && $ragChat.draft.trim().length > 0)

  $effect(() => {
    void ragChat.initialize()
  })
  let previousConversationId: string | null | undefined
  let previousMessageCount = -1

  $effect(() => {
    const conversationId = $ragChat.activeConversationId
    const messageCount = $ragChat.messages.length
    const contextChanged =
      (previousConversationId !== undefined && conversationId !== previousConversationId) ||
      (messageCount === 0 && previousMessageCount > 0)
    if (contextChanged) clearCopyFeedback()
    previousConversationId = conversationId
    previousMessageCount = messageCount
  })

  function formatTimestamp(seconds: number): string {
    const total = Math.max(0, Math.floor(seconds))
    const minutes = Math.floor(total / 60)
    const rest = total % 60
    return `${minutes}:${String(rest).padStart(2, '0')}`
  }

  function sourceTimestamp(source: RagSource): string | null {
    if (source.startSeconds == null) return null
    const start = formatTimestamp(source.startSeconds)
    if (source.endSeconds == null) return start
    return `${start}–${formatTimestamp(source.endSeconds)}`
  }

  function messageContent(message: UiMessage): string {
    const isEmptyAnswer =
      message.role === 'assistant' &&
      message.content.trim() === '' &&
      (message.sources?.length ?? 0) === 0
    return isEmptyAnswer ? t('ragChat.noResults') : message.content
  }

  function formatConversationDate(timestamp: number, activeLocale: Locale): string {
    return new Date(timestamp).toLocaleDateString(activeLocale)
  }

  function handleSend() {
    void ragChat.send($ragChat.draft)
  }

  function handleComposerKeydown(event: KeyboardEvent) {
    // keyCode 229 cubre WKWebView, donde isComposing puede no reportarse durante IME.
    if (event.key === 'Enter' && !event.shiftKey && !event.isComposing && event.keyCode !== 229) {
      event.preventDefault()
      handleSend()
    }
  }

  function handleDeleteConfirm() {
    const conversationId = pendingDeleteId
    pendingDeleteId = null
    if (conversationId) {
      void ragChat.remove(conversationId)
    }
  }

  function openSource(source: RagSource) {
    navigation.navigate({
      name: 'item',
      collectionId: source.collectionId,
      collectionName: source.collectionName,
      itemId: source.itemId,
      itemTitle: source.itemTitle,
      assetId: source.assetId,
    })
  }
  function clearCopyFeedback() {
    if (copyFeedbackTimer) clearTimeout(copyFeedbackTimer)
    copyFeedbackTimer = null
    copyFeedback = null
  }

  function showCopyFeedback(messageIndex: number, tone: ActionFeedback) {
    if (copyFeedbackTimer) clearTimeout(copyFeedbackTimer)
    copyFeedback = { messageIndex, tone }
    copyFeedbackTimer = setTimeout(() => {
      if (copyFeedback?.messageIndex === messageIndex) copyFeedback = null
      copyFeedbackTimer = null
    }, 1500)
  }

  async function copyResponse(message: UiMessage, messageIndex: number) {
    const copyAttempt = ++copyAttemptGeneration
    const copyContext = {
      conversationId: $ragChat.activeConversationId,
      message,
    }
    const isCurrentCopy = () =>
      copyAttempt === copyAttemptGeneration &&
      $ragChat.activeConversationId === copyContext.conversationId &&
      $ragChat.messages[messageIndex] === copyContext.message

    try {
      await navigator.clipboard.writeText(message.content)
      if (isCurrentCopy()) showCopyFeedback(messageIndex, 'success')
    } catch {
      if (isCurrentCopy()) showCopyFeedback(messageIndex, 'error')
    }
  }

  function scheduleConversationSearch(value: string) {
    conversationQuery = value
    conversationSearchError = null
    conversationSearchRequest += 1
    const request = conversationSearchRequest
    if (conversationSearchTimer) clearTimeout(conversationSearchTimer)

    if (!value.trim()) {
      conversationSearchResults = null
      conversationSearchLoading = false
      return
    }

    conversationSearchLoading = true
    conversationSearchTimer = setTimeout(async () => {
      conversationSearchTimer = null
      try {
        const results = await ragSearchConversations(value)
        if (request !== conversationSearchRequest) return
        conversationSearchResults = results
        conversationSearchLoading = false
      } catch {
        if (request !== conversationSearchRequest) return
        conversationSearchLoading = false
        conversationSearchError = t('ragChat.searchConversationsError')
      }
    }, 250)
  }

  function clearConversationSearch() {
    scheduleConversationSearch('')
  }

  async function openConversationSearch() {
    conversationSearchOpen = true
    await tick()
    conversationSearchInput?.focus()
  }

  function closeConversationSearch() {
    conversationSearchRequest += 1
    if (conversationSearchTimer) clearTimeout(conversationSearchTimer)
    conversationSearchTimer = null
    conversationSearchOpen = false
    conversationQuery = ''
    conversationSearchResults = null
    conversationSearchError = null
    conversationSearchLoading = false
  }

  function toggleConversationSearch() {
    if (conversationSearchOpen) closeConversationSearch()
    else void openConversationSearch()
  }

  async function downloadConversation(conversationId: string) {
    if (downloadFeedback[conversationId] === 'loading') return
    const previousTimer = downloadFeedbackTimers.get(conversationId)
    if (previousTimer) {
      clearTimeout(previousTimer)
      downloadFeedbackTimers.delete(conversationId)
    }
    downloadFeedback = { ...downloadFeedback, [conversationId]: 'loading' }
    try {
      await downloadRagConversationPdf(conversationId)
      downloadFeedback = { ...downloadFeedback, [conversationId]: 'success' }
    } catch {
      downloadFeedback = { ...downloadFeedback, [conversationId]: 'error' }
    }
    const timer = setTimeout(() => {
      const next = { ...downloadFeedback }
      delete next[conversationId]
      downloadFeedback = next
      downloadFeedbackTimers.delete(conversationId)
    }, 1800)
    downloadFeedbackTimers.set(conversationId, timer)
  }

  onDestroy(() => {
    if (copyFeedbackTimer) clearTimeout(copyFeedbackTimer)
    if (conversationSearchTimer) clearTimeout(conversationSearchTimer)
    for (const timer of downloadFeedbackTimers.values()) clearTimeout(timer)
    downloadFeedbackTimers.clear()
  })

  // Tracking previo para el autoscroll: lets planas (no $state) porque solo
  // comparan entre ejecuciones del efecto, no disparan reactividad.
  let lastMessageCount = -1
  let lastLoading: boolean | null = null

  $effect(() => {
    const container = messagesEl
    const messageCount = $ragChat.messages.length
    const loading = $ragChat.loading
    // El store emite en cada tecleo del borrador: solo autoscrolleamos
    // cuando los mensajes o el loading cambiaron de verdad.
    const changed = messageCount !== lastMessageCount || loading !== lastLoading
    lastMessageCount = messageCount
    lastLoading = loading
    if (changed && container) {
      container.scrollTop = container.scrollHeight
    }
  })
</script>

<div class="rag-chat page-shell">
  <section class="page-header rag-chat__header" aria-labelledby="rag-chat-title">
    <div class="page-header__content">
      <h1 id="rag-chat-title">{$currentLocale && t('ragChat.title')}</h1>
      <p>{$currentLocale && t('ragChat.subtitle')}</p>
    </div>
    <div class="page-toolbar">
      <Button
        variant="ghost"
        iconOnly
        aria-label={$currentLocale && t('ragChat.clear')}
        title={$currentLocale && t('ragChat.clear')}
        onclick={() => ragChat.startNew()}
      >
        <ActionIcon name="message-circle-plus" size={18} />
      </Button>
    </div>
  </section>

  <div class="rag-chat__body">
    <div class="rag-chat__main">
      <div
        class="rag-chat__messages"
        bind:this={messagesEl}
        role="log"
        aria-live="polite"
        aria-label={$currentLocale && t('ragChat.title')}
      >
        {#if $ragChat.messages.length === 0 && !$ragChat.loading}
          <div class="rag-chat__message-row rag-chat__message-row--assistant">
            <p class="surface-message surface-message--center rag-chat__state-message rag-chat__empty">
              {$currentLocale && t('ragChat.emptyState')}
            </p>
          </div>
        {/if}

        {#each $ragChat.messages as message, index (index)}
          <div
            class="rag-chat__message-row"
            class:rag-chat__message-row--user={message.role === 'user'}
            class:rag-chat__message-row--assistant={message.role === 'assistant'}
          >
            <article
              class="rag-chat__bubble"
              class:rag-chat__bubble--user={message.role === 'user'}
              class:rag-chat__bubble--assistant={message.role === 'assistant'}
            >
              {#if message.role === 'assistant'}
                <div class="rag-chat__content rag-chat__markdown">
                  <!-- markdown: renderMarkdown escapes all HTML before emitting tags -->
                  <!-- eslint-disable-next-line svelte/no-at-html-tags -->
                  {@html renderMarkdown($currentLocale && messageContent(message))}
                </div>
              {:else}
                <p class="rag-chat__content">{$currentLocale && messageContent(message)}</p>
              {/if}

              {#if message.sources && message.sources.length > 0}
                <section
                  class="rag-chat__sources"
                  aria-label={$currentLocale && t('ragChat.sources')}
                >
                  <h2 class="rag-chat__sources-title">{$currentLocale && t('ragChat.sources')}</h2>
                  <ul class="rag-chat__sources-list">
                    {#each message.sources as source (`${source.index}-${source.assetId}`)}
                      {@const timestamp = sourceTimestamp(source)}
                      <li>
                        <button
                          type="button"
                          class="rag-chat__source"
                          onclick={() => openSource(source)}
                          aria-label={$currentLocale &&
                            `${t('ragChat.openSource')}: [${source.index}] ${source.itemTitle}`}
                          title={$currentLocale && t('ragChat.openSource')}
                        >
                          <span class="rag-chat__source-heading">
                            <span class="rag-chat__source-ref">[{source.index}]</span>
                            <span class="rag-chat__source-name"
                              >{source.itemTitle} ({source.collectionName})</span
                            >
                            {#if timestamp}
                              <span class="rag-chat__source-time">{timestamp}</span>
                            {/if}
                          </span>
                          <span class="rag-chat__source-snippet">{source.snippet}</span>
                        </button>
                      </li>
                    {/each}
                  </ul>
                </section>
              {/if}
              {#if message.role === 'assistant'}
                <div class="rag-chat__message-actions">
                  <IconButton
                    size="sm"
                    label={$currentLocale && t('ragChat.copyResponse')}
                    title={$currentLocale && t('ragChat.copyResponse')}
                    onclick={() => void copyResponse(message, index)}
                  >
                    <ActionIcon name="copy" size={14} />
                  </IconButton>
                  {#if copyFeedback?.messageIndex === index}
                    <span class="rag-chat__action-feedback" role="status">
                      {copyFeedback.tone === 'success'
                        ? ($currentLocale && t('ragChat.copiedResponse'))
                        : ($currentLocale && t('ragChat.copyResponseError'))}
                    </span>
                  {/if}
                </div>
              {/if}
            </article>
          </div>
        {/each}

        {#if $ragChat.loading}
          <div class="rag-chat__message-row rag-chat__message-row--assistant">
            <p class="rag-chat__state-message rag-chat__thinking" role="status">
              {$currentLocale && t('ragChat.thinking')}
            </p>
          </div>
        {/if}
      </div>

      {#if $ragChat.error}
        <div class="rag-chat__message-row rag-chat__message-row--assistant">
          <p class="surface-message surface-message--error rag-chat__state-message" role="alert">
            {$ragChat.error}
          </p>
        </div>
      {/if}

      <form
        class="rag-chat__composer"
        onsubmit={(event) => {
          event.preventDefault()
          handleSend()
        }}
      >
        <textarea
          class="rag-chat__input"
          rows="2"
          maxlength="4000"
          value={$ragChat.draft}
          oninput={(event) => ragChat.setDraft(event.currentTarget.value)}
          placeholder={$currentLocale && t('ragChat.placeholder')}
          aria-label={$currentLocale && t('ragChat.placeholder')}
          onkeydown={handleComposerKeydown}
          disabled={$ragChat.loading}
        ></textarea>
        <Button
          variant="primary"
          iconOnly
          type="submit"
          aria-label={$currentLocale && t('ragChat.send')}
          title={$currentLocale && t('ragChat.send')}
          disabled={!canSend}
        >
          <ActionIcon name="send" size={18} />
        </Button>
      </form>
    </div>

    <Panel variant="default" padding="none" class="rag-chat__sidebar">
      <header class="rag-chat__sidebar-header">
        <div class="rag-chat__sidebar-heading">
          <h2 class="rag-chat__sidebar-title">{$currentLocale && t('ragChat.conversations')}</h2>
          <IconButton
            size="sm"
            active={conversationSearchOpen}
            label={$currentLocale && t('ragChat.searchConversations')}
            title={$currentLocale && t('ragChat.searchConversations')}
            onclick={toggleConversationSearch}
          >
            <ActionIcon name="search" size={16} />
          </IconButton>
        </div>
        {#if conversationSearchOpen}
          <div class="rag-chat__conversation-search-wrap">
            <input
              bind:this={conversationSearchInput}
              class="rag-chat__conversation-search"
              type="search"
              value={conversationQuery}
              placeholder={$currentLocale && t('ragChat.searchConversationsPlaceholder')}
              aria-label={$currentLocale && t('ragChat.searchConversations')}
              oninput={(event) => scheduleConversationSearch(event.currentTarget.value)}
              onkeydown={(event) => {
                if (event.key === 'Escape') {
                  event.preventDefault()
                  closeConversationSearch()
                }
              }}
            />
            {#if conversationQuery}
              <SearchClearButton
                class="search-clear-button--overlay"
                label={$currentLocale && t('ragChat.searchClear')}
                onclick={clearConversationSearch}
              />
            {/if}
          </div>
        {/if}
      </header>

      {#if conversationSearchError}
        <p class="rag-chat__sidebar-empty" role="alert">{conversationSearchError}</p>
      {/if}
      {#if conversationSearchLoading}
        <p class="rag-chat__sidebar-empty" role="status">
          {$currentLocale && t('ragChat.searchingConversations')}
        </p>
      {:else if visibleConversations.length === 0 && conversationQuery.trim()}
        <p class="rag-chat__sidebar-empty">{$currentLocale && t('ragChat.noMatchingConversations')}</p>
      {:else if visibleConversations.length === 0}
        <p class="rag-chat__sidebar-empty">{$currentLocale && t('ragChat.noConversations')}</p>
      {:else}
        <ul class="rag-chat__conversations">
          {#each visibleConversations as conversation (conversation.id)}
            <li
              class="rag-chat__conversation"
              class:rag-chat__conversation--active={conversation.id ===
                $ragChat.activeConversationId}
            >
              <button
                type="button"
                class="rag-chat__conversation-button"
                aria-current={conversation.id === $ragChat.activeConversationId
                  ? 'true'
                  : undefined}
                onclick={() => void ragChat.select(conversation.id)}
              >
                <span class="rag-chat__conversation-title">{conversation.title}</span>
                <span class="rag-chat__conversation-date">
                  {formatConversationDate(conversation.updatedAt, $currentLocale)}
                </span>
              </button>
              <div class="rag-chat__conversation-actions">
                <IconButton
                  size="sm"
                  label={$currentLocale && t('ragChat.downloadConversation')}
                  title={$currentLocale && t('ragChat.downloadConversation')}
                  disabled={downloadFeedback[conversation.id] === 'loading'}
                  aria-busy={downloadFeedback[conversation.id] === 'loading' ? 'true' : undefined}
                  onclick={() => void downloadConversation(conversation.id)}
                >
                  <ActionIcon name="download" size={14} />
                </IconButton>
                <IconButton
                  size="sm"
                  class="rag-chat__conversation-delete"
                  label={$currentLocale && t('ragChat.deleteConversation')}
                  title={$currentLocale && t('ragChat.deleteConversation')}
                  onclick={() => {
                    pendingDeleteId = conversation.id
                  }}
                >
                  <ActionIcon name="delete" size={14} />
                </IconButton>
                {#if downloadFeedback[conversation.id] && downloadFeedback[conversation.id] !== 'loading'}
                  <span class="rag-chat__action-feedback" role="status">
                    {downloadFeedback[conversation.id] === 'success'
                      ? ($currentLocale && t('ragChat.downloadedConversation'))
                      : ($currentLocale && t('ragChat.downloadConversationError'))}
                  </span>
                {/if}
              </div>
            </li>
          {/each}
        </ul>
      {/if}
    </Panel>
  </div>

  {#if pendingDeleteId}
    <ConfirmDialog
      title={$currentLocale && t('ragChat.deleteConversationTitle')}
      titleId="rag-chat-delete-conversation-title"
      message={$currentLocale && t('ragChat.deleteConversationMessage')}
      cancelLabel={$currentLocale && t('collections.cancel')}
      confirmLabel={$currentLocale && t('ragChat.confirmDelete')}
      variant="destructive"
      oncancel={() => {
        pendingDeleteId = null
      }}
      onconfirm={handleDeleteConfirm}
    />
  {/if}
</div>

<style>
  .rag-chat {
    height: 100%;
    min-height: 0;
    padding-block-end: var(--space-4);
  }

  .rag-chat__header {
    flex-shrink: 0;
  }

  .rag-chat__body {
    display: flex;
    gap: var(--space-3);
    flex: 1;
    min-height: 0;
  }

  .rag-chat__main {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
    container-type: inline-size;
  }

  .rag-chat :global(.rag-chat__sidebar) {
    display: flex;
    flex-direction: column;
    width: 280px;
    flex-shrink: 0;
    min-height: 0;
    overflow: hidden;
  }

  .rag-chat__sidebar-header {
    flex-shrink: 0;
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--border-subtle);
  }
  .rag-chat__sidebar-heading {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .rag-chat__conversation-search-wrap {
    position: relative;
    margin-top: var(--space-2);
  }

  .rag-chat__conversation-search {
    width: 100%;
    box-sizing: border-box;
    padding: var(--space-2) calc(var(--space-3) + 24px + var(--space-2)) var(--space-2)
      var(--space-3);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-input);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font: inherit;
  }

  .rag-chat__conversation-search:focus {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
  }

  .rag-chat__conversation-search::-webkit-search-cancel-button {
    display: none;
  }


  .rag-chat__sidebar-title {
    margin: 0;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.075em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }

  .rag-chat__sidebar-empty {
    margin: 0;
    padding: var(--space-3);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .rag-chat__conversations {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    margin: 0;
    padding: var(--space-2);
    list-style: none;
  }

  .rag-chat__conversation {
    display: flex;
    align-items: flex-start;
    gap: var(--space-1);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    transition:
      background-color var(--transition-base),
      border-color var(--transition-base);
  }

  .rag-chat__conversation:hover {
    background: var(--surface-toolbar);
    border-color: var(--border-subtle);
  }

  .rag-chat__conversation--active {
    background: color-mix(in srgb, var(--color-accent) 14%, var(--color-surface-glass));
    border-color: color-mix(in srgb, var(--color-accent) 24%, var(--border-subtle));
  }

  .rag-chat__conversation-button {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    flex: 1;
    min-width: 0;
    padding: var(--space-2);
    border: none;
    background: none;
    cursor: pointer;
    text-align: left;
    font-family: var(--font-sans);
  }

  .rag-chat__conversation-button:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
    border-radius: var(--radius-sm);
  }

  .rag-chat__conversation-title {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    overflow-wrap: anywhere;
  }

  .rag-chat__conversation-date {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    font-variant-numeric: tabular-nums;
  }

  .rag-chat__message-actions,
  .rag-chat__conversation-actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }

  .rag-chat__message-actions {
    align-self: flex-end;
  }

  .rag-chat__conversation-actions {
    flex-shrink: 0;
    flex-wrap: wrap;
    margin: var(--space-2) var(--space-2) 0 0;
  }

  .rag-chat__conversation-actions :global(.rag-chat__conversation-delete) {
    margin: 0;
  }

  .rag-chat__action-feedback {
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .rag-chat__messages {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-2) var(--space-1);
  }

  .rag-chat__empty {
    margin: 0;
  }

  .rag-chat__message-row {
    display: flex;
    width: 100%;
    min-width: 0;
  }

  .rag-chat__message-row--user {
    justify-content: flex-end;
  }

  .rag-chat__message-row--assistant {
    justify-content: flex-start;
  }

  .rag-chat__bubble {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    width: 75%;
    min-width: 0;
    box-sizing: border-box;
    padding: var(--space-3) var(--space-4);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-dialog);
    overflow-wrap: anywhere;
  }

  .rag-chat__bubble--user {
    background: color-mix(in srgb, var(--color-accent) 14%, var(--color-surface-glass));
    border-color: color-mix(in srgb, var(--color-accent) 24%, var(--border-subtle));
  }

  .rag-chat__bubble--assistant {
    background: var(--surface-panel);
  }

  .rag-chat__state-message {
    width: 75%;
    min-width: 0;
    box-sizing: border-box;
    overflow-wrap: anywhere;
  }

  @container (max-width: 640px) {
    .rag-chat__bubble,
    .rag-chat__state-message {
      width: 100%;
    }
  }

  .rag-chat__content {
    margin: 0;
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    line-height: var(--line-height-base, 1.5);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }

  .rag-chat__markdown {
    white-space: normal;
  }

  .rag-chat__markdown :global(p) {
    margin: 0;
  }

  .rag-chat__markdown :global(p + p),
  .rag-chat__markdown :global(p + ul),
  .rag-chat__markdown :global(p + ol),
  .rag-chat__markdown :global(ul + p),
  .rag-chat__markdown :global(ol + p) {
    margin-top: var(--space-2);
  }

  .rag-chat__markdown :global(ul),
  .rag-chat__markdown :global(ol) {
    margin: 0;
    padding-left: var(--space-4);
  }

  .rag-chat__markdown :global(li) {
    margin-block: var(--space-1);
  }

  .rag-chat__markdown :global(li::marker) {
    color: var(--color-text-muted);
  }

  .rag-chat__markdown :global(strong) {
    font-weight: var(--font-weight-semibold);
  }

  .rag-chat__markdown :global(em) {
    font-style: italic;
  }

  .rag-chat__markdown :global(code) {
    padding: 0.1em 0.35em;
    border-radius: var(--radius-sm);
    background: var(--surface-toolbar);
    font-family: var(--font-mono);
    font-size: 0.9em;
  }

  .rag-chat__markdown :global(a) {
    color: var(--color-accent);
    text-decoration: underline;
    text-underline-offset: 0.15em;
  }

  .rag-chat__markdown :global(a:hover) {
    text-decoration-thickness: 2px;
  }

  .rag-chat__markdown :global(h1),
  .rag-chat__markdown :global(h2),
  .rag-chat__markdown :global(h3) {
    margin: var(--space-2) 0 var(--space-1);
    font-weight: var(--font-weight-semibold);
    line-height: 1.3;
  }

  .rag-chat__markdown :global(h1) {
    font-size: var(--font-size-md);
  }

  .rag-chat__markdown :global(h2) {
    font-size: var(--font-size-sm);
  }

  .rag-chat__markdown :global(h3) {
    font-size: var(--font-size-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }

  .rag-chat__sources {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-top: var(--space-2);
    border-top: 1px solid var(--border-subtle);
  }

  .rag-chat__sources-title {
    margin: 0;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.075em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }

  .rag-chat__sources-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .rag-chat__source {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    width: 100%;
    min-width: 0;
    box-sizing: border-box;
    padding: var(--space-2) var(--space-3);
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: none;
    cursor: pointer;
    text-align: left;
    font-family: var(--font-sans);
    transition:
      background-color var(--transition-base),
      border-color var(--transition-base);
  }

  .rag-chat__source:hover {
    background: var(--surface-toolbar);
    border-color: var(--border-subtle);
  }

  .rag-chat__source:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .rag-chat__source-heading {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-2);
  }

  .rag-chat__source-ref {
    color: var(--color-accent);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-semibold);
  }

  .rag-chat__source-name {
    min-width: 0;
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
    overflow-wrap: anywhere;
  }

  .rag-chat__source-time {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    font-variant-numeric: tabular-nums;
  }

  .rag-chat__source-snippet {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    overflow-wrap: anywhere;
  }

  .rag-chat__thinking {
    align-self: flex-start;
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--surface-toolbar);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .rag-chat__composer {
    display: flex;
    align-items: flex-end;
    gap: var(--space-2);
    flex-shrink: 0;
    padding-top: var(--space-2);
    border-top: 1px solid var(--border-subtle);
  }

  .rag-chat__input {
    flex: 1;
    min-height: var(--control-height-lg);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-input);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font-family: var(--font-sans);
    font-size: var(--font-size-sm);
    resize: vertical;
    transition:
      border-color var(--transition-smooth),
      box-shadow var(--transition-smooth),
      background-color var(--transition-smooth);
  }

  .rag-chat__input:focus {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
    background: var(--surface-panel);
  }

  .rag-chat__input:disabled {
    cursor: not-allowed;
    opacity: 0.48;
  }

  @media (max-width: 720px) {
    .rag-chat__body {
      flex-direction: column;
    }

    .rag-chat :global(.rag-chat__sidebar) {
      width: 100%;
      max-height: 12rem;
    }

    .rag-chat__composer {
      flex-direction: column;
      align-items: stretch;
    }
  }
</style>
