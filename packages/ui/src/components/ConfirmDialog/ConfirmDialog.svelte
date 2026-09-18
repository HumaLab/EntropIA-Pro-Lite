<script lang="ts">
  import { tooltip } from '../Tooltip/tooltip'
  import { ActionIcon, Button } from '../Button'
  import type { ConfirmDialogProps } from './ConfirmDialog.types'

  const FOCUSABLE_SELECTOR =
    'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

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
    confirming = false,
    confirmDisabled = false,
    cancelDisabled = false,
    confirmFirst = false,
    confirmTitle,
    oncancel,
    onconfirm,
    children,
    errorContent,
  }: ConfirmDialogProps = $props()

  const isDestructive = $derived(variant === 'destructive')
  const isConfirmDisabled = $derived(confirmDisabled || confirming)

  let dialogEl: HTMLDivElement | undefined = $state()

  function getFocusableElements(): HTMLElement[] {
    if (!dialogEl) return []

    return Array.from(dialogEl.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
      (element) => !element.hasAttribute('disabled') && !element.hasAttribute('hidden')
    )
  }

  function handleOverlayClick() {
    oncancel()
  }

  function handleWindowKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      oncancel()
      return
    }

    if (event.key !== 'Tab') return

    const focusableElements = getFocusableElements()
    if (focusableElements.length === 0) {
      event.preventDefault()
      dialogEl?.focus()
      return
    }

    const currentElement =
      document.activeElement instanceof HTMLElement ? document.activeElement : null
    const currentIndex = currentElement ? focusableElements.indexOf(currentElement) : -1
    let nextElement: HTMLElement | undefined
    if (event.shiftKey) {
      nextElement =
        currentIndex <= 0
          ? focusableElements[focusableElements.length - 1]
          : focusableElements[currentIndex - 1]
    } else {
      nextElement =
        currentIndex === -1 || currentIndex === focusableElements.length - 1
          ? focusableElements[0]
          : focusableElements[currentIndex + 1]
    }

    const target = nextElement ?? dialogEl
    event.preventDefault()
    target?.focus()
  }

  $effect(() => {
    const previouslyFocused =
      document.activeElement instanceof HTMLElement ? document.activeElement : null
    const cancelButton = dialogEl?.querySelector<HTMLElement>(
      '.confirm-dialog__actions .btn--secondary:not([disabled])'
    )
    const initialFocusTarget = cancelButton ?? getFocusableElements()[0] ?? dialogEl
    initialFocusTarget?.focus()
    window.addEventListener('keydown', handleWindowKeydown, true)

    return () => {
      window.removeEventListener('keydown', handleWindowKeydown, true)
      if (previouslyFocused?.isConnected) {
        previouslyFocused.focus()
      }
    }
  })
</script>

{#snippet cancelAction()}
  <Button variant="secondary" onclick={oncancel} disabled={cancelDisabled}>
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
  {:else}
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

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="confirm-dialog__overlay" onclick={handleOverlayClick} role="presentation">
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    bind:this={dialogEl}
    class="confirm-dialog"
    class:confirm-dialog--destructive={isDestructive}
    role="dialog"
    aria-modal="true"
    aria-labelledby={titleId}
    tabindex="-1"
    onclick={(event) => event.stopPropagation()}
  >
    <div class="confirm-dialog__content">
      <h3 id={titleId} class="confirm-dialog__title">{title}</h3>

      {#if message}
        <p class="confirm-dialog__message">{message}</p>
      {/if}

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
    </div>

    <div class="confirm-dialog__actions">
      {#if confirmFirst}
        {@render confirmAction()}
        {@render cancelAction()}
      {:else}
        {@render cancelAction()}
        {@render confirmAction()}
      {/if}
    </div>
  </div>
</div>

<style>
  .confirm-dialog__overlay {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-4);
    background-color: var(--color-overlay);
    z-index: 1000;
  }

  .confirm-dialog {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: min(100%, 440px);
    padding: var(--space-6);
    background: var(--color-surface-glass);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-dialog);
    box-shadow: var(--shadow-lg);
    font-family: var(--font-ui);
    color: var(--color-text-primary);
  }

  .confirm-dialog:focus-visible {
    outline: none;
    box-shadow: var(--shadow-lg), var(--focus-ring);
  }

  .confirm-dialog__content {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .confirm-dialog__title {
    margin: 0;
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-bold);
    color: var(--color-text-primary);
  }

  .confirm-dialog__message,
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

  .confirm-dialog__actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    justify-content: flex-end;
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
    opacity: 0.48;
    cursor: not-allowed;
  }

  @media (max-width: 720px) {
    .confirm-dialog__actions {
      flex-direction: column-reverse;
    }

    .confirm-dialog__actions :global(.btn),
    .confirm-dialog__confirm-icon {
      width: 100%;
    }
  }
</style>
