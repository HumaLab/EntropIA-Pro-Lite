<script lang="ts">
  /**
   * "Copiar a Zotero" for a saved web source, or for one of its PDF captures
   * (the page is created in Zotero with that PDF attached).
   *
   * The person picks the library (their own is preselected); nothing is moved
   * and the saved source stays as it is. The copy is a durable row on the Rust
   * side: when Zotero is not running it waits in a queue, and the drawer keeps
   * showing it and sends it when Zotero answers. An item that is already in the
   * library is linked, never duplicated.
   *
   * The libraries are listed live from Zotero (the archive's known libraries
   * only when Zotero does not answer, and the dialog says so). When a library is
   * chosen the dialog checks whether the source is already in it; if so it says
   * so up front and offers "Abrir en Zotero" instead of copying.
   *
   * The work is `$lib/navegador-zotero`; this component holds the choice and
   * the result. Rust finds the file and builds the item from its own rows.
   */
  import { onMount } from 'svelte'
  import { ConfirmDialog } from '@entropia/ui'
  import { locale, t } from '$lib/i18n'
  import { libraryLabel, loadLibraries } from '$lib/writing-zotero-libraries'
  import {
    describeCopy,
    describeStatus,
    navegadorZoteroLibraries,
    navegadorZoteroOpenItem,
    navegadorZoteroStatus,
    type CopyStatus,
    navegadorZoteroRequest,
    navegadorZoteroRun,
    parseZoteroError,
    type ZoteroCopy,
  } from '$lib/navegador-zotero'

  let {
    source,
    capture = null,
    onclose,
    onchange,
  }: {
    source: { id: string; title: string }
    /** A saved PDF that goes along with the page. */
    capture?: { id: string; title: string } | null
    onclose: () => void
    /** The queue changed: the caller refreshes what it shows. */
    onchange?: () => void
  } = $props()

  const currentLocale = locale

  type Option = { libraryType: 'user' | 'group'; libraryId: string; name: string | null }

  let libraries = $state<Option[]>([])
  let fromKnown = $state(false)
  let status = $state<CopyStatus | null>(null)
  let checking = $state(false)
  let statusRequest = 0
  let loading = $state(true)
  let choice = $state('user:0')
  let phase = $state<'choosing' | 'sending' | 'done'>('choosing')
  let problem = $state<string | null>(null)
  let result = $state<ZoteroCopy | null>(null)

  const keyOf = (option: Pick<Option, 'libraryType' | 'libraryId'>) =>
    `${option.libraryType}:${option.libraryId}`
  const label = (option: Option) =>
    libraryLabel(
      { ...option, source: 'catalog', unverified: false },
      t('navegador.zotero.personal')
    )
  const chosen = $derived(libraries.find((option) => keyOf(option) === choice) ?? null)
  const chosenLabel = $derived(chosen ? label(chosen) : '')

  onMount(async () => {
    try {
      let live: Option[] = []
      try {
        const answer = await navegadorZoteroLibraries()
        if (answer?.reachable) live = answer.libraries
      } catch {
        // Treated like a closed Zotero: the known libraries are offered instead.
      }
      if (live.length > 0) {
        libraries = live
      } else {
        fromKnown = true
        libraries = (await loadLibraries()).map((option) => ({
          libraryType: option.libraryType,
          libraryId: option.libraryId,
          name: option.name ?? null,
        }))
      }
    } finally {
      loading = false
    }
  })

  // Whenever the chosen library changes, ask whether the source is already in it.
  $effect(() => {
    const library = chosen
    if (!library || phase !== 'choosing') return
    const request = ++statusRequest
    status = null
    checking = true
    navegadorZoteroStatus(source.id, capture?.id ?? null, {
      libraryType: library.libraryType,
      libraryId: library.libraryId,
      libraryName: library.name,
    })
      .then((answer) => {
        if (request === statusRequest) status = answer
      })
      .catch(() => {
        // A failed check never blocks copying: the run reports its own problem.
        if (request === statusRequest) status = null
      })
      .finally(() => {
        if (request === statusRequest) checking = false
      })
  })

  const present = $derived(status ? describeStatus(status) : null)
  const isPresent = $derived(present?.present === true)
  /* The item is there but lacks something the Web API key can fill in: the main
     action completes it instead of only opening it. */
  const completable = $derived(isPresent && present?.canComplete === true)

  function describeFailure(copy: ZoteroCopy): string {
    const code = copy.errorCode ?? 'unknown'
    const key = `navegador.zotero.error.${code}`
    const text = t(key, { message: copy.errorMessage ?? '' })
    return text === key
      ? t('navegador.zotero.error.unknown', { message: copy.errorMessage ?? code })
      : text
  }

  async function copy() {
    if (!chosen) return
    phase = 'sending'
    problem = null
    let queued: ZoteroCopy
    try {
      queued = await navegadorZoteroRequest(source.id, capture?.id ?? null, {
        libraryType: chosen.libraryType,
        libraryId: chosen.libraryId,
        libraryName: chosen.name ?? null,
      })
    } catch (reason) {
      const { code, detail } = parseZoteroError(reason)
      problem = t(`navegador.zotero.error.${code}`, { message: detail ?? '' })
      phase = 'choosing'
      return
    }
    result = queued
    try {
      const drained = await navegadorZoteroRun()
      result = drained.copies.find((row) => row.id === queued.id) ?? queued
    } catch {
      // The copy is queued whatever happened to the run: it reads as waiting.
    }
    phase = 'done'
    onchange?.()
  }

  const summary = $derived.by(() => {
    if (!result) return ''
    const library = chosenLabel
    switch (result.state) {
      case 'copied':
        return t('navegador.zotero.done.copied', { library })
      case 'linked':
        return t('navegador.zotero.done.linked', { library })
      case 'failed':
        return t('navegador.zotero.done.failed', { message: describeFailure(result) })
      case 'cancelled':
        return t('navegador.zotero.done.cancelled', { library })
      default:
        return t('navegador.zotero.done.waiting', { library })
    }
  })
  const notes = $derived(result ? describeCopy(result).notes : [])
  const completedKeys = $derived(result ? describeCopy(result).completedKeys : [])
  const reasons = $derived(result ? describeCopy(result).reasons : [])

  async function openInZotero() {
    if (!chosen || !status?.itemKey) return
    try {
      await navegadorZoteroOpenItem(chosen.libraryType, chosen.libraryId, status.itemKey)
      onclose()
    } catch {
      problem = t('navegador.zotero.openFailed')
    }
  }

  function confirm() {
    if (phase === 'done') {
      onclose()
      return
    }
    if (phase === 'choosing' && isPresent && !completable) {
      void openInZotero()
      return
    }
    if (phase === 'choosing' && chosen) void copy()
  }

  const confirmLabel = $derived(
    $currentLocale &&
      t(
        phase === 'done'
          ? 'navegador.zotero.ok'
          : completable
            ? 'navegador.zotero.complete'
            : isPresent
              ? 'navegador.zotero.open'
              : 'navegador.zotero.confirm'
      )
  )
