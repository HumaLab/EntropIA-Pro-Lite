<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import { Button, Panel } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import { writingAgent, isResolution, type SuggestionRow } from '$lib/writing-agent'

  /**
   * The Agente tab of the research panel (plan-editor.md §6.3, §14).
   *
   * # What this panel promises
   *
   * §14.2 opens with the rule it exists to keep: the agent never silently
   * changes the text. Every proposal is shown with what it would replace, what
   * it rests on and which model wrote it, and nothing happens to the manuscript
   * until one of the three actions is pressed.
   *
   * # Why unavailable actions are still listed
   *
   * Unit 7 asks for the capability matrix to be published *before* actions are
   * offered, and for what cannot run to be shown as such. Hiding it would leave
   * a writer wondering where a feature went; offering it and failing at the
   * moment of use is worse still, because by then they have chosen a passage
   * and formed an intention. And none of it blocks writing: the manuscript is
   * written by a person and the agent is an accessory to that.
   */

  interface Props {
    documentId?: string | null
    /** Whether a chat model is configured. A question for the settings. */
    hasChat?: boolean
    /** Whether corpus retrieval works here. A question about the agent crate. */
    hasRetrieval?: boolean
    /** The passage the agent would be asked about. */
    selection?: () => string
    /** Applies a proposal's text, once the backend has said it may be applied. */
    onapply?: (suggestion: SuggestionRow, text: string, below: boolean) => void
  }

  let {
    documentId = null,
    hasChat = false,
    hasRetrieval = false,
    selection,
    onapply,
  }: Props = $props()

  const store = writingAgent
  let snapshot = $state(store.snapshot)
  const unsubscribe = store.subscribe((value) => {
    snapshot = value
  })

  /** A failure that belongs to one suggestion, shown beside it. */
  let trouble = $state<{ id: string; code: string } | null>(null)

  onMount(() => {
    void store.loadActions(hasChat, hasRetrieval)
    if (documentId) void store.loadPending(documentId)
  })

  onDestroy(unsubscribe)

  /** Assembles what would be sent, so the writer can read it before it goes. */
  function prepare() {
    const passage = selection?.() ?? ''
    store.prepare(
      passage
        ? [{ kind: 'selection', label: t('writing.agentWillSend'), text: passage }]
        : []
    )
  }

  function evidenceOfRow(row: SuggestionRow): string {
    try {
      const evidence = JSON.parse(row.evidence_json) as Record<string, unknown[]>
      const counts = Object.entries(evidence)
        .filter(([, value]) => Array.isArray(value) && value.length > 0)
        .map(([key, value]) => `${key}: ${(value as unknown[]).length}`)
      return counts.join(' · ')
    } catch {
      return ''
    }
  }

  async function resolve(
    row: SuggestionRow,
    status: 'accepted' | 'inserted_below' | 'discarded'
  ) {
    trouble = null
    // The hash of the target as it stands now. The backend compares it before
    // letting anything be written, so a proposal made about words that have
    // since changed is reviewed again rather than applied to what is there.
    const current = row.selected_content_hash
    const out = await store.resolve(row.id, status, status === 'discarded' ? null : current)

    if (!isResolution(out)) {
      trouble = { id: row.id, code: out.code }
      return
    }
    // Only the resolution that actually changed the status carries text, which
    // is what keeps a second click from inserting the paragraph twice.
    if (out.apply && out.suggested_text) {
      onapply?.(row, out.suggested_text, status === 'inserted_below')
    } else if (!out.apply) {
      trouble = { id: row.id, code: 'suggestion_already_resolved' }
    }
  }
</script>

