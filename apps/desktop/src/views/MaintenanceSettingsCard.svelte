<script lang="ts">
  /**
   * Mantenimiento › "Revisar versiones de imágenes".
   *
   * Finds the images whose asset shows an older version than the one their
   * layout was computed on, and points the ones the person confirms back at
   * it. The review only reads; nothing is written before the confirmation, and
   * the backend judges every proposal again when it applies them
   * (`image_version_repair.rs`). No file is ever deleted or modified.
   */
  import { ActionIcon, Button, Card, Checkbox, ConfirmDialog } from '@entropia/ui'
  import { t } from '$lib/i18n'
  import {
    applyImageVersionRepairs,
    isStaleProposalError,
    scanImageVersionRepairs,
    type RepairCounts,
    type RepairFile,
    type RepairScan,
    type RepairScanProgress,
  } from '$lib/image-version-repair'

  // A review can propose hundreds of images: the list is drawn a page at a
  // time. The selection covers every proposal, drawn or not.
  const PAGE_SIZE = 100

  const COUNT_ORDER: (keyof RepairCounts)[] = [
    'scanned',
    'matching',
    'repairable',
    'needsLayoutRerun',
    'ambiguous',
    'noCandidate',
    'candidateAfterLayout',
    'fileMissing',
  ]

  let scan = $state<RepairScan | null>(null)
  let scanning = $state(false)
  let progress = $state<RepairScanProgress | null>(null)
  let applying = $state(false)
  let confirming = $state(false)
  let error = $state('')
  let done = $state('')
  /** Asset ids of the repairs that will be applied. */
  let selected = $state<Set<string>>(new Set())
  let completeSources = $state(false)
  let rendered = $state(PAGE_SIZE)

  const repairs = $derived(scan?.repairs ?? [])
  const completions = $derived(scan?.completions ?? [])
  const visibleRepairs = $derived(repairs.slice(0, rendered))
  const hiddenCount = $derived(Math.max(0, repairs.length - rendered))
  const selectedRepairs = $derived(repairs.filter((repair) => selected.has(repair.assetId)))
  const selectedCompletions = $derived(completeSources ? completions : [])
  const busy = $derived(scanning || applying)
  const nothingSelected = $derived(selectedRepairs.length === 0 && selectedCompletions.length === 0)

  const message = (failure: unknown) =>
    failure instanceof Error ? failure.message : String(failure)

  async function review() {
    if (busy) return
    scanning = true
    progress = null
    error = ''
    try {
      const found = await scanImageVersionRepairs((next) => (progress = next))
      scan = found
      // Everything proposed is selected: the person unticks what to leave.
      selected = new Set(found.repairs.map((repair) => repair.assetId))
      completeSources = false
      rendered = PAGE_SIZE
    } catch (failure) {
      scan = null
      error = t('settings.maintenance.scanError', { error: message(failure) })
    } finally {
      scanning = false
      progress = null
    }
  }

  function startReview() {
    done = ''
    void review()
  }

  function toggle(assetId: string, checked: boolean) {
    const next = new Set(selected)
    if (checked) next.add(assetId)
    else next.delete(assetId)
    selected = next
  }

  async function apply() {
    confirming = false
    if (busy || nothingSelected) return
    applying = true
    error = ''
    done = ''
    try {
      const applied = await applyImageVersionRepairs(selectedRepairs, selectedCompletions)
      done = t('settings.maintenance.done', {
        repaired: applied?.repaired.length ?? 0,
        completed: applied?.completed ?? 0,
      })
    } catch (failure) {
      error = isStaleProposalError(failure)
        ? t('settings.maintenance.stale')
        : t('settings.maintenance.applyError', { error: message(failure) })
      return
    } finally {
      applying = false
    }
    // What is left to repair is whatever a new review finds.
    await review()
  }

  function describe(file: RepairFile) {
    const label =
      file.version === 0
        ? t('item.imageVersionOriginal')
        : t('item.imageVersionNumber', { n: file.version })
    return file.width !== null && file.height !== null
      ? `${label} (${file.width}×${file.height})`
      : label
  }
</script>

