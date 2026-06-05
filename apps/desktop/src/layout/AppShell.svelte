<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { invoke } from '@tauri-apps/api/core'
  import { locale, t } from '$lib/i18n'
  import { navigation } from '$lib/navigation'
  import {
    getCachedDepsStatuses,
    checkAllDeps,
    getUvStatus,
    onDepsComplete,
    CRITICAL_DEPS,
    setCriticalMissing,
    type DepCheckResult,
    type UvStatusResult,
  } from '$lib/deps'
  import {
    getRuntimeStatus,
    onRuntimeStatus,
    repairRuntime,
    runtimeBlocksCurrentUse,
    shouldShowRuntimeRepairAction,
    type RuntimeStatus,
  } from '$lib/runtime'
  import DocumentExplorer from './DocumentExplorer.svelte'
  import TopBar from './TopBar.svelte'
  import EntropicConstellation from './EntropicConstellation.svelte'
  import type { Snippet } from 'svelte'

  const HLAB_URL = 'https://hlab.com.ar/'
  const GITHUB_REPO_URL = 'https://github.com/hlabrepo/EntropIA-Pro'

  let { children }: { children: Snippet } = $props()
  const currentLocale = locale
  const activeLocale = $derived($currentLocale)
  const showExplorer = $derived(
    $navigation.current.name === 'collection' || $navigation.current.name === 'item',
  )

  // ── Ribbon sidebar state ──
  let sidebarOpen = $state(true)
  let searchExpanded = $state(false)
  let searchFilter = $state('')
  let searchInputEl: HTMLInputElement | undefined = $state()
  let showCreateForm = $state(false)

  function toggleSidebar() {
    sidebarOpen = !sidebarOpen
  }

  function expandSearch() {
    searchExpanded = true
    setTimeout(() => searchInputEl?.focus(), 0)
  }

  function collapseSearch() {
    if (!searchFilter) {
      searchExpanded = false
    }
  }

  // Sync sidebar filter to CollectionsView via custom event
  $effect(() => {
    window.dispatchEvent(new CustomEvent('entropia:filter-collections', { detail: searchFilter }))
  })

  function handleCreateCollection() {
    // If already on collections, just open the form
    if ($navigation.current.name === 'collections') {
      window.dispatchEvent(new CustomEvent('entropia:create-collection'))
    } else {
      // Navigate to collections, then signal create form after a tick
      navigation.navigate({ name: 'collections' })
      setTimeout(() => {
        window.dispatchEvent(new CustomEvent('entropia:create-collection'))
      }, 200)
    }
  }

  function handleKeydown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key === 'b') {
      e.preventDefault()
      sidebarOpen = !sidebarOpen
    }
  }

  // ── Deps banner ──
  let depsResults = $state<DepCheckResult[]>([])
  let runtimeStatus = $state<RuntimeStatus | null>(null)
  let uvStatus = $state<UvStatusResult | null>(null)
  let showToast = $state(false)
  let toastDismissed = $state(false)

  const hasCriticalMissing = $derived(
    depsResults.some(
      (d) =>
        CRITICAL_DEPS.includes(d.id) &&
        (d.status.type === 'missing' || d.status.type === 'failed'),
    ),
  )
  const criticalDepsStatusKnown = $derived(
    CRITICAL_DEPS.every((id) => depsResults.some((dep) => dep.id === id)),
  )
  const allCriticalDepsInstalled = $derived(
    criticalDepsStatusKnown &&
    CRITICAL_DEPS.every((id) =>
      depsResults.some((dep) => dep.id === id && dep.status.type === 'installed'),
    ),
  )
  const runtimeBlocksActiveCapabilities = $derived(
    runtimeStatus?.state === 'fixture' && uvStatus?.dev_fallback_available
      ? false
      : runtimeBlocksCurrentUse(
          runtimeStatus,
          criticalDepsStatusKnown && allCriticalDepsInstalled,
          uvStatus?.dev_fallback_available === true,
        ),
  )
  const blockedRuntimeCapabilities = $derived(
    runtimeBlocksActiveCapabilities ? (runtimeStatus?.blockedCapabilities ?? []).join(', ') : '',
  )

  // Sync shared state so TopBar can show the dot
  $effect(() => {
    setCriticalMissing(hasCriticalMissing)
  })

  // Show toast once when critical deps are missing
  $effect(() => {
    if (hasCriticalMissing && !toastDismissed) {
      showToast = true
      const timer = setTimeout(() => { showToast = false }, 8000)
      return () => clearTimeout(timer)
    }
  })

  function dismissToast() {
    showToast = false
    toastDismissed = true
  }

  let unlistenDepsComplete: (() => void) | undefined
  let unlistenRuntimeStatus: (() => void) | undefined

  onMount(async () => {
    document.addEventListener('keydown', handleKeydown)

    unlistenDepsComplete = await onDepsComplete((event) => {
      depsResults = event.results ?? []
      void Promise.all([getRuntimeStatus(), getUvStatus()])
        .then(([status, uv]) => {
          runtimeStatus = status
          uvStatus = uv
        })
        .catch((e) => {
          console.error('[AppShell] deps completion refresh failed', e)
        })
    })

    void getCachedDepsStatuses()
      .then((results) => {
        depsResults = results
      })
      .catch((e) => {
        console.error('[AppShell] cached deps fetch failed', e)
      })

    unlistenRuntimeStatus = await onRuntimeStatus((status) => {
      runtimeStatus = status
    })

    void Promise.all([getRuntimeStatus(), getUvStatus()])
      .then(([status, uv]) => {
        runtimeStatus = status
        uvStatus = uv
      })
      .catch((e) => {
        console.error('[AppShell] runtime status fetch failed', e)
      })
  })

  onDestroy(() => {
    document.removeEventListener('keydown', handleKeydown)
    unlistenDepsComplete?.()
    unlistenRuntimeStatus?.()
  })

  function goToDepSettings() {
    navigation.openRootSection({ name: 'settings' })
  }

  async function handleRuntimeRepair() {
    try {
      runtimeStatus = await repairRuntime()
      const [results, status, uv] = await Promise.all([checkAllDeps(), getRuntimeStatus(), getUvStatus()])
      depsResults = results
      runtimeStatus = status
      uvStatus = uv
    } catch (error) {
      console.error('[AppShell] runtime repair failed', error)
    }
  }

  async function openHlabWebsite(event: MouseEvent) {
    event.preventDefault()
    try {
      await invoke('open_external_url', { url: HLAB_URL })
    } catch (error) {
      console.error('[Footer] No se pudo abrir el sitio de HLab', error)
    }
  }

  async function openGithubRepo(event: MouseEvent) {
    event.preventDefault()
    try {
      await invoke('open_external_url', { url: GITHUB_REPO_URL })
    } catch (error) {
      console.error('[Footer] No se pudo abrir el repositorio de GitHub', error)
    }
  }
