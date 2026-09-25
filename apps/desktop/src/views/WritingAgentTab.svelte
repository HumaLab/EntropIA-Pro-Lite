<script lang="ts">
  import { onDestroy, onMount } from 'svelte'
  import { tooltip, Button, Panel } from '@entropia/ui'
  import { t, type I18nKey } from '$lib/i18n'
  import { writingAgent, isResolution, type SuggestionRow } from '$lib/writing-agent'
  import { hashSourceText } from '$lib/source-selection'
  import { evidencePieces, needsEvidence, retrievePassages } from '$lib/writing-retrieval'
  import type { ContextPiece } from '$lib/agent-context'

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
    /** The revision the proposal is made against, recorded with it. */
    sourceRevision?: number
    /**
     * Whether a proposal's target is still in the manuscript, word for word.
     *
     * Asked of the editor rather than decided here: §14.2 wants the target
     * verified before anything is written, and only the document knows.
     */
    passagePresent?: (passage: string) => boolean
    /** Applies a proposal's text, once the backend has said it may be applied. */
    onapply?: (suggestion: SuggestionRow, text: string, below: boolean) => void
  }

  let {
    documentId = null,
    hasChat = false,
    hasRetrieval = false,
    selection,
    sourceRevision = 0,
    passagePresent,
    onapply,
  }: Props = $props()

  const store = writingAgent
  let snapshot = $state(store.snapshot)
  const unsubscribe = store.subscribe((value) => {
    snapshot = value
  })

  /** A failure that belongs to one suggestion, shown beside it. */
  let trouble = $state<{ id: string; code: string } | null>(null)
  /** Why the last request produced no proposal, if it produced none. */
  let asked = $state<string | null>(null)
  /** Looking for evidence, which happens before the model is asked anything. */
  let searching = $state(false)

  /**
   * The matrix is loaded whenever the answers change, not once on mount.
   *
   * Both come from a settings read the view starts asynchronously, so a panel
   * opened before it resolves would ask with `false, false` and publish a
   * matrix where nothing is available — and never ask again. The failure is
   * silent and looks exactly like a build without a credential.
   */
  $effect(() => {
    void store.loadActions(hasChat, hasRetrieval)
  })

  onMount(() => {
    if (documentId) void store.loadPending(documentId)
  })

  onDestroy(unsubscribe)

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

  /**
   * Asks the agent for one action over the current selection (§14).
   *
   * The context is assembled and sent in one step, and stays on screen
   * afterwards: what the writer sees under "Esto es lo que se va a enviar" is
   * the very object the request carried (§14.4).
   */
  async function ask(actionId: string) {
    if (!documentId) return
    trouble = null
    asked = null
    const passage = selection?.() ?? ''
    if (!passage.trim()) {
      store.prepare([])
      return
    }

    const pieces: ContextPiece[] = [
      { kind: 'selection', label: t('writing.agentWillSend'), text: passage },
    ]

    // The four evidence actions of §14.1 are the only ones that reach into the
    // corpus. The rest are about the passage itself, and retrieving for them
    // would send the writer's sources to a provider for no purpose.
    if (needsEvidence(actionId)) {
      searching = true
      try {
        pieces.push(...evidencePieces(await retrievePassages(passage)))
      } finally {
        searching = false
      }
    }

    const out = await store.ask(pieces, {
      documentId,
      actionType: actionId,
      selection: passage,
      selectionAnchorJson: null,
      sourceRevision,
      // The same hash function that will be used to check the target when
      // the proposal is resolved, so the two comparisons are of like values.
      selectedContentHash: (await hashSourceText(passage)) ?? '',
    })
    if (!('id' in out)) asked = out.code
  }

  /**
   * The hash of a proposal's target **as it stands now**.
   *
   * Sending back the hash that was stored would compare a value against itself
   * and the guard would never fire — which is precisely the bug this replaces.
   * A target that is gone, or that now occurs twice, hashes to nothing, and the
   * backend refuses to apply it.
   */
  async function currentHashOf(row: SuggestionRow): Promise<string> {
    const passage = row.original_text ?? ''
    if (!passage || !passagePresent?.(passage)) return ''
    return (await hashSourceText(passage)) ?? ''
  }

  async function resolve(row: SuggestionRow, status: 'accepted' | 'inserted_below' | 'discarded') {
    trouble = null
    const current = status === 'discarded' ? null : await currentHashOf(row)
    const out = await store.resolve(row.id, status, current)

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
  /**
   * The button shows a one-word face; the full phrasing is what a writer reads
   * on hover. An unavailable action says so there too, because a disabled
   * button receives no pointer events and cannot carry its own title.
   *
   * The long name is deliberately NOT set as an aria-label: the accessible name
   * would then be "Buscar evidencia en contra" while the visible label reads
   * "Contraevidencia", and an accessible name that does not contain its visible
   * label breaks WCAG 2.5.3. The visible text stays the accessible name.
   */
  function actionTitle(action: { id: string; available: boolean }): string {
    const full = t(`writing.agentAction.${action.id}` as I18nKey)
    return action.available ? full : `${full} · ${t('writing.agentUnavailable')}`
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
    <!-- Listed whether or not they can run. What cannot is disabled and says
         so, rather than hidden: a writer who loses a feature without being told
         goes looking for it. -->
    <ul class="agent__actions">
      {#each snapshot.actions as action (action.id)}
        <li class="agent__action" use:tooltip={actionTitle(action)}>
          <Button
            class="agent__action-btn"
            variant="secondary"
            size="sm"
            disabled={!action.available || snapshot.busy || searching || !documentId}
            onclick={() => ask(action.id)}
          >
            {t(`writing.agentActionShort.${action.id}` as I18nKey)}
          </Button>
          {#if !action.available}
            <span class="agent__off">{t('writing.agentUnavailable')}</span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}

  {#if searching}
    <!-- Named separately from the model call: looking through the corpus and
         waiting on a provider fail differently and take different amounts of
         time, and a single "working…" hides which one is happening. -->
    <p class="agent__notice" role="status">{t('writing.agentSearching')}</p>
  {:else if snapshot.busy}
    <p class="agent__notice" role="status">{t('writing.agentAsking')}</p>
  {/if}

  {#if asked}
    <p class="agent__warning" role="alert">
      {asked === 'agent_no_credential'
        ? t('writing.agentNoCredential')
        : t('writing.agentFailed', { message: snapshot.error ?? asked })}
    </p>
  {/if}

  {#if snapshot.context}
    {#if snapshot.context.pieces.length === 0}
      <p class="agent__notice">{t('writing.agentNoSelection')}</p>
    {:else}
      <!-- The same object the request carried, not a description of it: §14.3
           asks for a record of what was sent, and a record assembled
           separately is a record of what someone believed was sent.

           It reads as a record rather than as a promise because an action
           prepares and sends in one press. There was a button here to look
           first; it went because the context is exactly the passage the writer
           had just selected and could see highlighted, so it previewed
           something already on screen. The day the context grows past the
           selection — corpus evidence, notes — looking before sending becomes
           worth a control again. -->
      <p class="agent__label">{t('writing.agentSent')}</p>
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

  .agent__context {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    padding: 0;
    list-style: none;
  }

  /* Commands, not cards: a dense grid of real buttons that reflows with the
     panel it lives in.

     The 120px floor is set against the panel's OWN range rather than one
     window: RESEARCH_BOUNDS is drag-resizable 200–560 with a 280 default, and
     across that the floor steps 1 / 2 / 3 columns. A 150px floor would leave a
     single column at the default width, which is the one width most writers
     never change. At 280 the two cells come out 128px wide — enough for the
     longest label, `Buscar evidencia en contra`, over two lines.

     auto-fit rather than auto-fill: a short action list should fill the row it
     is given rather than leave phantom columns beside it. */
  .agent__actions {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(120px, 1fr));
    gap: var(--space-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  /* Grid items stretch to their row, and the button fills its item, so one
     two-line label lifts its whole row instead of leaving neighbours short. */
  .agent__action {
    display: flex;
    min-width: 0;
  }

  /* Only geometry is overridden. `secondary` already carries the hairline
     border, the raised surface, the hover lift, --focus-ring and the disabled
     opacity, so none of those are restated here. */
  .agent__actions :global(.agent__action-btn) {
    flex: 1;
    min-width: 0;
    height: auto;
    /* A card-like action whose label may wrap to two lines: it opts out of
       the Button's token-height cap (control-block-size.test.ts). */
    max-height: none;
    padding: var(--space-2);
    justify-content: flex-start;
    text-align: left;
    white-space: normal;
    line-height: var(--line-height-tight);
  }

  .agent__actions :global(.agent__action-btn .btn__label) {
    min-width: 0;
    text-align: left;
  }

  /* Kept for a screen reader. The same words reach a sighted reader as the
     cell's where they cannot break the grid's uniform rows — a
     disabled button does not receive pointer events, so the title belongs on
     the item around it. */
  .agent__off {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    clip-path: inset(50%);
    white-space: nowrap;
    border: 0;
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
    /* Larger than the chrome around it, and deliberately so. Everything else in
       this panel is a label to be glanced at; these two blocks are prose to be
       read word by word and compared against each other, which is the whole
       decision the panel exists for. At the manuscript's own 16px they were
       12px — subordinate to the point of being squinted at. */
    font-size: var(--font-size-sm);
    line-height: var(--line-height-base);
  }

  /* Inside a suggestion everything steps up one rung, so the block reads as one
     piece of writing to judge rather than as a quotation with fine print under
     it. The rationale and what it rests on are part of the decision — §14.2
     asks for them to be *shown beside* the proposal — and at 12px they were
     read as a caption and skipped.

     Scoped to the suggestion on purpose: the same classes carry the panel's own
     notices, which are chrome and stay small. */
  .agent__suggestion .agent__notice {
    font-size: var(--font-size-sm);
  }

  .agent__suggestion .agent__label {
    font-size: var(--font-size-xs);
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
