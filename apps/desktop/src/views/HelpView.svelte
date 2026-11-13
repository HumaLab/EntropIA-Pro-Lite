<script lang="ts">
  import { onMount } from 'svelte'
  import { getNavigation, getPaneId } from '$lib/pane-context'
  import { locale, t } from '$lib/i18n'
  import { renderMarkdown } from '$lib/markdown'
  import {
    helpAsk,
    helpDiagnosticsPreview,
    helpStatus,
    isNoKeyError,
    openHelpManual,
    type HelpSource,
    type HelpTurn,
  } from '$lib/help'
  import { ActionIcon, Button, Checkbox } from '@entropia/ui'

  const navigation = getNavigation()
  const paneId = getPaneId()
  const currentLocale = locale

  type HelpMessage = { role: 'user' | 'assistant'; content: string; sources?: HelpSource[] }

  // The conversation lives in component memory only: nothing is stored (D4).
  let status = $state<'loading' | 'no-key' | 'ready'>('loading')
  let messages = $state<HelpMessage[]>([])
  let draft = $state('')
  let sending = $state(false)
  let error = $state<string | null>(null)
  // Opt-in per question, off by default (S5, D3).
  let includeDiagnostics = $state(false)
  let preview = $state<string | null>(null)
  let previewOpen = $state(false)
  let previewLoading = $state(false)

  const canSend = $derived(status === 'ready' && !sending && draft.trim().length > 0)

  onMount(() => {
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
  })

  async function send(): Promise<void> {
    const question = draft.trim()
    if (!question || sending || status !== 'ready') return
    sending = true
    error = null
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
    const history: HelpTurn[] = messages
      .slice(-6)
      .map((message) => ({ role: message.role, content: message.content }))
    messages = [...messages, { role: 'user', content: question }]
    draft = ''
    try {
      const answer = await helpAsk(question, history, diagnostics)
      messages = [
        ...messages,
        { role: 'assistant', content: answer.answer, sources: answer.sources },
      ]
    } catch (sendError) {
      if (isNoKeyError(sendError)) {
        status = 'no-key'
        return
      }
      error = t('help.sendError')
      draft = question
    } finally {
      sending = false
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

<div class="help-view page-shell">
  <section class="page-header help-view__header" aria-labelledby="help-title-{paneId}">
    <div class="page-header__content">
      <h1 id="help-title-{paneId}">{$currentLocale && t('help.title')}</h1>
      <p>{$currentLocale && t('help.subtitle')}</p>
    </div>
  </section>

  {#if status === 'loading'}
    <p class="help-view__state" role="status">{$currentLocale && t('help.statusLoading')}</p>
  {:else if status === 'no-key'}
    <section class="help-view__no-key" aria-label={$currentLocale && t('help.noKeyTitle')}>
      <p class="help-view__no-key-title">{$currentLocale && t('help.noKeyTitle')}</p>
      <p class="help-view__no-key-body">{$currentLocale && t('help.noKeyBody')}</p>
      <div class="help-view__no-key-actions">
        <Button variant="primary" onclick={() => navigation.openRootSection({ name: 'settings' })}>
          {$currentLocale && t('help.openSettings')}
        </Button>
        <Button variant="secondary" onclick={() => void openHelpManual()}>
          <ActionIcon name="external-link" size={16} />
          {$currentLocale && t('help.openManual')}
        </Button>
      </div>
    </section>
  {:else}
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
            <Button variant="secondary" size="sm" onclick={() => (draft = t('help.suggestion1'))}>
              {$currentLocale && t('help.suggestion1')}
            </Button>
            <Button variant="secondary" size="sm" onclick={() => (draft = t('help.suggestion2'))}>
              {$currentLocale && t('help.suggestion2')}
            </Button>
            <Button variant="secondary" size="sm" onclick={() => (draft = t('help.suggestion3'))}>
              {$currentLocale && t('help.suggestion3')}
            </Button>
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
            {#if message.sources && message.sources.length > 0}
              <ul class="help-view__sources" aria-label={$currentLocale && t('help.sourcesLabel')}>
                {#each message.sources as source (source.chapter + source.title)}
                  <li>
                    {$currentLocale && t('help.sourcesLabel')}: {source.chapter} › {source.title}
                  </li>
                {/each}
              </ul>
            {/if}
          </article>
        {/if}
      {/each}

      {#if sending}
        <p class="help-view__state" role="status">{$currentLocale && t('help.thinking')}</p>
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
      <div class="help-view__diagnostics">
        <Checkbox
          bind:checked={includeDiagnostics}
          disabled={sending}
          label={$currentLocale && t('help.includeDiagnostics')}
        >
          {$currentLocale && t('help.includeDiagnostics')}
        </Checkbox>
        <button
          type="button"
          class="help-view__preview-toggle"
          onclick={() => void togglePreview()}
        >
          {$currentLocale && (previewOpen ? t('help.previewHide') : t('help.previewToggle'))}
        </button>
        <p class="help-view__hint">{$currentLocale && t('help.diagnosticsHint')}</p>
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
      <div class="help-view__input-row">
        <textarea
          class="help-view__input"
          rows="2"
          bind:value={draft}
          placeholder={$currentLocale && t('help.placeholder')}
          aria-label={$currentLocale && t('help.placeholder')}
          onkeydown={handleComposerKeydown}
          disabled={sending}></textarea>
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
      </div>
    </form>
  {/if}
</div>

<style>
  .help-view {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
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

  .help-view__diagnostics {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--space-1) var(--space-3);
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

  .help-view__hint {
    flex-basis: 100%;
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .help-view__preview {
    flex-basis: 100%;
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