<Card>
  <section class="maintenance">
    <h2 class="maintenance__title">{t('settings.maintenance.versionsTitle')}</h2>
    <p class="maintenance__hint">{t('settings.maintenance.versionsHint')}</p>

    <div class="maintenance__row">
      <Button
        variant="secondary"
        size="sm"
        disabled={busy}
        loading={scanning}
        onclick={startReview}
      >
        <ActionIcon name="search" size={14} />
        {t('settings.maintenance.scan')}
      </Button>
      {#if scanning}
        <span class="maintenance__status" role="status">
          {progress
            ? t('settings.maintenance.scanProgress', { ...progress })
            : t('settings.maintenance.scanning')}
        </span>
      {/if}
    </div>

    {#if error}
      <p class="maintenance__error" role="alert">{error}</p>
    {/if}
    {#if done}
      <p class="maintenance__status" role="status">{done}</p>
    {/if}

    {#if scan}
      <div class="maintenance__block">
        <h3 class="maintenance__subtitle">{t('settings.maintenance.summaryTitle')}</h3>
        <dl class="maintenance__counts" data-testid="maintenance-summary">
          {#each COUNT_ORDER as reason (reason)}
            <div class="maintenance__count">
              <dt>{t(`settings.maintenance.count.${reason}`)}</dt>
              <dd>{scan.counts[reason]}</dd>
            </div>
          {/each}
        </dl>
      </div>

      <div class="maintenance__block">
        {#if repairs.length === 0}
          <p class="maintenance__hint">{t('settings.maintenance.repairsEmpty')}</p>
        {:else}
          <div class="maintenance__row">
            <h3 class="maintenance__subtitle">
              {t('settings.maintenance.repairsTitle', { count: repairs.length })}
            </h3>
            <span class="maintenance__status">
              {t('settings.maintenance.selected', { count: selectedRepairs.length })}
            </span>
            <Button
              variant="ghost"
              size="sm"
              disabled={busy}
              onclick={() => (selected = new Set(repairs.map((repair) => repair.assetId)))}
            >
              {t('settings.maintenance.selectAll')}
            </Button>
            <Button
              variant="ghost"
              size="sm"
              disabled={busy}
              onclick={() => (selected = new Set())}
            >
              {t('settings.maintenance.selectNone')}
            </Button>
          </div>

          <ul class="maintenance__list">
            {#each visibleRepairs as repair (repair.assetId)}
              <li data-testid="maintenance-repair">
                <Checkbox
                  checked={selected.has(repair.assetId)}
                  disabled={busy}
                  onchange={(checked) => toggle(repair.assetId, checked)}
                >
                  <span class="maintenance__name">{repair.name}</span>
                  <span class="maintenance__change">
                    {t('settings.maintenance.repairChange', {
                      from: describe(repair.shown),
                      to: describe(repair.proposed),
                    })}
                  </span>
                </Checkbox>
              </li>
            {/each}
          </ul>
          {#if hiddenCount > 0}
            <Button variant="ghost" size="sm" onclick={() => (rendered += PAGE_SIZE)}>
              {t('settings.maintenance.showMore', { count: Math.min(PAGE_SIZE, hiddenCount) })}
            </Button>
          {/if}
        {/if}
      </div>

      {#if completions.length > 0}
        <div class="maintenance__block">
          <Checkbox bind:checked={completeSources} disabled={busy}>
            {t(
              completions.length === 1
                ? 'settings.maintenance.completions.one'
                : 'settings.maintenance.completions',
              { count: completions.length }
            )}
          </Checkbox>
          <p class="maintenance__hint">{t('settings.maintenance.completionsHint')}</p>
        </div>
      {/if}

      <div class="maintenance__row">
        <Button
          variant="primary"
          size="sm"
          disabled={busy || nothingSelected}
          loading={applying}
          onclick={() => (confirming = true)}
        >
          {t('settings.maintenance.apply')}
        </Button>
      </div>
    {/if}
  </section>
</Card>

{#if confirming}
  <ConfirmDialog
    title={t('settings.maintenance.confirmTitle')}
    titleId="maintenance-versions-confirm-title"
    cancelLabel={t('settings.maintenance.confirmCancel')}
    confirmLabel={t('settings.maintenance.apply')}
    oncancel={() => (confirming = false)}
    onconfirm={apply}
  >
    {#if selectedRepairs.length > 0}
      <p>
        {t(
          selectedRepairs.length === 1
            ? 'settings.maintenance.confirmRepairs.one'
            : 'settings.maintenance.confirmRepairs',
          { count: selectedRepairs.length }
        )}
      </p>
    {/if}
    {#if selectedCompletions.length > 0}
      <p>
        {t(
          selectedCompletions.length === 1
            ? 'settings.maintenance.confirmCompletions.one'
            : 'settings.maintenance.confirmCompletions',
          { count: selectedCompletions.length }
        )}
      </p>
    {/if}
    <p>{t('settings.maintenance.confirmNoDelete')}</p>
  </ConfirmDialog>
{/if}

<style>
  .maintenance {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .maintenance__title {
    margin: 0;
    color: var(--color-text-primary);
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-medium);
  }

  .maintenance__subtitle {
    margin: 0;
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
  }

  .maintenance__hint {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
    line-height: 1.4;
  }

  .maintenance__row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--space-3);
  }

  .maintenance__status {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
    font-variant-numeric: tabular-nums;
  }

  .maintenance__error {
    margin: 0;
    color: var(--color-danger);
    font-size: var(--font-size-sm);
    overflow-wrap: anywhere;
  }

  .maintenance__block {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding-top: var(--space-3);
    border-top: 1px solid var(--color-hairline);
  }

  .maintenance__counts {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    margin: 0;
  }

  .maintenance__count {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-3);
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .maintenance__count dd {
    margin: 0;
    color: var(--color-text-primary);
    font-variant-numeric: tabular-nums;
  }

  .maintenance__list {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    max-height: 24rem;
    margin: 0;
    padding: 0;
    overflow-y: auto;
    list-style: none;
  }

  .maintenance__name {
    display: block;
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    overflow-wrap: anywhere;
  }

  .maintenance__change {
    display: block;
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
    font-variant-numeric: tabular-nums;
  }
</style>
