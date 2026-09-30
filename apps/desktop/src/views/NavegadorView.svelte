<script lang="ts">
  /**
   * Experimental in-app browser (VITE_NAVEGADOR=1 + Cargo feature `navegador`).
   *
   * The page is not part of this DOM: the backend draws it in a native child
   * webview laid over the window. What this view owns is an empty placeholder
   * that marks where the page goes, and the controls around it. The native
   * webview draws above every HTML element, so anything from this UI that
   * overlaps the placeholder (a dialog, a menu) would be hidden behind it: the
   * view hides the page while an overlay is open in the shell's overlay root.
   * Menus that attach to <body> instead (ToolbarMenu) are not detected.
   */
  import { onDestroy, onMount, untrack } from 'svelte'
  import { get } from 'svelte/store'
  import { ActionIcon, IconButton } from '@entropia/ui'
  import { locale, t } from '$lib/i18n'
  import { zoomFactor } from '$lib/zoom'
  import {
    computeBounds,
    navegadorBack,
    navegadorForward,
    navegadorNavigate,
    navegadorReload,
    navegadorSession,
    navegadorState,
    onNavegadorState,
    type ViewerBounds,
    type ViewerState,
  } from '$lib/navegador'

  const currentLocale = locale
  const instanceId = crypto.randomUUID()
  /** How often the placeholder is re-measured, for moves no resize reveals. */
  const REMEASURE_MS = 400

  let placeholder: HTMLDivElement | undefined = $state()
  let address = $state('')
  let addressFocused = $state(false)
  let opened = $state(false)
  let overlayOpen = $state(false)
  let status = $state<ViewerState>({ url: null, title: null, blocked: null })
  let error = $state<string | null>(null)
  let lastSent: string | null = null

  function measure(): ViewerBounds | null {
    if (!placeholder) return null
    return computeBounds(placeholder.getBoundingClientRect(), get(zoomFactor))
  }

  function describe(reason: unknown): string {
    return reason instanceof Error ? reason.message : String(reason)
  }

  /** Send the placeholder's rect if it moved since the last time. */
  async function syncBounds(force = false) {
    if (!opened || overlayOpen) return
    const bounds = measure()
    if (!bounds) return
    const key = JSON.stringify(bounds)
    if (!force && key === lastSent) return
    lastSent = key
    try {
      await navegadorSession.setBounds(instanceId, bounds)
    } catch (reason) {
      console.warn('[navegador] could not move the page area:', reason)
    }
  }

  async function submit(event: SubmitEvent) {
    event.preventDefault()
    const text = address.trim()
    if (!text) return
    error = null
    const bounds = measure()
    if (!bounds) {
      error = t('navegador.error', { message: t('navegador.pageArea') })
      return
    }
    try {
      const alreadyOpen = navegadorSession.isOpen()
      lastSent = JSON.stringify(bounds)
      await navegadorSession.show(instanceId, text, bounds)
      status = alreadyOpen ? await navegadorNavigate(text) : await navegadorState()
      opened = true
      if (status.url) address = status.url
    } catch (reason) {
      error = t('navegador.error', { message: describe(reason) })
    }
  }

  async function act(command: () => Promise<void>) {
    error = null
    try {
      await command()
    } catch (reason) {
      error = t('navegador.error', { message: describe(reason) })
    }
  }

  // Show the page while nothing covers it, hide it while an overlay is open.
  $effect(() => {
    if (!opened) return
    const visible = !overlayOpen
    untrack(() => {
      const bounds = measure()
      if (visible && bounds) {
        lastSent = JSON.stringify(bounds)
        void navegadorSession.show(instanceId, address, bounds).catch(() => undefined)
      } else if (!visible) {
        void navegadorSession.hide(instanceId).catch(() => undefined)
      }
    })
  })

  // A zoom change moves the placeholder in logical pixels without resizing it.
  $effect(() => {
    void $zoomFactor
    untrack(() => void syncBounds(true))
  })

  onMount(() => {
    navegadorSession.attach(instanceId)

    let unlisten: (() => void) | undefined
    let disposed = false
    void onNavegadorState((next) => {
      status = next
      if (!addressFocused && next.url) address = next.url
    }).then((stop) => {
      if (disposed) stop()
      else unlisten = stop
    })

    // Adopt a browser another view left open: same page, this view's address.
    void navegadorState()
      .then((state) => {
        if (state.url) {
          status = state
          address = state.url
          opened = navegadorSession.isOpen()
        }
      })
      .catch(() => undefined)

    const resize = new ResizeObserver(() => void syncBounds())
    if (placeholder) resize.observe(placeholder)
    const onWindowChange = () => void syncBounds()
    window.addEventListener('resize', onWindowChange)
    // Scrolling any ancestor moves the placeholder without resizing it.
    window.addEventListener('scroll', onWindowChange, true)
    const timer = window.setInterval(() => void syncBounds(), REMEASURE_MS)

    const overlayRoot = document.querySelector('[data-overlay-root]')
    const watchOverlay = () => {
      overlayOpen = (overlayRoot?.childElementCount ?? 0) > 0
    }
    const overlays = new MutationObserver(watchOverlay)
    if (overlayRoot) overlays.observe(overlayRoot, { childList: true })
    watchOverlay()

    return () => {
      disposed = true
      unlisten?.()
      resize.disconnect()
      overlays.disconnect()
      window.removeEventListener('resize', onWindowChange)
      window.removeEventListener('scroll', onWindowChange, true)
      window.clearInterval(timer)
    }
  })

  onDestroy(() => {
    void navegadorSession.detach(instanceId).catch((reason) => {
      console.warn('[navegador] could not close the browser:', reason)
    })
  })
