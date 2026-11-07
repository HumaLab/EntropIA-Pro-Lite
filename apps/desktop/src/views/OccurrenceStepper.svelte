<script lang="ts">
  import type { I18nKey, I18nParams } from '$lib/i18n'
  import type { PartEvidence } from '$lib/relation-occurrences'

  type Role = 'subject' | 'predicate' | 'object'

  let {
    index,
    total,
    kind,
    status,
    evidence = null,
    translate,
    onchange,
  }: {
    index: number
    total: number
    kind: 'entity' | 'relation'
    status: 'exact' | 'approximate' | 'none' | 'loading'
    /** How the current passage holds each part of the relation. */
    evidence?: Record<Role, PartEvidence> | null
    translate: (key: I18nKey, params?: I18nParams) => string
    onchange: (index: number) => void
  } = $props()

  const ROLES: Role[] = ['subject', 'predicate', 'object']

  /** The parts in one evidence state, as lower-case role names. */
  function partsIn(state: PartEvidence): string {
    return ROLES.filter((role) => evidence?.[role] === state)
      .map((role) => translate(`occurrence.${role}`).toLocaleLowerCase())
      .join(', ')
  }

  const evidenceNote = $derived.by(() => {
    if (kind !== 'relation' || status !== 'approximate' || !evidence) return null
    const lines = [translate('occurrence.reconstructed')]
    const literal = partsIn('literal')
    const partial = partsIn('partial')
    const missing = partsIn('missing')
    if (literal) lines.push(translate('occurrence.literalParts', { parts: literal }))
    if (partial) lines.push(translate('occurrence.inferredParts', { parts: partial }))
    if (missing) lines.push(translate('occurrence.missingParts', { parts: missing }))
    return lines.join(' ')
  })
</script>

<div class="occurrence-stepper">
  <div class="controls">
    <button
      type="button"
      disabled={index <= 0 || status === 'loading'}
      onclick={() => onchange(index - 1)}>{translate('occurrence.previous')}</button
    >
    <span aria-live="polite"
      >{translate('occurrence.count', { index: total ? index + 1 : 0, total })}</span
    >
    <button
      type="button"
      disabled={index >= total - 1 || status === 'loading'}
      onclick={() => onchange(index + 1)}>{translate('occurrence.next')}</button
    >
  </div>
  {#if kind === 'relation'}
    <div class="legend">
      <span class="subject">{translate('occurrence.subject')}</span>
      <span class="predicate">{translate('occurrence.predicate')}</span>
      <span class="object">{translate('occurrence.object')}</span>
      <span class="reconstructed">{translate('occurrence.legendReconstructed')}</span>
    </div>
  {/if}
  {#if evidenceNote}
    <p role="status">{evidenceNote}</p>
  {:else if status !== 'exact'}
    <p role="status">
      {translate(
        status === 'loading'
          ? 'occurrence.loading'
          : status === 'approximate'
            ? 'occurrence.approximate'
            : 'occurrence.none'
      )}
    </p>
  {/if}
</div>

<style>
  .occurrence-stepper {
    padding: var(--space-2);
    font-size: var(--font-size-sm);
    border-bottom: 1px solid var(--border-subtle);
  }
  .controls,
  .legend {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
  }
  button {
    font: inherit;
    color: inherit;
    background: var(--surface-input);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-xs);
    padding: var(--space-1) var(--space-2);
    cursor: pointer;
  }
  button:disabled {
    opacity: 0.5;
    cursor: default;
  }
  button:focus-visible {
    outline: 2px solid var(--color-accent);
  }
  .legend {
    margin-top: var(--space-2);
  }
  .subject {
    border-bottom: 3px solid var(--color-accent);
  }
  .predicate {
    border-bottom: 3px solid var(--color-warning);
  }
  .object {
    border-bottom: 3px solid var(--color-success);
  }
  .reconstructed {
    border-bottom: 3px dotted var(--color-text-secondary);
  }
  p {
    margin: var(--space-2) 0 0;
  }
</style>
