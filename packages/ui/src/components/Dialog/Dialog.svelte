<script module lang="ts">
  /**
   * Open dialogs, oldest first. Every open dialog listens on `window`, so
   * only the last one here answers Escape and Tab: a nested dialog closes
   * before the one under it.
   */
  const openDialogs: object[] = []
</script>

<script lang="ts">
  import { portal } from '../Portal/portal'
  import type { DialogProps } from './Dialog.types'

  const FOCUSABLE_SELECTOR =
    'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'
  const INITIAL_FOCUS_SELECTOR = '[data-dialog-initial-focus]:not([disabled])'

  const uid = Math.random().toString(36).slice(2)
  const descriptionId = `dialog-description-${uid}`

  let {
    open = $bindable(true),
    title,
    titleId = `dialog-title-${uid}`,
    description,
    eyebrow,
    size = 'sm',
    dismissible = true,
    dismissOnBackdrop = true,
    nested = false,
    class: className,
    overlayClass,
    onclose,
    children,
    headerActions,
    footer,
  }: DialogProps = $props()

  function close() {
    open = false
    onclose?.()
  }

  function handleOverlayClick() {
    if (dismissible && dismissOnBackdrop) close()
  }

  function focusableIn(panel: HTMLElement): HTMLElement[] {
    return Array.from(panel.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
      (element) => !element.hasAttribute('disabled') && !element.hasAttribute('hidden')
    )
  }

  function trapTab(panel: HTMLElement, event: KeyboardEvent) {
    const focusable = focusableIn(panel)
    event.preventDefault()
    if (focusable.length === 0) {
      panel.focus()
      return
    }

    const current = document.activeElement instanceof HTMLElement ? document.activeElement : null
    const index = current ? focusable.indexOf(current) : -1
    const last = focusable.length - 1
    const next = event.shiftKey
      ? focusable[index <= 0 ? last : index - 1]
      : focusable[index === -1 || index === last ? 0 : index + 1]
    ;(next ?? panel).focus()
  }

  let panelEl: HTMLDivElement | null = $state(null)

  // Focus in, keyboard trapped, focus back out: for as long as the panel exists.
  $effect(() => {
    if (!open || !panelEl) return
    const panel: HTMLDivElement = panelEl

    const token = {}
    openDialogs.push(token)
    const previouslyFocused =
      document.activeElement instanceof HTMLElement ? document.activeElement : null

    const initial =
      panel.querySelector<HTMLElement>(INITIAL_FOCUS_SELECTOR) ?? focusableIn(panel)[0] ?? panel
    initial.focus()

    function handleKeydown(event: KeyboardEvent) {
      if (openDialogs[openDialogs.length - 1] !== token) return

      if (event.key === 'Escape') {
        event.preventDefault()
        event.stopPropagation()
        if (dismissible) close()
        return
      }

      if (event.key === 'Tab') trapTab(panel, event)
    }

    window.addEventListener('keydown', handleKeydown, true)

    return () => {
      window.removeEventListener('keydown', handleKeydown, true)
      openDialogs.splice(openDialogs.indexOf(token), 1)
      if (previouslyFocused?.isConnected) previouslyFocused.focus()
    }
  })
</script>

