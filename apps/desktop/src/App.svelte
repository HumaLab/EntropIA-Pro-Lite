<script lang="ts">
  import { onMount, tick } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { initDb } from '$lib/db'
  import { primeDataDir } from '$lib/file-import'
  import { setupKeyboardShortcuts } from '$lib/keyboard'
  import { initZoom } from '$lib/zoom'
  import { initializeAppearance } from '$lib/appearance'
  import { initLocale, t } from '$lib/i18n'
  import { resolveDesktopPlatform } from '$lib/platform'
  import { PRODUCT_NAME } from '$lib/product'
  import { checkMicrosoftStoreUpdate, type StoreUpdateStatus } from '$lib/store-updates'
  import { waitForFirstPaint } from '$lib/first-paint'
  import { matchWindowBackground } from '$lib/window-background'
  import startupMark from './assets/hlab-mark.png'
  import AppShell from './layout/AppShell.svelte'

  let ready = $state(false)
  let error = $state<string | null>(null)
  // Owned here so a dismissal outlives navigation but not the session.
  let storeUpdate = $state<StoreUpdateStatus | null>(null)
  let storeNoticeDismissed = $state(false)
  let storeCheckStarted = false

  // Outside the startup chain: it never delays the window nor fails startup.
  function checkStoreUpdate() {
    if (storeCheckStarted) return
    storeCheckStarted = true
    checkMicrosoftStoreUpdate().then(
      (status) => {
        storeUpdate = status
      },
      (e) => {
        console.error('[App] Store update check failed:', e)
      }
    )
  }

  // The main window starts hidden behind the native startup window (src-tauri/src/splash.rs).
  // Handing over only once this view has actually painted avoids showing an empty
  // window for a frame where the engine paints hidden windows (WebView2, which
  // always waits for the frames); where it does not (WKWebView, WebKitGTK)
  // waitForFirstPaint gives up after a short bound instead of leaving the reveal
  // to Rust's 20 s watchdog.
  async function dismissSplash() {
    await tick()
    await waitForFirstPaint()
    // Where the window is revealed before its first frame (WebKitGTK), the
    // frame before it shows the window's own background: make it the theme's.
    await matchWindowBackground()
    try {
      await invoke('splash_finish')
    } catch (e) {
      console.error('[App] splash_finish ERROR:', e)
    }
  }

  function initializeApp() {
    ready = false
    error = null
    // primeDataDir() gates the first render: getAssetUrl is synchronous and
    // needs the data directory to resolve relative asset keys.
    Promise.all([initLocale(), initDb(), primeDataDir()])
      .then(() => {
        ready = true
        // Needs the settings store, so it waits for initDb().
        void initZoom()
      })
      .catch((e) => {
        console.error('[App] init ERROR:', e)
        error = e instanceof Error ? e.message : t('app.initError')
      })
      .finally(() => {
        void dismissSplash().then(() => {
          if (ready) checkStoreUpdate()
        })
      })
  }

  onMount(() => {
    document.documentElement.dataset.platform = resolveDesktopPlatform()
    // Theme, contrast and font used to apply once TopBar/TypographyMenu
    // mounted, deep inside AppShell. Both preferences now live in the
    // Apariencia settings tab, so this is the startup path that restores them
    // instead — same storage keys, same defaults, applied before first paint.
    initializeAppearance()
    const cleanupKeyboard = setupKeyboardShortcuts()

    initializeApp()

    return cleanupKeyboard
  })
</script>

{#if !ready && !error}
  <main class="startup" aria-labelledby="startup-title">
    <section class="startup-card" role="status" aria-live="polite">
      <img class="startup-mark" src={startupMark} alt="" />
      <div class="startup-copy">
        <p class="startup-eyebrow">{PRODUCT_NAME}</p>
        <h1 id="startup-title">{t('app.startupTitle')}</h1>
        <p>{t('app.initializing')}</p>
      </div>
    </section>
  </main>
{:else if error}
  <main class="startup" aria-labelledby="startup-error-title">
    <section class="startup-card startup-card--error" role="alert" aria-live="assertive">
      <div class="startup-mark startup-mark--error" aria-hidden="true">!</div>
      <div class="startup-copy">
        <p class="startup-eyebrow">{PRODUCT_NAME}</p>
        <h1 id="startup-error-title">{t('app.initError')}</h1>
        <p>{error}</p>
      </div>
      <button type="button" class="startup-action" onclick={initializeApp}
        >{t('app.retryInit')}</button
      >
    </section>
  </main>
{:else}
  <AppShell
    storeUpdateAvailable={storeUpdate === 'available' && !storeNoticeDismissed}
    onDismissStoreUpdate={() => {
      storeNoticeDismissed = true
    }}
  />
{/if}

<style>
  .startup {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 100%;
    padding: var(--space-5);
    background:
      radial-gradient(
        circle at 50% 18%,
        color-mix(in srgb, var(--color-accent) 12%, transparent),
        transparent 34%
      ),
      var(--surface-app, var(--color-bg));
  }

  .startup-card {
    display: grid;
    grid-template-columns: auto minmax(0, 1fr);
    align-items: center;
    gap: var(--space-4);
    width: min(100%, 440px);
    padding: var(--space-5);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: color-mix(in srgb, var(--color-surface-glass) 88%, transparent);
    box-shadow: var(--shadow-surface);
  }

  .startup-card--error {
    border-color: color-mix(in srgb, var(--color-danger) 32%, var(--color-hairline));
  }

  .startup-mark {
    display: block;
    width: 44px;
    height: 44px;
    object-fit: contain;
  }

  .startup-mark--error {
    background: var(--color-danger-soft);
    color: var(--color-danger);
  }

  .startup-copy {
    display: grid;
    gap: var(--space-1);
  }

  .startup-eyebrow {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.075em;
    text-transform: uppercase;
  }

  .startup-copy p:last-child {
    color: var(--color-text-secondary);
  }

  .startup-action {
    grid-column: 2;
    justify-self: start;
    min-height: var(--control-height-md);
    padding: 0 var(--space-3);
    border: 1px solid color-mix(in srgb, var(--color-accent) 22%, var(--color-hairline));
    border-radius: var(--radius-control);
    background: color-mix(in srgb, var(--color-accent) 14%, var(--color-surface-glass));
    color: var(--color-text-primary);
    cursor: pointer;
    transition:
      background-color var(--transition-smooth),
      border-color var(--transition-smooth);
  }

  .startup-action:hover {
    border-color: color-mix(in srgb, var(--color-accent) 40%, var(--color-hairline));
    background: color-mix(in srgb, var(--color-accent) 20%, var(--color-surface-glass));
  }

  .startup-action:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  @media (max-width: 520px) {
    .startup {
      padding: var(--space-4);
    }

    .startup-card {
      grid-template-columns: 1fr;
      justify-items: start;
    }

    .startup-action {
      grid-column: 1;
    }
  }
</style>
