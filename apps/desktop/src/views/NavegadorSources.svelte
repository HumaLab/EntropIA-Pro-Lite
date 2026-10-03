<script lang="ts">
  /**
   * The saved web sources, as a drawer beside the browser.
   *
   * It lives in the Navegador view and is laid out OUTSIDE the placeholder that
   * marks where the native webview goes: opening it narrows the placeholder and
   * the view's ResizeObserver moves the webview with it. (The webview draws
   * above every HTML element, so anything that overlapped it would be hidden.)
   *
   * Reads and deletes go through Rust commands (`lib/navegador-sources.ts`);
   * the drawer names a source by id and never handles a path. Everything shown
   * came from a web page and is rendered as text. A saved HTML snapshot is only
   * reported (it exists, how big it is): showing it needs its active content
   * stripped first, which is not built. A saved PDF can be opened in the app's
   * own viewer (`onviewpdf`, by capture id), and a source made of PDFs opens its
   * page of origin rather than "the browser". A saved PDF can also be copied into
   * a collection (`NavegadorCopyDialog`): an explicit, independent copy that
   * leaves the source as it is. The source can also be copied
   * to Zotero (`NavegadorZoteroDialog`, from the source only; its latest PDF goes
   * along); the copies of the open source are listed
   * with their state, and one that waits for Zotero is sent again while the
   * source is open.
   */
  import { onDestroy, onMount, untrack } from 'svelte'
  import { ActionIcon, Button, ConfirmDialog, IconButton } from '@entropia/ui'
  import { locale, t } from '$lib/i18n'
  import { navegadorStore } from '$lib/navegador-store'
  import NavegadorCopyDialog, { type CopiedTarget } from './NavegadorCopyDialog.svelte'
  import NavegadorZoteroDialog from './NavegadorZoteroDialog.svelte'
  import {
    describeCapture,
    describeSource,
    formatLocalTime,
    navegadorDeleteSource,
    navegadorListSources,
    navegadorSourceDetail,
    parseSourceError,
    sourceOpenAction,
    type SourceDetail,
    type SourceSummary,
  } from '$lib/navegador-sources'
  import {
    describeCopy,
    hasPending,
    navegadorZoteroCancel,
    navegadorZoteroLaunch,
    navegadorZoteroList,
    navegadorZoteroRequest,
    navegadorZoteroRun,
    type ZoteroCopy,
  } from '$lib/navegador-zotero'

  /**
   * `onopen` loads an address in the browser's active tab (through the URL
   * policy); `onviewpdf` opens a saved PDF capture in the app's viewer;
   * `onopenitem` opens the document a copy created; `focusSource` asks the drawer
   * to show one source's detail (a new `nonce` asks again, even for the same
   * source).
   */
  let {
    onopen,
    onclose,
    onviewpdf,
    onopenitem,
    focusSource = null,
  }: {
    onopen: (url: string) => Promise<void> | void
    onclose: () => void
    onviewpdf: (capture: { id: string; title: string }) => void
    onopenitem: (target: CopiedTarget) => void
    focusSource?: { id: string; nonce: number } | null
  } = $props()

  const currentLocale = locale
  /** How long typing must pause before the search runs. */
  const SEARCH_DELAY_MS = 250
  /** How often a copy that waits for Zotero is sent again while its source is open. */
  const ZOTERO_POLL_MS = 10_000
  const KINDS = ['page', 'selection', 'pdf']

  let query = $state('')
  let list = $state<SourceSummary[] | null>(null)
  let listError = $state<string | null>(null)
  let selectedId = $state<string | null>(null)
  let detail = $state<SourceDetail | null>(null)
  let detailState = $state<'loading' | 'ready' | 'gone' | 'error'>('loading')
  let detailError = $state<string | null>(null)
  let notice = $state<{ kind: 'ok' | 'error'; text: string } | null>(null)
  let confirming = $state<{ id: string; title: string; count: number } | null>(null)
  let deleting = $state(false)
  let deleteError = $state<string | null>(null)
  let copying = $state<{ id: string; title: string; rendered: boolean } | null>(null)
  let zoteroCopying = $state<{ source: { id: string; title: string } } | null>(null)
  let zoteroCopies = $state<ZoteroCopy[]>([])
  let zoteroNotice = $state<string | null>(null)

  let listRequest = 0
  let detailRequest = 0
  let searchTimer: ReturnType<typeof setTimeout> | undefined
  let mounted = false

  const lang = $derived(($currentLocale === 'en' ? 'en' : 'es') as 'es' | 'en')
  const rows = $derived((list ?? []).map(describeSource))
  const captures = $derived(
    (detail?.captures ?? []).map((capture) => describeCapture(capture, lang))
  )
  const openAction = $derived(sourceOpenAction(detail?.captures ?? []))
  const zoteroRows = $derived(zoteroCopies.map((copy) => ({ copy, view: describeCopy(copy) })))
  const savedCount = $derived(Object.keys($navegadorStore.saved).length)
  let seenSaved = 0
  let seenFocus = 0

  function describe(reason: unknown): string {
    return reason instanceof Error ? reason.message : String(reason)
  }

  function kindLabel(kind: string): string {
    return KINDS.includes(kind) ? t(`navegador.sources.kind.${kind}`) : kind
  }

  /** The newest search wins: an answer to an older one is dropped. */
  async function loadList() {
    const request = ++listRequest
    try {
      const result = await navegadorListSources(query)
      if (request !== listRequest) return
      list = result
      listError = null
    } catch (reason) {
      if (request !== listRequest) return
      listError = t('navegador.sources.error', { message: describe(reason) })
    }
  }

  function searchChanged() {
    clearTimeout(searchTimer)
    searchTimer = setTimeout(() => void loadList(), SEARCH_DELAY_MS)
  }

  async function loadDetail(id: string) {
    const request = ++detailRequest
    try {
      const result = await navegadorSourceDetail(id)
      if (request !== detailRequest) return
      detail = result
      detailState = result ? 'ready' : 'gone'
      if (result) void loadZoteroCopies(id, true)
    } catch (reason) {
      if (request !== detailRequest) return
      const { code, detail: message } = parseSourceError(reason)
      detailError = t('navegador.sources.detailError', { message: message ?? code })
      detailState = 'error'
    }
  }

  /**
   * The copies of one source. With `send`, a copy that waits is tried at once:
   * opening a source whose copy was queued earlier is the moment to send it.
   */
  async function loadZoteroCopies(id: string, send = false) {
    try {
      const answer = await navegadorZoteroList(id)
      if (id !== selectedId) return
      const rows = Array.isArray(answer) ? answer : []
      zoteroCopies = rows
      if (send && hasPending(rows)) await sendWaiting(id)
    } catch {
      // The list of copies is a courtesy: the source itself still shows.
    }
  }

  /** One drain, then the list again. Never starts Zotero. */
  async function sendWaiting(id: string) {
    try {
      await navegadorZoteroRun()
    } catch {
      // A failed drain leaves the copy where it was: still waiting.
    }
    if (id !== selectedId) return
    try {
      const answer = await navegadorZoteroList(id)
      if (Array.isArray(answer)) zoteroCopies = answer
    } catch {
      // Keep what is shown.
    }
  }

  async function openZotero() {
    try {
      const outcome = await navegadorZoteroLaunch()
      zoteroNotice = t(`navegador.zotero.launch.${outcome}`)
    } catch (reason) {
      zoteroNotice = describe(reason)
    }
  }

  async function cancelZoteroCopy(copy: ZoteroCopy) {
    try {
      await navegadorZoteroCancel(copy.id)
    } catch {
      // Already moving on (it started or finished): the list below says which.
    }
    if (selectedId) await loadZoteroCopies(selectedId)
  }

  async function retryZoteroCopy(copy: ZoteroCopy) {
    try {
      await navegadorZoteroRequest(copy.sourceId, copy.captureId, {
        libraryType: copy.libraryType,
        libraryId: copy.libraryId,
        libraryName: copy.libraryName,
      })
    } catch (reason) {
      zoteroNotice = describe(reason)
    }
    if (selectedId) await sendWaiting(selectedId)
  }

  function zoteroErrorText(copy: ZoteroCopy): string | null {
    if (copy.state !== 'failed') return null
    const key = `navegador.zotero.error.${copy.errorCode ?? 'unknown'}`
    const text = t(key, { message: copy.errorMessage ?? '' })
    return text === key
      ? t('navegador.zotero.error.unknown', { message: copy.errorMessage ?? '' })
      : text
  }

  // While a copy of the open source waits, send it again every few seconds.
  $effect(() => {
    const id = selectedId
    const waiting = hasPending(zoteroCopies)
    if (!id || !waiting) return
    const timer = setInterval(() => void sendWaiting(id), ZOTERO_POLL_MS)
    return () => clearInterval(timer)
  })

  function openDetail(id: string) {
    notice = null
    zoteroCopies = []
    zoteroNotice = null
    selectedId = id
    detail = null
    detailError = null
    detailState = 'loading'
    void loadDetail(id)
  }

  function backToList() {
    selectedId = null
    detail = null
    detailRequest++
    void loadList()
  }

  async function copyUrl(url: string) {
    try {
      await navigator.clipboard.writeText(url)
      notice = { kind: 'ok', text: t('navegador.sources.copied') }
    } catch {
      notice = { kind: 'error', text: t('navegador.sources.copyError') }
    }
  }

  function askToDelete(source: SourceDetail) {
    deleteError = null
    confirming = {
      id: source.id,
      title: source.title?.trim() || source.finalUrl,
      count: source.captures.length,
    }
  }

  function cancelDelete() {
    if (deleting) return
    confirming = null
    deleteError = null
  }

  async function confirmDelete() {
    const target = confirming
    if (!target || deleting) return
    deleting = true
    deleteError = null
    try {
      const outcome = await navegadorDeleteSource(target.id)
      notice = {
        kind: 'ok',
        text: t(
          outcome.leftoverFiles ? 'navegador.sources.deletedLeftover' : 'navegador.sources.deleted'
        ),
      }
      confirming = null
      backToList()
    } catch (reason) {
      const { code, detail: message } = parseSourceError(reason)
      const text = t(`navegador.sources.deleteError.${code}`, { message: message ?? '' })
      if (code === 'not_found') {
        // Somebody else (or an earlier delete) already removed it.
        notice = { kind: 'error', text }
        confirming = null
        backToList()
      } else {
        deleteError = text
      }
    } finally {
      deleting = false
    }
  }

  // Another part of the view asked for one source (a download already saved).
  $effect(() => {
    const wanted = focusSource
    untrack(() => {
      if (!wanted || wanted.nonce === seenFocus) return
      seenFocus = wanted.nonce
      openDetail(wanted.id)
    })
  })

  // A capture saved while the drawer is open shows up without reopening it.
  $effect(() => {
    const count = savedCount
    untrack(() => {
      if (!mounted || count === seenSaved) return
      seenSaved = count
      void loadList()
      if (selectedId) void loadDetail(selectedId)
    })
  })

  onMount(() => {
    mounted = true
    seenSaved = savedCount
    void loadList()
  })

  onDestroy(() => {
    mounted = false
    clearTimeout(searchTimer)
  })
