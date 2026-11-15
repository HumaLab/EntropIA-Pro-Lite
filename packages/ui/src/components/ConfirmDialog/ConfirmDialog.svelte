<script lang="ts">
  import { tooltip } from '../Tooltip/tooltip'
  import { Dialog } from '../Dialog'
  import { ActionIcon, Button } from '../Button'
  import type { ConfirmDialogProps } from './ConfirmDialog.types'

  let dialogId = `confirm-dialog-title-${Math.random().toString(36).slice(2)}`

  let {
    title,
    message,
    titleId = dialogId,
    error = null,
    cancelLabel,
    confirmLabel,
    confirmIcon,
    confirmAriaLabel,
    variant = 'default',
    size = 'compact',
    confirming = false,
    confirmDisabled = false,
    cancelDisabled = false,
    dismissOnOverlay = true,
    confirmFirst = false,
    confirmTitle,
    oncancel,
    onconfirm,
    children,
    errorContent,
  }: ConfirmDialogProps = $props()

  const isDestructive = $derived(variant === 'destructive')
  const isConfirmDisabled = $derived(confirmDisabled || confirming)
  const hasBody = $derived(Boolean(children || error || errorContent))
</script>

{#snippet cancelAction()}
  <Button
    variant="secondary"
    onclick={oncancel}
    disabled={cancelDisabled}
    data-dialog-initial-focus
  >
    {cancelLabel}
  </Button>
{/snippet}

{#snippet confirmAction()}
  {#if confirmIcon}
    <button
      type="button"
      class="confirm-dialog__confirm-icon"
      class:confirm-dialog__confirm-icon--destructive={isDestructive}
      aria-label={confirmAriaLabel}
      use:tooltip={confirmTitle ?? confirmAriaLabel}
      aria-busy={confirming}
      onclick={onconfirm}
      disabled={isConfirmDisabled}
    >
      <ActionIcon name={confirmIcon} size={16} />
    </button>
  {:else if confirmLabel !== undefined}
    <Button
      variant={isDestructive ? 'danger' : 'primary'}
      onclick={onconfirm}
      disabled={isConfirmDisabled}
      aria-label={confirmAriaLabel}
      aria-busy={confirming}
    >
      {confirmLabel}
    </Button>
  {/if}
{/snippet}

{#snippet body()}
  {#if children}
    <div class="confirm-dialog__body">
      {@render children()}
    </div>
  {/if}

  {#if error || errorContent}
    <div class="confirm-dialog__error" role="alert">
      {#if errorContent}
        {@render errorContent()}
      {:else}
        {error}
      {/if}
    </div>
  {/if}
{/snippet}

{#snippet actions()}
  {#if confirmFirst}
    {@render confirmAction()}
    {@render cancelAction()}
  {:else}
    {@render cancelAction()}
    {@render confirmAction()}
  {/if}
{/snippet}

<!--
  The dialog stays open until the caller unmounts it: the binding reports a
  dismissal (Escape, overlay) as a cancel and always reads back open.
-->
<Dialog
  bind:open={() => true, (next) => !next && oncancel()}
  {title}
  {titleId}
  description={message || undefined}
  size={size === 'wide' ? 'lg' : 'sm'}
  dismissOnBackdrop={dismissOnOverlay}
  class={[
    'confirm-dialog',
    isDestructive && 'confirm-dialog--destructive',
    size === 'wide' && 'confirm-dialog--wide',
  ]}
  overlayClass="confirm-dialog__overlay"
  children={hasBody ? body : undefined}
  footer={actions}
/>

<style>
  /* The shell (overlay, panel, layout, focus) is Dialog's; these are the body
     and the actions this dialog puts in it. */
  :global(.confirm-dialog--wide) .confirm-dialog__body {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
  }

  .confirm-dialog__body {
    margin: 0;
    font-size: var(--font-size-sm);
    line-height: 1.5;
    color: var(--color-text-secondary);
  }

  .confirm-dialog__error {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background-color: var(--color-danger-soft);
    color: var(--color-danger);
    font-size: var(--font-size-sm);
    line-height: 1.5;
  }

  .confirm-dialog__confirm-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: var(--control-height-sm);
    height: var(--control-height-sm);
    padding: 0;
    border: 1px solid var(--color-border);
    border-radius: var(--radius-control);
    background-color: var(--color-surface-raised);
    color: var(--color-text-primary);
    cursor: pointer;
    transition:
      background-color var(--transition-smooth),
      border-color var(--transition-smooth),
      color var(--transition-smooth),
      box-shadow var(--transition-smooth),
      opacity var(--transition-smooth);
    box-shadow: none;
  }

  .confirm-dialog__confirm-icon:hover:not(:disabled) {
    background-color: var(--color-surface-elevated);
    border-color: var(--color-border-strong);
  }

  .confirm-dialog__confirm-icon--destructive {
    border-color: var(--color-danger);
    background-color: var(--color-danger-soft);
    color: var(--color-danger);
  }

  .confirm-dialog__confirm-icon--destructive:hover:not(:disabled) {
    background-color: var(--color-danger-soft);
    border-color: var(--color-danger-hover);
    color: var(--color-danger-hover);
  }

  .confirm-dialog__confirm-icon:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .confirm-dialog__confirm-icon:disabled {
    opacity: var(--opacity-disabled);
    cursor: not-allowed;
  }

  @media (max-width: 720px) {
    .confirm-dialog__confirm-icon {
      width: 100%;
    }
  }
</style>