{#if open}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- Portalled out of the work pane; see Portal/portal.ts. -->
  <div
    class={['dialog__overlay', nested && 'dialog__overlay--nested', overlayClass]}
    {@attach portal}
    onclick={handleOverlayClick}
    role="presentation"
  >
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class={['dialog', `dialog--${size}`, className]}
      role="dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      aria-describedby={description ? descriptionId : undefined}
      tabindex="-1"
      onclick={(event) => event.stopPropagation()}
      bind:this={panelEl}
    >
      <div class="dialog__content">
        <div class="dialog__header">
          <div class="dialog__heading">
            {#if eyebrow}
              <span class="dialog__eyebrow">{eyebrow}</span>
            {/if}
            <h3 id={titleId} class="dialog__title">{title}</h3>

            {#if description}
              <p id={descriptionId} class="dialog__description">{description}</p>
            {/if}
          </div>

          {#if headerActions}
            <div class="dialog__header-actions">
              {@render headerActions()}
            </div>
          {/if}
        </div>

        {#if children}
          <div class="dialog__body">
            {@render children()}
          </div>
        {/if}
      </div>

      {#if footer}
        <div class="dialog__footer">
          {@render footer()}
        </div>
      {/if}
    </div>
  </div>
{/if}

<style>
  .dialog__overlay {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: var(--space-4);
    background-color: var(--surface-overlay);
    z-index: var(--z-dialog);
  }

  .dialog__overlay--nested {
    z-index: var(--z-dialog-nested);
  }

  .dialog {
    --dialog-width-sm: 440px;
    --dialog-width-md: 640px;
    --dialog-width-lg: 1120px;

    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    width: min(100%, var(--dialog-width-sm));
    padding: var(--space-6);
    background: var(--color-surface-glass);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-dialog);
    box-shadow: var(--shadow-lg);
    font-family: var(--font-ui);
    color: var(--color-text-primary);
  }

  .dialog--md {
    width: min(100%, var(--dialog-width-md));
  }

  .dialog--lg {
    width: min(100%, var(--dialog-width-lg));
    max-height: 100%;
    padding: var(--space-5);
  }

  .dialog--lg .dialog__content,
  .dialog--lg .dialog__body {
    flex: 1 1 auto;
    min-height: 0;
  }

  .dialog--lg .dialog__body {
    overflow-y: auto;
  }

  /* A document viewer: fixed height, the header on its own band and the body
     reaching the panel's edges, so the viewer and its side panel fill it. */
  .dialog--viewer {
    width: min(100%, var(--dialog-width-lg));
    height: min(820px, calc(100vh - var(--space-8)));
    min-height: 0;
    gap: 0;
    padding: 0;
    overflow: hidden;
  }

  .dialog--viewer .dialog__content {
    flex: 1 1 auto;
    min-height: 0;
    gap: 0;
  }

  .dialog--viewer .dialog__header {
    padding: var(--space-4) var(--space-5);
    border-bottom: 1px solid var(--color-hairline);
    background: color-mix(in srgb, var(--surface-panel) 88%, transparent);
  }

  .dialog--viewer .dialog__title {
    overflow: hidden;
    line-height: var(--line-height-tight);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dialog--viewer .dialog__description {
    margin-top: var(--space-1);
    overflow: hidden;
    font-size: var(--font-size-xs);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dialog--viewer .dialog__body {
    flex: 1 1 auto;
    min-height: 0;
    gap: 0;
    overflow: hidden;
  }

  .dialog:focus-visible {
    outline: none;
    box-shadow: var(--shadow-lg), var(--focus-ring);
  }

  .dialog__content,
  .dialog__body {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .dialog__header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-4);
  }

  .dialog__heading {
    flex: 1 1 auto;
    min-width: 0;
  }

  .dialog__header-actions {
    display: flex;
    flex: none;
    align-items: center;
    gap: var(--space-2);
  }

  .dialog__eyebrow {
    display: block;
    margin-bottom: var(--space-1);
    color: var(--color-accent);
    font-family: var(--font-mono);
    font-size: var(--font-size-2xs);
    font-weight: var(--font-weight-semibold);
    letter-spacing: var(--letter-spacing-caps);
    text-transform: uppercase;
  }

  .dialog__title {
    margin: 0;
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-bold);
    color: var(--color-text-primary);
  }

  .dialog__description {
    margin: var(--space-3) 0 0;
    font-size: var(--font-size-sm);
    line-height: 1.5;
    color: var(--color-text-secondary);
  }

  .dialog__footer {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-3);
    justify-content: flex-end;
  }

  @media (max-width: 760px) {
    .dialog--viewer {
      height: calc(100vh - var(--space-8));
    }

    .dialog--viewer .dialog__header {
      padding: var(--space-3);
    }
  }

  @media (max-width: 720px) {
    .dialog__footer {
      flex-direction: column-reverse;
    }

    .dialog__footer :global(.btn) {
      width: 100%;
    }
  }
</style>