</script>

<aside class="sources" aria-label={$currentLocale && t('navegador.sources.title')}>
  <header class="sources__head">
    <h2 class="sources__title">{$currentLocale && t('navegador.sources.title')}</h2>
    <IconButton
      size="sm"
      variant="ghost"
      label={$currentLocale && t('navegador.sources.close')}
      title={$currentLocale && t('navegador.sources.close')}
      onclick={onclose}
    >
      <ActionIcon name="close" size={14} />
    </IconButton>
  </header>

  {#if notice}
    <p
      class="sources__notice"
      class:sources__problem={notice.kind === 'error'}
      role="status"
      aria-live="polite"
    >
      {notice.text}
    </p>
  {/if}

  {#if selectedId === null}
    <div class="sources__search-wrap">
      <span class="search-field__icon" aria-hidden="true">
        <ActionIcon name="search" size={16} />
      </span>
      <input
        class="sources__search"
        type="search"
        autocomplete="off"
        spellcheck="false"
        aria-label={$currentLocale && t('navegador.sources.search')}
        placeholder={$currentLocale && t('navegador.sources.searchPlaceholder')}
        bind:value={query}
        oninput={searchChanged}
      />
    </div>

    {#if listError}
      <p class="sources__problem" role="alert">{listError}</p>
    {:else if list === null}
      <p class="sources__muted">{$currentLocale && t('navegador.sources.loading')}</p>
    {:else if rows.length === 0}
      <p class="sources__muted">
        {$currentLocale &&
          t(query.trim() ? 'navegador.sources.noMatches' : 'navegador.sources.empty')}
      </p>
    {:else}
      <ul class="sources__list">
        {#each rows as row (row.id)}
          <li>
            <button type="button" class="sources__row" onclick={() => openDetail(row.id)}>
              <strong class="sources__row-title">{row.title}</strong>
              <span class="sources__row-meta">
                {#if row.host}<span>{row.host}</span>{/if}
                <span
                  >{$currentLocale &&
                    t('navegador.sources.captureCount', { count: row.captureCount })}</span
                >
              </span>
              <span class="sources__row-kinds">
                {#each row.kinds as kind (kind)}
                  <span class="sources__chip">{$currentLocale && kindLabel(kind)}</span>
                {/each}
              </span>
              <span class="sources__row-meta">
                {$currentLocale &&
                  t('navegador.sources.updated', {
                    when: formatLocalTime(new Date(row.updatedAt).toISOString(), lang),
                  })}
              </span>
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  {:else}
    <div class="sources__bar">
      <Button size="sm" variant="ghost" onclick={backToList}>
        {$currentLocale && t('navegador.sources.back')}
      </Button>
    </div>

    {#if detailState === 'loading'}
      <p class="sources__muted">{$currentLocale && t('navegador.sources.loading')}</p>
    {:else if detailState === 'gone'}
      <p class="sources__problem" role="alert">{$currentLocale && t('navegador.sources.gone')}</p>
    {:else if detailState === 'error'}
      <p class="sources__problem" role="alert">{detailError}</p>
    {:else if detail}
      <h3 class="sources__detail-title">{detail.title?.trim() || detail.finalUrl}</h3>
      <div class="sources__actions">
        <Button size="sm" variant="secondary" onclick={() => void onopen(detail!.finalUrl)}>
          {$currentLocale &&
            t(
              openAction === 'origin'
                ? 'navegador.sources.openOrigin'
                : 'navegador.sources.openInBrowser'
            )}
        </Button>
        <Button size="sm" variant="secondary" onclick={() => void copyUrl(detail!.finalUrl)}>
          {$currentLocale && t('navegador.sources.copyUrl')}
        </Button>
        <Button
          size="sm"
          variant="secondary"
          onclick={() =>
            (zoteroCopying = {
              source: { id: detail!.id, title: detail!.title?.trim() || detail!.finalUrl },
            })}
        >
          {$currentLocale && t('navegador.zotero.copy')}
        </Button>
        <Button size="sm" variant="danger" onclick={() => askToDelete(detail!)}>
          {$currentLocale && t('navegador.sources.delete')}
        </Button>
      </div>
      <dl class="sources__facts">
        <dt>{$currentLocale && t('navegador.sources.detail.originalUrl')}</dt>
        <dd>{detail.originalUrl}</dd>
        <dt>{$currentLocale && t('navegador.sources.detail.finalUrl')}</dt>
        <dd>{detail.finalUrl}</dd>
        {#if detail.canonicalUrl}
          <dt>{$currentLocale && t('navegador.sources.detail.canonicalUrl')}</dt>
          <dd>{detail.canonicalUrl}</dd>
        {/if}
        <dt>{$currentLocale && t('navegador.sources.detail.firstAccessed')}</dt>
        <dd>{detail.firstAccessedAt}</dd>
      </dl>

      {#if zoteroRows.length > 0}
        <h3 class="sources__subtitle">{$currentLocale && t('navegador.zotero.section')}</h3>
        <ul class="sources__captures">
          {#each zoteroRows as { copy, view } (copy.id)}
            <li class="sources__capture">
              <div class="sources__capture-head">
                <span class="sources__chip sources__chip--{view.tone}">
                  {$currentLocale && t(view.stateKey)}
                </span>
                {#if copy.captureId}
                  <span class="sources__chip">PDF</span>
                {/if}
              </div>
              <span>
                {$currentLocale &&
                  t('navegador.zotero.library', {
                    library: view.libraryName ?? t('navegador.zotero.personal'),
                  })}
              </span>
              {#each view.notes as note (note)}
                <span class="sources__muted">{$currentLocale && t(note)}</span>
              {/each}
              {#if view.reasons.length > 0}
                <span class="sources__muted">
                  {$currentLocale &&
                    t('navegador.zotero.note.web.reason', { reason: view.reasons.join(', ') })}
                </span>
              {/if}
              {#if view.completedKeys.length > 0}
                <span class="sources__muted">
                  {$currentLocale &&
                    t('navegador.zotero.note.web.fields', {
                      fields: view.completedKeys.map((key) => t(key)).join(', '),
                    })}
                </span>
              {/if}
              {#if zoteroErrorText(copy)}
                <span class="sources__problem" role="alert">{zoteroErrorText(copy)}</span>
              {/if}
              {#if view.canLaunch || view.canCancel || view.canRetry}
                <div class="sources__actions">
                  {#if view.canLaunch}
                    <Button size="sm" variant="secondary" onclick={() => void openZotero()}>
                      {$currentLocale && t('navegador.zotero.openZotero')}
                    </Button>
                  {/if}
                  {#if view.canCancel}
                    <Button size="sm" variant="ghost" onclick={() => void cancelZoteroCopy(copy)}>
                      {$currentLocale && t('navegador.zotero.cancelCopy')}
                    </Button>
                  {/if}
                  {#if view.canRetry}
                    <Button
                      size="sm"
                      variant="secondary"
                      onclick={() => void retryZoteroCopy(copy)}
                    >
                      {$currentLocale && t('navegador.zotero.retry')}
                    </Button>
                  {/if}
                </div>
              {/if}
            </li>
          {/each}
        </ul>
        {#if zoteroNotice}
          <p class="sources__muted" role="status">{zoteroNotice}</p>
        {/if}
      {/if}

      <h3 class="sources__subtitle">{$currentLocale && t('navegador.sources.detail.captures')}</h3>
      {#if captures.length === 0}
        <p class="sources__muted">{$currentLocale && t('navegador.sources.detail.noCaptures')}</p>
      {:else}
        <ul class="sources__captures">
          {#each captures as capture (capture.id)}
            <li class="sources__capture">
              <div class="sources__capture-head">
                <span class="sources__chip">{$currentLocale && kindLabel(capture.kind)}</span>
                <span class="sources__strong">{capture.accessedLocal}</span>
              </div>
              <span class="sources__muted">
                {$currentLocale && t('navegador.sources.accessedUtc', { utc: capture.accessedUtc })}
              </span>
              <span class="sources__wrap">{capture.finalUrl}</span>
              <span>
                {$currentLocale && t(`navegador.sources.hash.${capture.hashOf}`) + ' '}<code
                  >{capture.shortSha}</code
                >
              </span>
              <span>{$currentLocale && t('navegador.sources.size', { size: capture.size })}</span>
              {#if capture.file === 'present'}
                <span>
                  {$currentLocale &&
                    t(
                      capture.kind === 'pdf'
                        ? 'navegador.sources.file.pdf'
                        : 'navegador.sources.file.page'
                    )}
                </span>
              {:else if capture.file === 'downloading'}
                <span class="sources__muted">
                  {$currentLocale && t('navegador.sources.file.downloading')}
                </span>
              {:else if capture.file === 'missing'}
                <span class="sources__problem">
                  {$currentLocale && t('navegador.sources.file.missing')}
                </span>
              {/if}
              {#if capture.canViewPdf || capture.canCopy || (capture.kind === 'pdf' && capture.file === 'present')}
                <div class="sources__actions">
                  {#if capture.canViewPdf}
                    <Button
                      size="sm"
                      variant="secondary"
                      onclick={() =>
                        onviewpdf({
                          id: capture.id,
                          title: capture.title?.trim() || detail!.title?.trim() || detail!.finalUrl,
                        })}
                    >
                      {$currentLocale && t('navegador.sources.viewPdf')}
                    </Button>
                  {/if}
                  {#if capture.canCopy}
                    <Button
                      size="sm"
                      variant="secondary"
                      onclick={() =>
                        (copying = {
                          id: capture.id,
                          title: capture.title?.trim() || detail!.title?.trim() || detail!.finalUrl,
                          // A page or a selection is copied as a PDF of its text.
                          rendered: capture.kind !== 'pdf',
                        })}
                    >
                      {$currentLocale && t('navegador.sources.copyToCollection')}
                    </Button>
                  {/if}
                </div>
              {/if}
              {#if capture.quote}
                <blockquote class="sources__quote">
                  {capture.quote.before}<mark>{capture.quote.quote}</mark>{capture.quote.after}
                </blockquote>
              {:else if capture.preview}
                <p class="sources__preview">{capture.preview}</p>
              {/if}
              {#if capture.textInFile}
                <span class="sources__muted">
                  {$currentLocale && t('navegador.sources.textInFile')}
                </span>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  {/if}
</aside>

{#if zoteroCopying}
  <NavegadorZoteroDialog
    source={zoteroCopying.source}
    onclose={() => (zoteroCopying = null)}
    onchange={() => selectedId && void loadZoteroCopies(selectedId)}
  />
{/if}

{#if copying}
  <NavegadorCopyDialog
    capture={copying}
    rendered={copying.rendered}
    onclose={() => (copying = null)}
    {onopenitem}
  />
{/if}

{#if confirming}
  <ConfirmDialog
    title={t('navegador.sources.deleteTitle')}
    message={t('navegador.sources.deleteMessage', {
      title: confirming.title,
      count: confirming.count,
    })}
    cancelLabel={t('navegador.sources.deleteCancel')}
    confirmIcon="delete"
    confirmAriaLabel={t('navegador.sources.deleteConfirm', { title: confirming.title })}
    confirmTitle={deleting ? t('navegador.sources.deleting') : undefined}
    variant="destructive"
    confirming={deleting}
    cancelDisabled={deleting}
    error={deleteError}
    confirmFirst
    oncancel={cancelDelete}
    onconfirm={() => void confirmDelete()}
  />
{/if}

<style>
  .sources {
    display: flex;
    flex: none;
    flex-direction: column;
    gap: var(--space-2);
    width: min(24rem, 45%);
    min-width: 0;
    padding: var(--space-3);
    overflow-y: auto;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-surface);
    background: var(--color-surface);
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  .sources__head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .sources__title {
    flex: 1;
    margin: 0;
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
  }

  .sources__search-wrap {
    position: relative;
  }

  .sources__search {
    width: 100%;
    min-height: var(--control-height-md);
    padding: 0 var(--space-3) 0 var(--search-field-inset);
    box-sizing: border-box;
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
    background: var(--color-surface-sunken);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
  }

  .sources__search:focus {
    outline: none;
    border-color: var(--color-accent);
    box-shadow: var(--focus-ring);
    background: var(--color-surface);
  }

  .sources__notice,
  .sources__muted {
    margin: 0;
  }

  .sources__problem {
    margin: 0;
    color: var(--color-danger, var(--color-text-primary));
  }

  .sources__list,
  .sources__captures {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
    padding: 0;
    list-style: none;
  }

  .sources__row {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    width: 100%;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
    background: var(--color-surface-sunken);
    color: inherit;
    font: inherit;
    text-align: start;
    cursor: pointer;
  }

  .sources__row:hover,
  .sources__row:focus-visible {
    border-color: var(--color-accent);
    outline: none;
  }

  .sources__row-title {
    overflow: hidden;
    color: var(--color-text-primary);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .sources__row-meta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .sources__row-kinds {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
  }

  .sources__chip {
    padding: 0 var(--space-2);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
    white-space: nowrap;
  }

  .sources__detail-title {
    margin: 0;
    overflow-wrap: anywhere;
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
  }

  .sources__chip--good {
    color: var(--color-success, var(--color-text-primary));
  }

  .sources__chip--bad {
    color: var(--color-danger, var(--color-text-primary));
  }

  .sources__chip--pending {
    color: var(--color-accent);
  }

  .sources__subtitle {
    margin: var(--space-2) 0 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-primary);
  }

  .sources__actions {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-2);
  }

  .sources__facts {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--space-1) var(--space-3);
    margin: 0;
  }

  .sources__facts dd {
    min-width: 0;
    margin: 0;
    overflow-wrap: anywhere;
    color: var(--color-text-primary);
  }

  .sources__capture {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--color-hairline);
    border-radius: var(--radius-input);
  }

  .sources__capture-head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-2);
  }

  .sources__strong {
    color: var(--color-text-primary);
  }

  .sources__wrap {
    overflow-wrap: anywhere;
  }

  .sources__quote,
  .sources__preview {
    margin: 0;
    overflow-wrap: anywhere;
    color: var(--color-text-primary);
  }

  .sources__quote {
    padding-inline-start: var(--space-3);
    border-inline-start: 2px solid var(--color-hairline);
  }
</style>
