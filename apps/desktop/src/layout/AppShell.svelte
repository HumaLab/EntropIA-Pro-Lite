<script lang="ts">
  import { onMount, onDestroy } from 'svelte'
  import { openExternalUrl, openExternalUrlFromClick } from '$lib/external-links'
  import { MICROSOFT_STORE_PRODUCT_URI } from '$lib/store-updates'
  import { locale, t } from '$lib/i18n'
  import { workspace } from '$lib/workspace'
  import { requestCreateCollection } from '$lib/document-explorer'
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
  import { LOCAL_ML } from '$lib/capabilities'
  import { APP_VERSION, GITHUB_REPO_URL, PRODUCT_NAME_BADGE } from '$lib/product'
  import { watchStacking } from '$lib/resize-stacking'
  import { clampSplitRatio } from '$lib/split-ratio'
  import { tooltip, TooltipLayer, ActionIcon, Button, IconButton, StatusBadge } from '@entropia/ui'
  import DocumentExplorer from './DocumentExplorer.svelte'
  import TopBar from './TopBar.svelte'
  import EntropicConstellation from './EntropicConstellation.svelte'
  import SyncStatusIndicator from './SyncStatusIndicator.svelte'
  import BatchStatusIndicator from './BatchStatusIndicator.svelte'
  import NotificationBell from './NotificationBell.svelte'
  import WorkPane from './WorkPane.svelte'
  import SplitDivider from './SplitDivider.svelte'

  const HLAB_URL = 'https://hlab.com.ar/'

  let {
    storeUpdateAvailable = false,
    onDismissStoreUpdate,
  }: {
    storeUpdateAvailable?: boolean
    onDismissStoreUpdate?: () => void
  } = $props()
  const currentLocale = locale
  // The active tab can change (TabStrip, Task 2.1), so chrome that used to
  // read a single frozen `workspace.activeNavigation` capture must instead
  // re-derive which tab is active from the subscribed workspace snapshot on
  // every emit, then re-subscribe (via the `$`-prefixed store read below) to
  // whichever NavigationStore that tab currently owns — plain `.current`
  // reads on a `$derived`-held NavigationStore reference would freeze: the
  // derived only recomputes when the active tab *id* changes, not when that
  // tab's own history changes, so `$activeNav` (not `activeNav`) is what
  // keeps this reactive to in-tab navigation too.
  const wsSnapshot = $derived($workspace)
  const activeNav = $derived(workspace.navigationFor(wsSnapshot.activeTabId))
  // Recomputed from the subscribed `wsSnapshot`, not from a bare
  // `workspace.visiblePaneIds` getter call: the getter reads plain class
  // fields with no rune/store involved, so a `$derived` that called it
  // directly would capture only its value at mount and never update again
  // (the same freeze pitfall as reading a NavigationStore without `$`,
  // carried forward from Task 1.4/2.3's rulings).
  const visiblePaneIds: readonly [string, string] | readonly [string] = $derived(
    wsSnapshot.split &&
      (wsSnapshot.activeTabId === wsSnapshot.split.leftId ||
        wsSnapshot.activeTabId === wsSnapshot.split.rightId)
      ? ([wsSnapshot.split.leftId, wsSnapshot.split.rightId] as const)
      : ([wsSnapshot.activeTabId] as const)
  )
  const isSplit = $derived(visiblePaneIds.length === 2)
  // Whether the split container is narrower than two 320px panes side by
  // side (spec, Responsive) — driven live by `watchStacking` below; vertical
  // (side-by-side) is the correct default until the first ResizeObserver
  // callback fires (or forever, if ResizeObserver is unavailable there).
  let stacked = $state(false)
  let splitContainerEl: HTMLElement | undefined = $state()
  // Raw pixel size of the split container along each axis, kept live via
  // Svelte's own `bind:client*` (not `watchStacking`, which only reports the
  // stacked/not-stacked transition) so the render-time ratio below can be
  // clamped to the CURRENT size rather than the size the ratio was last
  // dragged/stored at. happy-dom has no real layout, so both stay 0 in
  // tests; the real proof is a human's narrow-window check.
  let splitWidth = $state(0)
  let splitHeight = $state(0)

  $effect(() => {
    if (!splitContainerEl) return
    return watchStacking(splitContainerEl, (next) => {
      stacked = next
    })
  })

  // Clamped to the container's current size along the active axis so a ratio
  // stored on a wide window never squeezes a pane below 320px on a narrower
  // one — the stored ratio itself is never rewritten just because the
  // window shrank (Task 3.4 ruling).
  const clampedSplitRatio = $derived(
    clampSplitRatio(wsSnapshot.split?.ratio ?? 0.5, stacked ? splitHeight : splitWidth)
  )
  // The left pane takes the ratio as its basis side by side, or as its grow
  // share when stacked; the right pane fills the rest. A lone pane keeps the
  // stylesheet's plain `flex: 1 1 0`.
  function paneFlexBasis(index: number): string | undefined {
    if (!isSplit || index !== 0) return undefined
    return stacked ? 'auto' : `${clampedSplitRatio * 100}%`
  }
  function paneFlexGrow(index: number): number | undefined {
    if (!isSplit) return undefined
    if (index === 0) return stacked ? clampedSplitRatio * 100 : 0
    return stacked ? (1 - clampedSplitRatio) * 100 : 1
  }
  const activeLocale = $derived($currentLocale)
  const sidebarLabels = $derived.by(() => {
    $currentLocale
    return {
      aria: t('appshell.sidebarAria'),
      collapse: t('appshell.sidebarCollapse'),
      expand: t('appshell.sidebarExpand'),
      newCollection: t('appshell.sidebarNewCollection'),
      filter: t('appshell.sidebarFilterCollections'),
      filterPlaceholder: t('appshell.sidebarFilterCollectionsPlaceholder'),
    }
  })
  // The explorer belongs to the Collections hierarchy only. The tree is the
  // collection list, so it stands on its own on the Collections root too — with
  // no collection open, no row is marked active. The root sections reachable
  // from that breadcrumb (database, chat, settings) are not part of it.
  const showExplorer = $derived(
    $activeNav.current.name === 'collections' ||
      $activeNav.current.name === 'collection' ||
      $activeNav.current.name === 'item'
  )

  // ── Ribbon sidebar state ──
  let sidebarOpen = $state(true)
  let searchExpanded = $state(false)
  let searchFilter = $state('')
  let searchInputEl: HTMLInputElement | undefined = $state()

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
    requestCreateCollection($activeNav.current.name === 'collections')
  }

  function isEditableTarget(target: EventTarget | null): boolean {
    if (!(target instanceof Element)) return false

    const tagName = target.tagName.toLowerCase()
    return (
      tagName === 'input' ||
      tagName === 'textarea' ||
      tagName === 'select' ||
      target.closest('[contenteditable="true"]') !== null
    )
  }

  function handleKeydown(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key === 'b') {
      // Editors use Ctrl+B for bold (e.g. the TipTap note editor); leave it to them.
      if (e.defaultPrevented || isEditableTarget(e.target)) return
      e.preventDefault()
      sidebarOpen = !sidebarOpen
    }
  }

  // ── Microsoft Store update notice (Lite) ──
  let storeOpenFailed = $state(false)

  // Opening the listing neither installs nor dismisses anything. rundll32 only
  // reports a failure to launch, so there is no success to announce.
  async function openStoreListing() {
    storeOpenFailed = false
    try {
      await openExternalUrl(MICROSOFT_STORE_PRODUCT_URI)
    } catch (e) {
      console.error('[AppShell] Opening Microsoft Store failed:', e)
      storeOpenFailed = true
    }
  }

  // The close button leaves with the notice; focus goes to the content area
  // instead of being dropped on the document. The area is focusable only for
  // that hand-off, so clicking the content never focuses it.
  function dismissStoreUpdate(event: MouseEvent) {
    const content = (event.currentTarget as HTMLElement).closest('main')
    onDismissStoreUpdate?.()
    if (!content) return
    content.tabIndex = -1
    content.addEventListener('blur', () => content.removeAttribute('tabindex'), { once: true })
    content.focus({ preventScroll: true })
  }

  // ── Deps banner (Pro-only local subsystem) ──
  let depsResults = $state<DepCheckResult[]>([])
  let runtimeStatus = $state<RuntimeStatus | null>(null)
  let uvStatus = $state<UvStatusResult | null>(null)

  const hasCriticalMissing = $derived(
    depsResults.some(
      (d) =>
        CRITICAL_DEPS.includes(d.id) && (d.status.type === 'missing' || d.status.type === 'failed')
    )
  )
  const criticalDepsStatusKnown = $derived(
    CRITICAL_DEPS.every((id) => depsResults.some((dep) => dep.id === id))
  )
  const allCriticalDepsInstalled = $derived(
    criticalDepsStatusKnown &&
      CRITICAL_DEPS.every((id) =>
        depsResults.some((dep) => dep.id === id && dep.status.type === 'installed')
      )
  )
  const runtimeBlocksActiveCapabilities = $derived(
    runtimeStatus?.state === 'fixture' && uvStatus?.dev_fallback_available
      ? false
      : runtimeBlocksCurrentUse(
          runtimeStatus,
          criticalDepsStatusKnown && allCriticalDepsInstalled,
          uvStatus?.dev_fallback_available === true
        )
  )
  const blockedRuntimeCapabilities = $derived(
    runtimeBlocksActiveCapabilities ? (runtimeStatus?.blockedCapabilities ?? []).join(', ') : ''
  )

  // Critical-missing is announced through a single persistent channel: the
  // actionable deps banner in <main> (see template below). The TopBar badge is
  // kept as a discreet indicator via the shared state synced here. The legacy
  // toast was removed so banner and toast never co-exist for this state (#27).
  $effect(() => {
    setCriticalMissing(hasCriticalMissing)
  })

  let unlistenDepsComplete: (() => void) | undefined
  let unlistenRuntimeStatus: (() => void) | undefined

  onMount(async () => {
    document.addEventListener('keydown', handleKeydown)

    // The local dependency manager and managed runtime only exist in the
    // local-ML (Pro) build. Skip the dead deps/runtime wiring under the
    // API-only variant — the kept-stubs return inert values anyway.
    if (!LOCAL_ML) return

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
    workspace.navigateActive({ name: 'settings' })
  }

  async function handleRuntimeRepair() {
    try {
      runtimeStatus = await repairRuntime()
      const [results, status, uv] = await Promise.all([
        checkAllDeps(),
        getRuntimeStatus(),
        getUvStatus(),
      ])
      depsResults = results
      runtimeStatus = status
      uvStatus = uv
    } catch (error) {
      console.error('[AppShell] runtime repair failed', error)
    }
  }

  async function openHlabWebsite(event: MouseEvent) {
    try {
      await openExternalUrlFromClick(event, HLAB_URL)
    } catch (error) {
      console.error('[Footer] No se pudo abrir el sitio de HLab', error)
    }
  }

  async function openGithubRepo(event: MouseEvent) {
    try {
      await openExternalUrlFromClick(event, GITHUB_REPO_URL)
    } catch (error) {
      console.error('[Footer] No se pudo abrir el repositorio de GitHub', error)
    }
  }
</script>

<!-- Fondo constelación entrópica: animada solo en Inicio (home-view.md T6) -->
<EntropicConstellation animated={$activeNav.current.name === 'home'} />

<div class="shell">
  <!-- One bubble for the whole application, mounted here so it escapes every
       overflow-hidden panel below and needs a single z-index. -->
  <TooltipLayer />

  <TopBar />

  <div class="workspace" class:workspace--home={$activeNav.current.name === 'home'}>
    <!-- Sidebar: only mounted inside the Collections hierarchy, so the root
         sections (database, chat, settings) get the full workspace width. -->
    {#if showExplorer}
      <aside
        class="sidebar"
        class:sidebar--collapsed={!sidebarOpen}
        aria-label={sidebarLabels.aria}
      >
        <!-- Sidebar toolbar -->
        <div class="sidebar__toolbar">
          <!-- Toggle sidebar -->
          <IconButton
            class="sidebar__tool"
            size="sm"
            variant="ghost"
            label={sidebarOpen ? sidebarLabels.collapse : sidebarLabels.expand}
            onclick={toggleSidebar}
            title={sidebarOpen ? sidebarLabels.collapse : sidebarLabels.expand}
          >
            <ActionIcon name={sidebarOpen ? 'panel-left-close' : 'panel-left'} size={16} />
          </IconButton>

          {#if sidebarOpen}
            <!-- New collection -->
            <IconButton
              class="sidebar__tool"
              size="sm"
              variant="ghost"
              label={sidebarLabels.newCollection}
              onclick={handleCreateCollection}
              title={sidebarLabels.newCollection}
            >
              <ActionIcon name="folder-plus" size={16} />
            </IconButton>

            <!-- Search / filter -->
            {#if searchExpanded}
              <input
                bind:this={searchInputEl}
                class="sidebar__search-input"
                type="text"
                placeholder={sidebarLabels.filterPlaceholder}
                bind:value={searchFilter}
                onblur={collapseSearch}
                onkeydown={(e) => {
                  if (e.key === 'Escape') {
                    searchFilter = ''
                    searchExpanded = false
                  }
                }}
              />
            {:else}
              <div class="sidebar__toolbar-spacer"></div>
              <IconButton
                class="sidebar__tool"
                size="sm"
                variant="ghost"
                label={sidebarLabels.filter}
                onclick={expandSearch}
                title={sidebarLabels.filter}
              >
                <ActionIcon name="search" size={16} />
              </IconButton>
            {/if}
          {/if}
        </div>

        <!-- Sidebar body (hidden when collapsed) -->
        {#if sidebarOpen}
          <div class="sidebar__body">
            <DocumentExplorer filterText={searchFilter} />
          </div>
        {/if}
      </aside>
    {/if}

    <main
      class="content"
      class:content--item={$activeNav.current.name === 'item'}
      class:content--home={$activeNav.current.name === 'home'}
    >
      {#if storeUpdateAvailable}
        <section class="store-update" aria-labelledby="store-update-title">
          <div class="store-update__copy" role="status" aria-live="polite">
            <strong id="store-update-title">{t('storeUpdate.title')}</strong>
            <span>{t('storeUpdate.body')}</span>
            {#if storeOpenFailed}
              <span class="store-update__error">{t('storeUpdate.openError')}</span>
            {/if}
          </div>
          <div class="store-update__actions">
            <Button variant="secondary" size="sm" onclick={openStoreListing}>
              <ActionIcon name="external-link" size={14} />
              {t('storeUpdate.view')}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              aria-label={t('storeUpdate.closeLabel')}
              onclick={dismissStoreUpdate}
            >
              {t('storeUpdate.close')}
            </Button>
          </div>
        </section>
      {/if}

      {#if LOCAL_ML}
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
              <button class="deps-banner__btn" type="button" onclick={handleRuntimeRepair}>
                <ActionIcon name="wrench" size={16} />
                Reparar runtime
              </button>
            {/if}
          </div>
        {/if}

        {#if criticalDepsStatusKnown && hasCriticalMissing}
          <div class="deps-banner" role="alert">
            <span class="deps-banner__message">
              <ActionIcon name="triangle-alert" size={20} />
              Algunas funciones de IA no están disponibles.
            </span>
            <button class="deps-banner__btn" type="button" onclick={goToDepSettings}>
              <ActionIcon name="settings" size={16} />
              Configurar dependencias
            </button>
          </div>
        {/if}
      {/if}

      <!-- One keyed list for one pane or two, so turning split on or off
           never remounts the pane that stays on screen (final review item 5):
           a template-branch switch here would throw away its editor state,
           scroll and in-flight work. A tab switch still remounts, because the
           key is the tab id. -->
      <div
        class="content__split"
        class:content__split--stacked={isSplit && stacked}
        bind:this={splitContainerEl}
        bind:clientWidth={splitWidth}
        bind:clientHeight={splitHeight}
      >
        {#each visiblePaneIds as paneId, index (paneId)}
          {#if index === 1}
            <SplitDivider
              ratio={clampedSplitRatio}
              orientation={stacked ? 'horizontal' : 'vertical'}
              onratiochange={(r) => workspace.setSplitRatio(r)}
            />
          {/if}
          <div
            class="content__pane"
            class:content__pane--active={isSplit && wsSnapshot.activeTabId === paneId}
            style:flex-basis={paneFlexBasis(index)}
            style:flex-grow={paneFlexGrow(index)}
            onfocusin={() => workspace.activateTab(paneId)}
            onpointerdowncapture={() => workspace.activateTab(paneId)}
          >
            <WorkPane {paneId} />
          </div>
        {/each}
      </div>
    </main>
  </div>

  <!-- Status bar -->
  {#key activeLocale}
    <footer class="statusbar" data-locale={activeLocale}>
      <div class="statusbar__left">
        <StatusBadge variant="neutral" size="sm" class="statusbar__badge"
          >{PRODUCT_NAME_BADGE}</StatusBadge
        >
        <span class="statusbar__sep">·</span>
        <span>{APP_VERSION}</span>
        <span class="statusbar__sep">·</span>
        <span class="statusbar__caption">{t('appshell.caption')}</span>
      </div>
      <div class="statusbar__center">
        <a
          class="statusbar__link"
          href={GITHUB_REPO_URL}
          onclick={openGithubRepo}
          aria-label={t('appshell.githubAria')}
          use:tooltip={t('appshell.githubTitle')}
        >
          <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true">
            <path
              d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49C3.78 14.2 3.31 12.73 3.31 12.73c-.36-.92-.88-1.16-.88-1.16-.72-.49.05-.48.05-.48.79.06 1.21.82 1.21.82.71 1.21 1.87.86 2.33.66.07-.51.28-.86.5-1.06-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.58.82-2.14-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82A7.64 7.64 0 0 1 8 4.77c.68 0 1.36.09 2 .27 1.53-1.03 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.14 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.06-.01 1.91-.01 2.17 0 .21.15.46.55.38A8.01 8.01 0 0 0 16 8c0-4.42-3.58-8-8-8Z"
            />
          </svg>
        </a>
      </div>
      <div class="statusbar__right">
        <SyncStatusIndicator />
        <BatchStatusIndicator />
        <NotificationBell />
        <span
          >{t('appshell.developedBy')}
          <a class="statusbar__link" href={HLAB_URL} onclick={openHlabWebsite}><b>HLab</b></a>
        </span>
      </div>
    </footer>
  {/key}
</div>

<style>
  .shell {
    --statusbar-height: 30px;
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
    background: color-mix(in srgb, var(--surface-app) 72%, transparent);
  }

  /* ── Sidebar (Zotero-style, always visible) ── */
  .sidebar {
    display: flex;
    flex-direction: column;
    flex: 0 0 auto;
    background: var(--surface-panel);
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
    border-bottom: 1px solid var(--border-subtle);
    background: color-mix(in srgb, var(--surface-toolbar) 78%, transparent);
    flex-shrink: 0;
  }

  .sidebar--collapsed .sidebar__toolbar {
    flex-direction: column;
    padding: 4px 3px;
  }

  .sidebar__toolbar-spacer {
    flex: 1;
  }

  :global(.sidebar__tool) {
    border-radius: var(--radius-sm);
    color: var(--color-text-muted);
  }

  :global(.sidebar__tool:hover:not(:disabled)) {
    background: var(--color-accent-faint);
  }

  .sidebar__search-input {
    flex: 1;
    min-width: 0;
    height: 26px;
    padding: 0 var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-sm);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font-size: var(--font-size-xs);
    outline: none;
    transition: border-color var(--transition-base);
  }

  .sidebar__search-input:focus {
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
  }

  .sidebar__search-input::placeholder {
    color: var(--color-text-muted);
  }

  .sidebar__body {
    display: flex;
    flex: 1;
    min-height: 0;
  }

  /* ── Main content ── */
  /* A column flex container, not just a block: WorkPane (and, before Stage 2,
     each routed view directly) needs a flex *container* with a definite
     height to fill via `flex: 1; min-height: 0`, otherwise it collapses to
     its own content height instead of filling the pane. */
  .content {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow-y: auto;
    padding: 0 var(--space-5);
    background: color-mix(in srgb, var(--surface-app) 42%, transparent);
  }

  /* Inicio is the one view that animates the constellation; the two veils
     above (72 % and 42 %) would hide ~84 % of it, so they step aside there. */
  .workspace--home {
    background: transparent;
  }

  .content--home {
    background: transparent;
  }

  .content--item {
    padding-block-end: 0;
  }

  /* The split-view row wrapper: `.content` is a COLUMN flex container (the
     banners above it stack), so this needs its own `flex: 1; min-height: 0`
     to fill the remaining height rather than the `height: 100%` a plain
     nested box would need — matching how the single-pane `.work-pane`
     already fills `.content` (Stage 2 visual fix, carried forward to 3.3). */
  .content__split {
    display: flex;
    flex: 1;
    min-width: 0;
    min-height: 0;
  }

  .content__split--stacked {
    flex-direction: column;
  }

  /* `WorkPane.svelte`'s own `.work-pane { min-width: 320px }` (the
     side-by-side floor) must not force horizontal overflow once stacked —
     a `:global()` override, since that class belongs to WorkPane's own
     scoped styles, not this component's (spec, Responsive). */
  :global(.content__split--stacked .work-pane) {
    min-width: 0;
  }

  .content__pane {
    display: flex;
    min-width: 0;
    min-height: 0;
    flex: 1 1 0;
    overflow: hidden;
  }

  /* The active pane is the last one clicked or focused (spec, Split view):
     marked with a thin accent border. */
  .content__pane--active {
    box-shadow: inset 0 0 0 1px var(--color-accent);
  }

  /* Focus lands here only after the Store notice closes, never through the tab
     order, so it needs no ring. */
  .content:focus {
    outline: none;
  }

  /* ── Microsoft Store update notice (Lite) ── */
  .store-update {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    margin-block: var(--space-3);
    padding: var(--space-3);
    border: 1px solid color-mix(in srgb, var(--color-accent) 28%, var(--color-hairline));
    border-radius: var(--radius-md);
    background: color-mix(in srgb, var(--color-accent) 8%, transparent);
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .store-update__copy {
    display: flex;
    flex: 1 1 16rem;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .store-update__copy strong {
    color: var(--color-text-primary);
  }

  .store-update__error {
    color: var(--color-danger);
  }

  .store-update__actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  /* ── Deps / runtime banner (Pro-only local subsystem) ── */
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

  /* Leading icon + label: the arrow glyph these buttons used to end with said
     nothing the label did not already say. The icon names the action instead. */
  .deps-banner__message {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
  }

  .deps-banner__btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    flex-shrink: 0;
    padding: var(--space-1) var(--space-3);
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

  /* ── Status bar (compact, replaces footer) ── */
  .statusbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: var(--statusbar-height);
    padding: 0 var(--space-3);
    border-top: 1px solid var(--border-subtle);
    background: var(--surface-input);
    font-family: var(--font-mono);
    font-size: var(--font-size-sm);
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
    /* The bar is one line tall by declaration (`--statusbar-height`), so text
       that wraps does not make it taller — it spills out of it. Every group
       therefore stays on one line, and what cannot fit is dealt with below by
       deciding what yields rather than by letting the layout decide. */
    white-space: nowrap;
  }

  /* Neither the link nor the indicators may be squeezed: an icon at 80% of its
     size is not smaller, it is broken. They keep their width and the left group
     gives way instead. */
  .statusbar__center,
  .statusbar__right {
    flex-shrink: 0;
  }

  .statusbar__right {
    justify-content: flex-end;
  }

  /* The only group that yields, because it is the only one holding something
     nobody needs complete. `min-width: 0` is what lets it: a flex child's
     default floor is its content, which is precisely why the bar overflowed
     instead of shortening. */
  .statusbar__left {
    min-width: 0;
  }

  /* And within it, the tagline goes first. The product name and the version
     identify the build someone is looking at — a truncated version number is
     worse than no version number — while a sentence describing the product is
     the one thing in this bar that can end in an ellipsis and lose nothing. */
  .statusbar__caption {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
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
    font-weight: var(--font-weight-semibold);
  }

  :global(.statusbar__badge) {
    min-height: 18px;
    padding: 0 var(--space-2);
    font-size: calc(0.58rem + 3px);
    letter-spacing: 0.04em;
  }
</style>