<div class="agent">
  {#if !hasChat}
    <p class="agent__notice" role="status">{t('writing.agentNoModel')}</p>
  {:else if !hasRetrieval}
    <p class="agent__notice" role="status">{t('writing.agentNoRetrieval')}</p>
  {/if}

  {#if snapshot.error}
    <p class="agent__error" role="alert">{snapshot.error}</p>
  {/if}

  {#if snapshot.actions.length > 0}
    <p class="agent__label">{t('writing.agentActions')}</p>
    <!-- Listed whether or not they can run. What cannot is said, not hidden. -->
    <ul class="agent__actions">
      {#each snapshot.actions as action (action.id)}
        <li class="agent__action" class:agent__action--off={!action.available}>
          <span>{action.id}</span>
          {#if !action.available}
            <span class="agent__off">{t('writing.agentUnavailable')}</span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}

  <Button variant="ghost" size="sm" onclick={prepare}>{t('writing.agentWillSend')}</Button>

  {#if snapshot.context}
    {#if snapshot.context.pieces.length === 0}
      <p class="agent__notice">{t('writing.agentNoSelection')}</p>
    {:else}
      <!-- The preview is the assembled context itself, not a description of it:
           §14.4 is only satisfied while what is shown and what would be sent
           are the same object. -->
      <ul class="agent__context">
        {#each snapshot.context.pieces as piece (piece.kind + piece.label + piece.text)}
          <li class="agent__piece">
            <span class="agent__piece-kind">{piece.kind}</span>
            <span class="agent__piece-text">{piece.text}</span>
          </li>
        {/each}
      </ul>
      <p class="agent__notice">
        {t('writing.agentChars', {
          chars: String(snapshot.context.chars),
          pieces: String(snapshot.context.pieces.length),
        })}
      </p>
      {#if snapshot.context.omitted.length > 0}
        <p class="agent__notice">
          {t('writing.agentOmitted', { count: String(snapshot.context.omitted.length) })}
        </p>
      {/if}
    {/if}
    <p class="agent__notice">{t('writing.agentNotWired')}</p>
  {/if}

  <p class="agent__label">{t('writing.agentPending')}</p>
  {#if snapshot.pending.length === 0}
    <p class="agent__notice">{t('writing.agentNonePending')}</p>
  {:else}
    {#each snapshot.pending as row (row.id)}
      <Panel padding="md">
        <div class="agent__suggestion">
          {#if row.original_text}
            <p class="agent__label">{t('writing.agentOriginal')}</p>
            <blockquote class="agent__quote">{row.original_text}</blockquote>
          {/if}

          <p class="agent__label">{t('writing.agentProposed')}</p>
          <blockquote class="agent__quote agent__quote--proposed">
            {row.suggested_text}
          </blockquote>

          {#if row.rationale}
            <p class="agent__label">{t('writing.agentRationale')}</p>
            <p class="agent__notice">{row.rationale}</p>
          {/if}

          <p class="agent__label">{t('writing.agentEvidence')}</p>
          <p class="agent__notice">
            {evidenceOfRow(row) || t('writing.agentNoEvidence')}
          </p>

          {#if row.provider || row.model}
            <p class="agent__notice">
              {t('writing.agentModel', {
                provider: row.provider ?? '',
                model: row.model ?? '',
              })}
            </p>
          {/if}

          {#if trouble?.id === row.id}
            <p class="agent__warning" role="alert">
              {trouble.code === 'suggestion_target_changed'
                ? t('writing.agentTargetChanged')
                : t('writing.agentAlreadyResolved')}
            </p>
          {/if}

          <!-- The three actions §14.2 names, and no fourth that applies
               anything without being asked. -->
          <div class="agent__buttons">
            <Button
              variant="secondary"
              size="sm"
              disabled={snapshot.busy}
              onclick={() => resolve(row, 'accepted')}
            >
              {t('writing.agentReplace')}
            </Button>
            <Button
              variant="secondary"
              size="sm"
              disabled={snapshot.busy}
              onclick={() => resolve(row, 'inserted_below')}
            >
              {t('writing.agentInsertBelow')}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              disabled={snapshot.busy}
              onclick={() => resolve(row, 'discarded')}
            >
              {t('writing.agentDiscard')}
            </Button>
          </div>
        </div>
      </Panel>
    {/each}
  {/if}
</div>

<style>
  .agent {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-height: 0;
  }

  .agent__label {
    margin: 0;
    color: var(--color-text-muted);
    font-size: var(--font-size-2xs);
    text-transform: uppercase;
    letter-spacing: 0.08em;
  }

  .agent__actions,
  .agent__context {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .agent__action {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    padding: 2px var(--space-2);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .agent__action--off {
    color: var(--color-text-muted);
  }

  .agent__off {
    font-size: var(--font-size-2xs);
  }

  .agent__piece {
    display: flex;
    gap: var(--space-2);
    padding: 2px var(--space-2);
    font-size: var(--font-size-2xs);
    color: var(--color-text-secondary);
  }

  .agent__piece-kind {
    color: var(--color-text-muted);
    flex: 0 0 8ch;
  }

  .agent__piece-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .agent__suggestion {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }

  .agent__quote {
    margin: 0;
    padding-left: var(--space-3);
    border-left: 2px solid var(--border-subtle);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }

  .agent__quote--proposed {
    border-left-color: var(--color-accent);
    color: var(--color-text-primary);
  }

  .agent__buttons {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
    padding-top: var(--space-1);
  }

  .agent__notice,
  .agent__warning,
  .agent__error {
    margin: 0;
    font-size: var(--font-size-xs);
    line-height: var(--line-height-base);
  }

  .agent__notice {
    color: var(--color-text-muted);
  }

  .agent__warning {
    color: var(--color-warning);
  }

  .agent__error {
    color: var(--color-danger);
  }
</style>