</script>

<div class="navegador-view page-shell">
  <form class="navegador-view__bar" onsubmit={submit}>
    <IconButton
      size="md"
      variant="secondary"
      label={$currentLocale && t('navegador.back')}
      title={$currentLocale && t('navegador.back')}
      disabled={!opened}
      onclick={() => void act(navegadorBack)}
    >
      <ActionIcon name="chevron-left" size={16} />
    </IconButton>
    <IconButton
      size="md"
      variant="secondary"
      label={$currentLocale && t('navegador.forward')}
      title={$currentLocale && t('navegador.forward')}
      disabled={!opened}
      onclick={() => void act(navegadorForward)}
    >
      <ActionIcon name="chevron-right" size={16} />
    </IconButton>
    <IconButton
      size="md"
      variant="secondary"
      label={$currentLocale && t('navegador.reload')}
      title={$currentLocale && t('navegador.reload')}
      disabled={!opened}
      onclick={() => void act(navegadorReload)}
    >
      <ActionIcon name="refresh" size={16} />
    </IconButton>
    <input
      class="navegador-view__address"
      type="text"
      inputmode="url"
      autocomplete="off"
      autocapitalize="off"
      spellcheck="false"
      aria-label={$currentLocale && t('navegador.address')}
      placeholder={$currentLocale && t('navegador.addressPlaceholder')}
      bind:value={address}
      onfocus={() => (addressFocused = true)}
      onblur={() => (addressFocused = false)}
    />
  </form>

  <p class="navegador-view__status" role="status" aria-live="polite">
    {#if error}
      <span class="navegador-view__problem">{error}</span>
    {:else if status.blocked}
      <span class="navegador-view__problem"
        >{$currentLocale && t('navegador.blocked', { reason: status.blocked })}</span
      >
    {:else if status.url}
      {#if status.title}<strong>{status.title}</strong>{/if}
      <span>{status.url}</span>
    {:else}
      {$currentLocale && t('navegador.idle')}
    {/if}
  </p>

  <div
    class="navegador-view__page"
    role="region"
    aria-label={$currentLocale && t('navegador.pageArea')}
    bind:this={placeholder}
  ></div>
</div>

<style>
  .navegador-view {
    height: 100%;
    padding-inline: var(--space-4);
    padding-block-start: var(--space-4);
  }

  .navegador-view__bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .navegador-view__address {
    flex: 1;
    min-width: 0;
    min-height: var(--control-height-md);
    padding: 0 var(--space-3);
    box-sizing: border-box;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
    background: var(--color-surface-sunken);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
  }

  .navegador-view__address:focus {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
    background: var(--color-surface);
  }

  .navegador-view__status {
    display: flex;
    gap: var(--space-2);
    min-height: 1.5em;
    margin: 0;
    overflow: hidden;
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .navegador-view__problem {
    color: var(--color-danger, var(--color-text-primary));
  }

  /* Only marks where the native webview goes; the page is not DOM. */
  .navegador-view__page {
    flex: 1;
    min-height: 200px;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: var(--color-surface-sunken);
  }
</style>
