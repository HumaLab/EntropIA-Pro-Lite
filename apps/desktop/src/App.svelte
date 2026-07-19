<script lang="ts">
  import { onMount } from 'svelte'
  import { initDb } from '$lib/db'
  import { navigation } from '$lib/navigation'
  import { setupKeyboardShortcuts } from '$lib/keyboard'
  import { initLocale, t } from '$lib/i18n'
  import { resolveDesktopPlatform } from '$lib/platform'
  import { PRODUCT_NAME } from '$lib/product'
  import type { View } from '$lib/navigation'
  import startupMark from './assets/hlab-mark.png'
  import AppShell from './layout/AppShell.svelte'
  import CollectionsView from './views/CollectionsView.svelte'
  import CollectionView from './views/CollectionView.svelte'
  import ItemView from './views/ItemView.svelte'
  import DbBrowserView from './views/DbBrowserView.svelte'
  import RagChatView from './views/RagChatView.svelte'
  import SettingsView from './views/SettingsView.svelte'

  let ready = $state(false)
  let error = $state<string | null>(null)
  const currentView = $derived($navigation.current as View)
  const currentViewName = $derived(($navigation.current as { name: string }).name)

  function initializeApp() {
    ready = false
    error = null
    Promise.all([initLocale(), initDb()])
      .then(() => {
        ready = true
      })
      .catch((e) => {
        console.error('[App] init ERROR:', e)
        error = e instanceof Error ? e.message : t('app.initError')
      })
  }

  onMount(() => {
    document.documentElement.dataset.platform = resolveDesktopPlatform()
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
      <button type="button" class="startup-action" onclick={initializeApp}>{t('app.retryInit')}</button>
    </section>
  </main>
{:else}
  <AppShell>
    {#if currentViewName === 'collections'}
      <CollectionsView />
    {:else if currentViewName === 'collection'}
      <CollectionView collectionId={(currentView as Extract<View, { name: 'collection' }>).id} />
    {:else if currentViewName === 'item'}
      <ItemView
        itemId={(currentView as Extract<View, { name: 'item' }>).itemId}
        collectionId={(currentView as Extract<View, { name: 'item' }>).collectionId}
      />
    {:else if currentViewName === 'db-browser'}
      <DbBrowserView />
    {:else if currentViewName === 'rag-chat'}
      <RagChatView />
    {:else if currentViewName === 'settings'}
      <SettingsView />
    {/if}
  </AppShell>
{/if}

<style>
  .startup {
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 100%;
    padding: var(--space-5);
    background:
      radial-gradient(circle at 50% 18%, color-mix(in srgb, var(--color-accent) 12%, transparent), transparent 34%),
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