</script>

<ConfirmDialog
  title={$currentLocale && t('navegador.zotero.title')}
  cancelLabel={$currentLocale &&
    t(phase === 'done' ? 'navegador.zotero.close' : 'navegador.zotero.cancel')}
  cancelDisabled={phase === 'sending'}
  dismissOnOverlay={false}
  {confirmLabel}
  confirmDisabled={phase === 'choosing' ? loading || !chosen || checking : false}
  confirming={phase === 'sending'}
  error={problem}
  oncancel={() => phase !== 'sending' && onclose()}
  onconfirm={confirm}
>
  {#if phase === 'sending'}
    <p class="zotero-dialog__status" role="status" aria-live="polite">
      {$currentLocale && t('navegador.zotero.sending')}
    </p>
  {:else if phase === 'done' && result}
    <p
      class="zotero-dialog__status"
      role={result.state === 'failed' ? 'alert' : 'status'}
      aria-live="polite"
    >
      {$currentLocale && summary}
    </p>
    {#each notes as note (note)}
      <p class="zotero-dialog__note">{$currentLocale && t(note)}</p>
    {/each}
    {#if reasons.length > 0}
      <p class="zotero-dialog__note">
        {$currentLocale && t('navegador.zotero.note.web.reason', { reason: reasons.join(', ') })}
      </p>
    {/if}
    {#if completedKeys.length > 0}
      <p class="zotero-dialog__note">
        {$currentLocale &&
          t('navegador.zotero.note.web.fields', {
            fields: completedKeys.map((key) => t(key)).join(', '),
          })}
      </p>
    {/if}
  {:else}
    <p class="zotero-dialog__intro">
      {$currentLocale &&
        t(capture ? 'navegador.zotero.introPdf' : 'navegador.zotero.intro', {
          title: capture?.title ?? source.title,
        })}
    </p>
    <fieldset class="zotero-dialog__destination">
      <legend class="zotero-dialog__legend">
        {$currentLocale && t('navegador.zotero.destination')}
      </legend>
      {#if loading}
        <p class="zotero-dialog__status">{$currentLocale && t('navegador.zotero.loading')}</p>
      {:else}
        <div class="zotero-dialog__options">
          {#each libraries as option (keyOf(option))}
            <label class="zotero-dialog__option">
              <input
                type="radio"
                name="zotero-destination"
                value={keyOf(option)}
                bind:group={choice}
              />
              <span class="zotero-dialog__option-name">{$currentLocale && label(option)}</span>
            </label>
          {/each}
        </div>
      {/if}
    </fieldset>
    {#if fromKnown && !loading}
      <p class="zotero-dialog__note" role="status">
        {$currentLocale && t('navegador.zotero.fallback')}
      </p>
    {/if}
    {#if checking}
      <p class="zotero-dialog__note" role="status">
        {$currentLocale && t('navegador.zotero.checking')}
      </p>
    {:else if status && present && present.pdfCapture !== undefined}
      <p class="zotero-dialog__note">
        {$currentLocale &&
          (present.pdfCapture
            ? t('navegador.zotero.pdf.goes', { date: present.pdfCapture.savedAt.slice(0, 10) })
            : t('navegador.zotero.pdf.none'))}
      </p>
    {/if}
    {#if checking}
      <!-- the checking note above already speaks -->
    {:else if present?.present && status}
      <p class="zotero-dialog__status" role="status">
        {$currentLocale && t('navegador.zotero.present', { library: chosenLabel })}
      </p>
      {#if present.fromRecord}
        <p class="zotero-dialog__note">{$currentLocale && t('navegador.zotero.present.record')}</p>
      {/if}
      {#if present.pdfKey}
        <p class="zotero-dialog__note">{$currentLocale && t(present.pdfKey)}</p>
      {/if}
      {#if present.pendingKeys.length > 0}
        <p class="zotero-dialog__note">
          {$currentLocale &&
            t('navegador.zotero.present.pending', {
              fields: present.pendingKeys.map((key) => t(key)).join(', '),
            })}
        </p>
      {/if}
      {#if completable}
        <p class="zotero-dialog__note">
          {$currentLocale && t('navegador.zotero.present.completable')}
        </p>
      {/if}
      {#if present.keptKeys.length > 0}
        <p class="zotero-dialog__note">
          {$currentLocale &&
            t('navegador.zotero.present.kept', {
              fields: present.keptKeys.map((key) => t(key)).join(', '),
            })}
        </p>
      {/if}
    {:else}
      <p class="zotero-dialog__note">{$currentLocale && t('navegador.zotero.hint')}</p>
    {/if}
  {/if}
</ConfirmDialog>

<style>
  .zotero-dialog__intro,
  .zotero-dialog__status,
  .zotero-dialog__note {
    margin: 0 0 var(--space-2);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
    overflow-wrap: anywhere;
  }

  .zotero-dialog__note {
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .zotero-dialog__destination {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0 0 var(--space-2);
    padding: 0;
    border: none;
  }

  .zotero-dialog__legend {
    padding: 0;
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.075em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }

  .zotero-dialog__options {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-height: 240px;
    overflow-y: auto;
  }

  .zotero-dialog__option {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2);
    border-radius: var(--radius-control);
    cursor: pointer;
  }

  .zotero-dialog__option:hover {
    background: var(--color-accent-faint);
  }

  .zotero-dialog__option-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
  }
</style>