</script>

<!-- Fondo constelación entrópica -->
<EntropicConstellation />

<div class="shell">
  <TopBar />

  <div class="workspace">
    <!-- Sidebar: always visible, collapses to icon strip -->
    <aside id="app-sidebar" class="sidebar" class:sidebar--collapsed={!sidebarOpen} aria-label="Panel lateral">
      <!-- Sidebar toolbar -->
      <div class="sidebar__toolbar">
        <!-- Toggle sidebar -->
        <button
          class="sidebar__tool"
          onclick={toggleSidebar}
          aria-label={sidebarOpen ? 'Colapsar panel lateral' : 'Expandir panel lateral'}
          aria-expanded={sidebarOpen}
          aria-controls="app-sidebar"
          title={sidebarOpen ? 'Colapsar panel (Ctrl+B)' : 'Expandir panel (Ctrl+B)'}
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="3" width="18" height="18" rx="2" ry="2"/>
            <line x1="9" y1="3" x2="9" y2="21"/>
          </svg>
        </button>

        {#if sidebarOpen}
          <!-- New collection -->
          <button
            class="sidebar__tool"
            onclick={handleCreateCollection}
            aria-label="Nueva colección"
            title="Nueva colección"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
              <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z"/>
              <line x1="12" y1="11" x2="12" y2="17"/>
              <line x1="9" y1="14" x2="15" y2="14"/>
            </svg>
          </button>

          <!-- Search / filter -->
          {#if searchExpanded}
            <input
              bind:this={searchInputEl}
              class="sidebar__search-input"
              type="text"
              aria-label="Filtrar colecciones"
              placeholder="Filtrar colecciones..."
              bind:value={searchFilter}
              onblur={collapseSearch}
              onkeydown={(e) => { if (e.key === 'Escape') { searchFilter = ''; searchExpanded = false } }}
            />
          {:else}
            <div class="sidebar__toolbar-spacer"></div>
            <button
              class="sidebar__tool"
              onclick={expandSearch}
              aria-label="Filtrar colecciones"
              aria-expanded={searchExpanded}
              title="Filtrar colecciones"
            >
              <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
                <circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/>
              </svg>
            </button>
          {/if}
        {/if}
      </div>

      <!-- Sidebar body (hidden when collapsed) -->
      {#if sidebarOpen}
        <div class="sidebar__body">
          {#if showExplorer}
            <DocumentExplorer filterText={searchFilter} />
          {:else}
            <div class="sidebar__placeholder">
              <p>Abrí una colección para ver el explorador</p>
            </div>
          {/if}
        </div>
      {/if}
    </aside>

    <main class="content">
      {#if runtimeBlocksActiveCapabilities}
        <div class="deps-banner" role="alert">
          <div class="deps-banner__copy">
            <strong>{runtimeStatus?.summary}</strong>
            {#if runtimeStatus?.state === 'fixture'}
              <span>
                La app no se cayó: estás viendo un runtime-pack de desarrollo que todavía requiere
                payloads externos para habilitar OCR, NLP y transcripción.
              </span>
            {/if}
            {#if blockedRuntimeCapabilities}
              <span>Capacidades afectadas: {blockedRuntimeCapabilities}</span>
            {/if}
            {#if runtimeStatus?.guidance?.length}
              <span>{runtimeStatus.guidance[0]}</span>
            {/if}
          </div>
          {#if shouldShowRuntimeRepairAction(runtimeStatus)}
            <button class="deps-banner__btn" type="button" onclick={handleRuntimeRepair}
              >Reparar runtime →</button
            >
          {/if}
        </div>
      {/if}

      {#if criticalDepsStatusKnown && hasCriticalMissing}
        <div class="deps-banner" role="alert">
          <span>⚠ Algunas funciones de IA no están disponibles.</span>
          <button class="deps-banner__btn" type="button" onclick={goToDepSettings}
            >Configurar dependencias →</button
          >
        </div>
      {/if}

      {@render children()}
    </main>

    <!-- Toast notification (appears once, auto-dismisses) -->
    {#if showToast}
      <div class="toast" role="alert">
        <span class="toast__icon">⚠</span>
        <div class="toast__body">
          <span class="toast__title">Dependencias de IA pendientes</span>
          <span class="toast__text">Se necesitan Python y paquetes para OCR/transcripción; embeddings usan OpenRouter.</span>
        </div>
        <button class="toast__action" onclick={goToDepSettings}>Configurar →</button>
        <button class="toast__close" onclick={dismissToast} aria-label="Cerrar">×</button>
      </div>
    {/if}
  </div>

  <!-- Status bar -->
  {#key activeLocale}
    <footer class="statusbar" data-locale={activeLocale}>
      <div class="statusbar__left">
        <span>EntropIA Pro</span>
        <span class="statusbar__sep">·</span>
        <span>{t('appshell.caption')}</span>
      </div>
      <div class="statusbar__center">
        <a
          class="statusbar__link"
          href={GITHUB_REPO_URL}
          onclick={openGithubRepo}
          aria-label={t('appshell.githubAria')}
          title={t('appshell.githubTitle')}
        >
          <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
            <path d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49C3.78 14.2 3.31 12.73 3.31 12.73c-.36-.92-.88-1.16-.88-1.16-.72-.49.05-.48.05-.48.79.06 1.21.82 1.21.82.71 1.21 1.87.86 2.33.66.07-.51.28-.86.5-1.06-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.58.82-2.14-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82A7.64 7.64 0 0 1 8 4.77c.68 0 1.36.09 2 .27 1.53-1.03 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.14 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.06-.01 1.91-.01 2.17 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z"/>
          </svg>
        </a>
      </div>
      <div class="statusbar__right">
        <span>{t('appshell.developedBy')}
          <a class="statusbar__link" href={HLAB_URL} onclick={openHlabWebsite}><b>HLab</b></a>
        </span>
      </div>
    </footer>
  {/key}
</div>

<style>
  .shell {
    position: relative;
    z-index: 1;
    display: flex;
    flex-direction: column;
    height: 100%;
    background: transparent;
  }

  /* ── Workspace: ribbon + sidebar + content ── */
  .workspace {
    display: flex;
    flex: 1;
    min-height: 0;
    overflow: hidden;
    background:
      linear-gradient(180deg, rgba(255, 255, 255, 0.01), transparent 30%),
      color-mix(in srgb, var(--color-bg) 34%, transparent);
  }

  /* ── Sidebar (Zotero-style, always visible) ── */
  .sidebar {
    display: flex;
    flex-direction: column;
    width: 240px;
    flex-shrink: 0;
    border-right: 1px solid var(--color-border-subtle);
    background: var(--color-surface);
    overflow: hidden;
    transition: width var(--transition-base);
  }

  .sidebar--collapsed {
    width: 36px;
  }

  .sidebar__toolbar {
    display: flex;
    align-items: center;
    gap: 1px;
    padding: 3px 4px;
    border-bottom: 1px solid var(--color-border-subtle);
    background: var(--color-surface-sunken);
    flex-shrink: 0;
  }

  .sidebar--collapsed .sidebar__toolbar {
    flex-direction: column;
    padding: 4px 3px;
  }

  .sidebar__toolbar-spacer {
    flex: 1;
  }

  .sidebar__tool {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: none;
    border-radius: 3px;
    background: transparent;
    color: var(--color-text-muted);
    cursor: pointer;
    flex-shrink: 0;
    transition: color var(--transition-base), background-color var(--transition-base);
  }

  .sidebar__tool:hover {
    color: var(--color-text-primary);
    background: var(--color-accent-soft);
  }

  .sidebar__search-input {
    flex: 1;
    min-width: 0;
    height: 26px;
    padding: 0 var(--space-2);
    border: 1px solid var(--color-border);
    border-radius: 3px;
    background: var(--color-surface-raised);
    color: var(--color-text-primary);
    font-size: var(--font-size-xs);
    outline: none;
    transition: border-color var(--transition-base);
  }

  .sidebar__search-input:focus {
    border-color: var(--color-accent);
  }

  .sidebar__search-input::placeholder {
    color: var(--color-text-muted);
  }

  .sidebar__body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }

  .sidebar__placeholder {
    padding: var(--space-6) var(--space-4);
    text-align: center;
  }

  .sidebar__placeholder p {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  /* ── Main content ── */
  .content {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
    padding: var(--space-5);
    background:
      linear-gradient(90deg, rgba(255, 255, 255, 0.012), transparent 18%),
      color-mix(in srgb, var(--color-bg) 24%, transparent);
  }

  .deps-banner {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
    padding: var(--space-3);
    border: 1px solid rgba(245, 158, 11, 0.32);
    border-radius: var(--radius-md);
    background: rgba(245, 158, 11, 0.08);
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .deps-banner__copy {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .deps-banner__copy strong {
    color: var(--color-text-primary);
  }

  .deps-banner__btn {
    flex-shrink: 0;
    padding: 2px var(--space-3);
    border: 1px solid rgba(245, 158, 11, 0.5);
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-warning);
    font-size: var(--font-size-xs);
    cursor: pointer;
    transition: background-color var(--transition-base);
  }

  .deps-banner__btn:hover {
    background: rgba(245, 158, 11, 0.12);
  }

  /* ── Toast notification ── */
  .toast {
    position: fixed;
    bottom: 36px;
    right: var(--space-4);
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-3);
    background: var(--color-surface-elevated);
    border: 1px solid var(--color-border);
    border-radius: 4px;
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
    z-index: 1000;
    animation: toast-in 0.3s ease;
  }

  @keyframes toast-in {
    from { opacity: 0; transform: translateY(8px); }
    to { opacity: 1; transform: translateY(0); }
  }

  .toast__icon {
    font-size: var(--font-size-sm);
    color: var(--color-warning);
    align-self: flex-start;
    margin-top: 2px;
  }

  .toast__body {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }

  .toast__title {
    font-weight: 600;
    color: var(--color-text-primary);
  }

  .toast__text {
    color: var(--color-text-muted);
  }

  .toast__action {
    padding: 2px var(--space-2);
    border: 1px solid var(--color-accent);
    border-radius: 2px;
    background: transparent;
    color: var(--color-accent);
    font-size: var(--font-size-xs);
    cursor: pointer;
    transition: background-color var(--transition-base);
  }

  .toast__action:hover {
    background: var(--color-accent-soft);
  }

  .toast__close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    border: none;
    border-radius: 2px;
    background: transparent;
    color: var(--color-text-muted);
    font-size: 14px;
    cursor: pointer;
    transition: color var(--transition-base);
  }

  .toast__close:hover {
    color: var(--color-text-primary);
  }

  /* ── Status bar (compact, replaces footer) ── */
  .statusbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 26px;
    padding: 0 var(--space-3);
    border-top: 1px solid var(--color-border-subtle);
    background: var(--color-surface-sunken);
    font-family: var(--font-mono);
    font-size: 0.6rem;
    color: var(--color-text-muted);
    flex-shrink: 0;
    letter-spacing: 0.02em;
  }

  .statusbar__left,
  .statusbar__center,
  .statusbar__right {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .statusbar__right {
    justify-content: flex-end;
  }

  .statusbar__sep {
    opacity: 0.4;
  }

  .statusbar__link {
    display: inline-flex;
    align-items: center;
    color: var(--color-text-muted);
    text-decoration: none;
    transition: color var(--transition-base);
  }

  .statusbar__link:hover {
    color: var(--color-accent);
  }

  .statusbar__link b {
    font-weight: 600;
  }
</style>
