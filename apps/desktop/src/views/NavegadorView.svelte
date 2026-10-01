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
   *
   * The capture panel sits below the placeholder, outside the native webview's
   * rect: showing it shrinks the placeholder, and the ResizeObserver below moves
   * the webview with it. Captures and downloads are drafts; nothing is stored.
   *
   * This view comes and goes (another section, another app tab), the browser
   * does not: unmounting only hides it, and the next mount shows the same active
   * tab at its own rect. The browser's tabs, the drafts and the download list
   * live in `navegadorStore`, outside the component, for the same reason. The
   * browser closes when its app tab does (`watchNavegadorTabs`, installed by the
   * shell) or the app exits.
   *
   * The browser has its own tabs (up to four), each a native webview the backend
   * keeps; the strip above the address bar shows them. The backend owns which
   * one is active and tells this view through `navegadorStore`; every button
   * that acts on a page names the tab it meant, so a tab that became active in
   * between is never the one acted on.
   */
  import { onDestroy, onMount, tick, untrack } from 'svelte'
  import { get } from 'svelte/store'
  import { open as openFolderDialog } from '@tauri-apps/plugin-dialog'
  import { ActionIcon, Button, IconButton, tooltip } from '@entropia/ui'
  import { locale, t } from '$lib/i18n'
  import { zoomFactor } from '$lib/zoom'
  import {
    computeBounds,
    navegadorActivateTab,
    navegadorBack,
    navegadorCloseTab,
    navegadorForward,
    navegadorNavigate,
    navegadorNewTab,
    navegadorReload,
    navegadorSession,
    navegadorState,
    type BrowserState,
    type ViewerBounds,
  } from '$lib/navegador'
  import { MAX_TABS, activeTab, canOpenTab, describeTabs } from '$lib/navegador-tabs'
  import { navegadorStore } from '$lib/navegador-store'
  import {
    describeCaptureDraft,
    describeDownload,
    downloadReasonKey,
    formatBytes,
    navegadorCapturePage,
    navegadorCaptureSelection,
    navegadorDownloadDir,
    navegadorSetDownloadDir,
    parseCaptureError,
    type CaptureDraft,
    type DownloadFolder,
  } from '$lib/navegador-capture'

  const currentLocale = locale
  const instanceId = crypto.randomUUID()
  /** How often the placeholder is re-measured, for moves no resize reveals. */
  const REMEASURE_MS = 400

  let placeholder: HTMLDivElement | undefined = $state()
  let addressInput: HTMLInputElement | undefined = $state()
  let address = $state('')
  let addressFocused = $state(false)
  /** Whether the browser exists (its first tab was made); not whether a tab has a page. */
  let opened = $state(false)
  let overlayOpen = $state(false)
  let error = $state<string | null>(null)
  let lastSent: string | null = null
  /** The tab the address bar is showing, to tell a tab switch from a page moving. */
  let shownTab: number | null = null

  const browser = $derived($navegadorStore.browser)
  const current = $derived(activeTab(browser))
  /** What the active tab's page says; all empty for a blank tab. */
  const status = $derived({
    url: current?.url ?? null,
    title: current?.title ?? null,
    blocked: current?.blocked ?? null,
  })
  /** The active tab has a page, so there is something to go back in or capture. */
  const hasPage = $derived(current?.url != null)
  const newTabLabel = $derived($currentLocale ? t('navegador.tabs.new') : '')
  const tabItems = $derived(describeTabs(browser, newTabLabel))
  const canAddTab = $derived(canOpenTab(tabItems.length))

  let capturing = $state(false)
  let folder = $state<DownloadFolder | null>(null)
  let folderError = $state<string | null>(null)
  const captureError = $derived($navegadorStore.captureError)
  const captureView = $derived(
    $navegadorStore.capture ? describeCaptureDraft($navegadorStore.capture) : null
  )
  const downloadViews = $derived($navegadorStore.downloads.map((draft) => describeDownload(draft)))
  const panelOpen = $derived(
    captureView !== null || captureError !== null || downloadViews.length > 0
  )

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
      // The first time this makes the first tab and loads the address in it.
      await navegadorSession.show(instanceId, text, bounds)
      let next: BrowserState
      if (alreadyOpen) {
        const tab = current?.id ?? activeTab(await navegadorState())?.id
        if (tab === undefined) throw new Error(t('navegador.tabs.new'))
        next = await navegadorNavigate(tab, text)
      } else {
        next = await navegadorState()
      }
      navegadorStore.applyBrowser(next)
      opened = true
      const url = activeTab(next)?.url
      if (url) address = url
    } catch (reason) {
      error = t('navegador.error', { message: describe(reason) })
    }
  }

  /** Run a command on the tab on screen right now; the tab is fixed at the click. */
  async function actOnTab(command: (tab: number) => Promise<void>) {
    const tab = current?.id
    if (tab === undefined) return
    error = null
    try {
      await command(tab)
    } catch (reason) {
      error = t('navegador.error', { message: describe(reason) })
    }
  }

  async function changeTabs(command: () => Promise<BrowserState>) {
    error = null
    try {
      navegadorStore.applyBrowser(await command())
    } catch (reason) {
      error = t('navegador.error', { message: describe(reason) })
    }
  }

  async function openTab() {
    if (!canAddTab) return
    await changeTabs(navegadorNewTab)
    // A blank tab is for typing an address into.
    await tick()
    addressInput?.focus()
  }

  async function takeCapture(command: (tab: number) => Promise<CaptureDraft>) {
    const tab = current?.id
    if (capturing || tab === undefined) return
    capturing = true
    navegadorStore.clearCapture()
    try {
      navegadorStore.setCapture(await command(tab))
    } catch (reason) {
      const { code, detail } = parseCaptureError(reason)
      navegadorStore.setCaptureError(
        t(`navegador.capture.error.${code}`, { message: detail ?? '' })
      )
    } finally {
      capturing = false
    }
  }

  async function changeFolder() {
    folderError = null
    try {
      const picked = await openFolderDialog({
        directory: true,
        multiple: false,
        defaultPath: folder?.path ?? undefined,
        title: t('navegador.download.folderDialog'),
      })
      if (typeof picked !== 'string') return
      folder = await navegadorSetDownloadDir(picked)
    } catch (reason) {
      folderError = t('navegador.download.folderError', { message: describe(reason) })
    }
  }

  function dismissCapture() {
    navegadorStore.clearCapture()
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

  // The address bar shows the active tab's page. Another tab always replaces
  // what is typed; a page that moves only does while the person is not typing.
  $effect(() => {
    const id = current?.id ?? null
    const url = current?.url ?? ''
    untrack(() => {
      if (id !== shownTab) {
        shownTab = id
        address = url
      } else if (!addressFocused) {
        address = url
      }
    })
  })

  // A zoom change moves the placeholder in logical pixels without resizing it.
  $effect(() => {
    void $zoomFactor
    untrack(() => void syncBounds(true))
  })

  onMount(() => {
    let disposed = false

    void navegadorDownloadDir()
      .then((next) => {
        if (next && !disposed) folder = next
      })
      .catch(() => undefined)

    // The store keeps listening after this view is gone: it hears the tabs and
    // the downloads for the rest of the session.
    void navegadorStore.startListening().catch((reason) => {
      console.warn('[navegador] could not follow the browser:', reason)
    })

    // Adopt a browser another view left open: same tabs, this view's rect.
    void navegadorState()
      .then((state) => {
        if (state.tabs.length > 0) {
          navegadorStore.applyBrowser(state)
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
      resize.disconnect()
      overlays.disconnect()
      window.removeEventListener('resize', onWindowChange)
      window.removeEventListener('scroll', onWindowChange, true)
      window.clearInterval(timer)
    }
  })

  onDestroy(() => {
    void navegadorSession.detach(instanceId).catch((reason) => {
      console.warn('[navegador] could not hide the browser:', reason)
    })
  })
</script>

<div class="navegador-view page-shell">
  {#if tabItems.length > 0}
    <div
      class="navegador-view__tabs"
      role="group"
      aria-label={$currentLocale && t('navegador.tabs.aria')}
    >
      {#each tabItems as item (item.id)}
        <div class="navegador-view__tab" class:navegador-view__tab--active={item.active}>
          <button
            type="button"
            class="navegador-view__tab-label"
            aria-current={item.active ? 'true' : undefined}
            use:tooltip={item.url ?? item.label}
            onclick={() => void changeTabs(() => navegadorActivateTab(item.id))}
          >
            {item.label}
          </button>
          <IconButton
            size="sm"
            variant="ghost"
            label={$currentLocale && t('navegador.tabs.close', { title: item.label })}
            title={$currentLocale && t('navegador.tabs.close', { title: item.label })}
            onclick={() => void changeTabs(() => navegadorCloseTab(item.id))}
          >
            <ActionIcon name="close" size={14} />
          </IconButton>
        </div>
      {/each}
      <IconButton
        size="sm"
        variant="ghost"
        label={$currentLocale &&
          (canAddTab ? t('navegador.tabs.add') : t('navegador.tabs.full', { max: MAX_TABS }))}
        title={$currentLocale &&
          (canAddTab ? t('navegador.tabs.add') : t('navegador.tabs.full', { max: MAX_TABS }))}
        disabled={!canAddTab}
        onclick={() => void openTab()}
      >
        <ActionIcon name="add" size={14} />
      </IconButton>
    </div>
  {/if}

  <form class="navegador-view__bar" onsubmit={submit}>
    <IconButton
      size="md"
      variant="secondary"
      label={$currentLocale && t('navegador.back')}
      title={$currentLocale && t('navegador.back')}
      disabled={!hasPage}
      onclick={() => void actOnTab(navegadorBack)}
    >
      <ActionIcon name="chevron-left" size={16} />
    </IconButton>
    <IconButton
      size="md"
      variant="secondary"
      label={$currentLocale && t('navegador.forward')}
      title={$currentLocale && t('navegador.forward')}
      disabled={!hasPage}
      onclick={() => void actOnTab(navegadorForward)}
    >
      <ActionIcon name="chevron-right" size={16} />
    </IconButton>
    <IconButton
      size="md"
      variant="secondary"
      label={$currentLocale && t('navegador.reload')}
      title={$currentLocale && t('navegador.reload')}
      disabled={!hasPage}
      onclick={() => void actOnTab(navegadorReload)}
    >
      <ActionIcon name="refresh" size={16} />
    </IconButton>
    <IconButton
      size="md"
      variant="secondary"
      label={$currentLocale && t('navegador.capturePage')}
      title={$currentLocale && t('navegador.capturePage')}
      disabled={!hasPage || capturing}
      onclick={() => void takeCapture(navegadorCapturePage)}
    >
      <ActionIcon name="file-text" size={16} />
    </IconButton>
    <IconButton
      size="md"
      variant="secondary"
      label={$currentLocale && t('navegador.captureSelection')}
      title={$currentLocale && t('navegador.captureSelection')}
      disabled={!hasPage || capturing}
      onclick={() => void takeCapture(navegadorCaptureSelection)}
    >
      <ActionIcon name="text-quote" size={16} />
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
      bind:this={addressInput}
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

  {#if panelOpen}
    <section
      class="navegador-view__panel"
      aria-label={$currentLocale && t('navegador.capture.title')}
    >
      {#if captureError}
        <p class="navegador-view__problem" role="alert">{captureError}</p>
      {/if}
      {#if captureView}
        <header class="navegador-view__panel-head">
          <strong class="navegador-view__panel-title">{captureView.title}</strong>
          <span class="navegador-view__chip"
            >{$currentLocale && t(`navegador.capture.kind.${captureView.kind}`)}</span
          >
          <IconButton
            size="sm"
            variant="ghost"
            label={$currentLocale && t('navegador.capture.dismiss')}
            title={$currentLocale && t('navegador.capture.dismiss')}
            onclick={dismissCapture}
          >
            <ActionIcon name="close" size={14} />
          </IconButton>
        </header>
        <dl class="navegador-view__facts">
          <dt>{$currentLocale && t('navegador.capture.finalUrl')}</dt>
          <dd>{captureView.finalUrl}</dd>
          <dt>{$currentLocale && t('navegador.capture.accessedAt')}</dt>
          <dd>{captureView.accessedAt}</dd>
          <dt>{$currentLocale && t(`navegador.capture.hash.${captureView.hashOf}`)}</dt>
          <dd><code>{captureView.shortSha}</code></dd>
        </dl>
        <p class="navegador-view__meta">
          {$currentLocale && t('navegador.capture.text', { count: captureView.textLength })}
          {#if captureView.kind === 'page'}
            · {$currentLocale &&
              t('navegador.capture.html', { size: formatBytes(captureView.htmlBytes) })}
          {/if}
        </p>
        {#if captureView.truncated}
          <p class="navegador-view__problem">
            {$currentLocale && t('navegador.capture.truncated')}
          </p>
        {/if}
        <blockquote class="navegador-view__preview">{captureView.preview}</blockquote>
      {/if}

      {#if downloadViews.length > 0}
        <header class="navegador-view__panel-head navegador-view__downloads-head">
          <h3 class="navegador-view__panel-subtitle">
            {$currentLocale && t('navegador.download.title')}
          </h3>
          <Button size="sm" variant="ghost" onclick={() => navegadorStore.clearDownloads()}>
            {$currentLocale && t('navegador.download.clear')}
          </Button>
        </header>
        <p class="navegador-view__folder">
          <span>{$currentLocale && t('navegador.download.folder')}</span>
          {#if folder?.path}<code>{folder.path}</code>{/if}
          <Button size="sm" variant="ghost" onclick={() => void changeFolder()}>
            {$currentLocale && t('navegador.download.folderChange')}
          </Button>
        </p>
        {#if folderError}
          <p class="navegador-view__problem" role="alert">{folderError}</p>
        {/if}
        <ul class="navegador-view__downloads">
          {#each downloadViews as item (item.id)}
            <li>
              <strong class="navegador-view__download-text" use:tooltip={item.fileName}
                >{item.fileName}</strong
              >
              <span class="navegador-view__chip"
                >{$currentLocale && t(`navegador.download.status.${item.status}`)}</span
              >
              {#if item.size}<span>{item.size}</span>{/if}
              {#if item.shortSha}<code>{item.shortSha}</code>{/if}
              {#if item.host}
                {@const origin =
                  $currentLocale &&
                  (item.pageTitle
                    ? t('navegador.download.fromPage', { host: item.host, title: item.pageTitle })
                    : t('navegador.download.from', { host: item.host }))}
                <span class="navegador-view__download-text" use:tooltip={origin}>{origin}</span>
              {/if}
              {#if item.status === 'saved' && item.savedTo}
                {@const saved =
                  $currentLocale && t('navegador.download.savedIn', { folder: item.savedTo })}
                <span class="navegador-view__download-text" use:tooltip={saved}>{saved}</span>
              {/if}
              {#if item.status === 'rejected' || item.status === 'failed'}
                {@const problem = $currentLocale && t(downloadReasonKey(item.reason))}
                <span
                  class="navegador-view__problem navegador-view__download-text"
                  use:tooltip={problem}>{problem}</span
                >
              {/if}
              <span class="navegador-view__row-end">
                <IconButton
                  size="sm"
                  variant="ghost"
                  label={$currentLocale && t('navegador.download.dismiss', { name: item.fileName })}
                  title={$currentLocale && t('navegador.download.dismiss', { name: item.fileName })}
                  onclick={() => navegadorStore.dismissDownload(item.id)}
                >
                  <ActionIcon name="close" size={14} />
                </IconButton>
              </span>
            </li>
          {/each}
        </ul>
      {/if}
    </section>
  {/if}
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

  .navegador-view__tabs {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin-block-end: var(--space-2);
    min-width: 0;
  }

  .navegador-view__tab {
    display: flex;
    align-items: center;
    min-width: 0;
    max-width: 16rem;
    padding-inline-start: var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
    background: var(--color-surface-sunken);
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
  }

  .navegador-view__tab--active {
    border-color: var(--color-accent);
    background: var(--color-surface);
    color: var(--color-text-primary);
  }

  .navegador-view__tab-label {
    min-width: 0;
    padding: 0;
    overflow: hidden;
    border: 0;
    background: none;
    color: inherit;
    font: inherit;
    text-align: start;
    text-overflow: ellipsis;
    white-space: nowrap;
    cursor: pointer;
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

  .navegador-view__panel {
    flex: none;
    max-height: 30%;
    overflow-y: auto;
    padding: var(--space-3);
    margin-block-start: var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: var(--color-surface);
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  .navegador-view__panel-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .navegador-view__panel-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    color: var(--color-text-primary);
    white-space: nowrap;
    text-overflow: ellipsis;
  }

  .navegador-view__panel-subtitle {
    margin: var(--space-3) 0 var(--space-1);
    font-size: var(--font-size-xs);
    color: var(--color-text-primary);
  }

  .navegador-view__downloads-head {
    margin-block-start: var(--space-3);
  }

  .navegador-view__downloads-head .navegador-view__panel-subtitle {
    flex: 1;
    margin: 0;
  }

  .navegador-view__folder {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-1) 0 var(--space-2);
  }

  .navegador-view__folder code {
    overflow-wrap: anywhere;
    color: var(--color-text-primary);
  }

  .navegador-view__row-end {
    margin-inline-start: auto;
  }

  .navegador-view__chip {
    padding: 0 var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
    white-space: nowrap;
  }

  .navegador-view__facts {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--space-1) var(--space-3);
    margin: var(--space-2) 0;
  }

  .navegador-view__facts dd {
    min-width: 0;
    margin: 0;
    overflow-wrap: anywhere;
    color: var(--color-text-primary);
  }

  .navegador-view__meta {
    margin: 0;
  }

  .navegador-view__preview {
    margin: var(--space-2) 0 0;
    padding-inline-start: var(--space-3);
    border-inline-start: 2px solid var(--color-hairline);
    overflow-wrap: anywhere;
    color: var(--color-text-primary);
  }

  .navegador-view__downloads {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  /* One line per download: the long texts shrink with an ellipsis (full text
     in the tooltip) and the dismiss button stays pinned at the end. */
  .navegador-view__downloads li {
    display: flex;
    flex-wrap: nowrap;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
  }

  .navegador-view__downloads li > * {
    flex-shrink: 0;
  }

  .navegador-view__downloads li > .navegador-view__download-text {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
