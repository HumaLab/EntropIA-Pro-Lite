<script lang="ts">
  /**
   * Copias de seguridad (T-47): where they go, how often they happen on their
   * own, how many are kept, and a button to make one now. The copy itself and
   * its schedule live in the backend (`backup.rs`); the folder is chosen with
   * the native picker there, never typed here.
   */
  import { invoke } from '@tauri-apps/api/core'
  import { ActionIcon, Button, Card, Input, ToolbarMenu, type ToolbarMenuItem } from '@entropia/ui'
  import { locale, t } from '$lib/i18n'
  import { settingsGet, settingsSet } from '$lib/settings'

  const DIR_KEY = 'backend_grant.backup_dir'
  const LAST_AT_KEY = 'backend_grant.backup_last_at'
  const FREQUENCY_KEY = 'backup_frequency'
  const KEEP_KEY = 'backup_keep'
  const DEFAULT_KEEP = 5

  type Frequency = 'never' | 'daily' | 'weekly'
  const FREQUENCIES: Frequency[] = ['never', 'daily', 'weekly']

  const currentLocale = locale
  let dir = $state('')
  let lastAt = $state<number | null>(null)
  let frequency = $state<Frequency>('never')
  let keep = $state(String(DEFAULT_KEEP))
  let busy = $state(false)
  let error = $state('')
  let done = $state('')

  async function load() {
    const [d, last, freq, k] = await Promise.all(
      [DIR_KEY, LAST_AT_KEY, FREQUENCY_KEY, KEEP_KEY].map((key) =>
        settingsGet(key).catch(() => null)
      )
    )
    dir = d ?? ''
    lastAt = last ? Number(last) : null
    frequency = FREQUENCIES.includes(freq as Frequency) ? (freq as Frequency) : 'never'
    keep = k ?? String(DEFAULT_KEEP)
  }

  $effect(() => {
    void load()
  })

  const frequencyLabel = (f: Frequency) => t(`settings.backup.frequency.${f}`)

  const frequencyItems = $derived<ToolbarMenuItem[]>(
    FREQUENCIES.map((id) => ({
      kind: 'radio' as const,
      id,
      label: $currentLocale && frequencyLabel(id),
      checked: frequency === id,
      onselect: () => {
        frequency = id
        void settingsSet(FREQUENCY_KEY, id)
      },
    }))
  )

  function saveKeep() {
    const n = Math.min(100, Math.max(1, Math.round(Number(keep)) || DEFAULT_KEEP))
    keep = String(n)
    void settingsSet(KEEP_KEY, keep)
  }

  async function act(action: () => Promise<void>) {
    if (busy) return
    busy = true
    error = ''
    done = ''
    try {
      await action()
    } catch (failure) {
      error = String(failure)
    } finally {
      busy = false
    }
  }

  const chooseDir = () =>
    act(async () => {
      const picked = await invoke<string | null>('backup_dir_grant')
      if (picked) dir = picked
    })

  const runNow = () =>
    act(async () => {
      done = await invoke<string>('backup_run_now')
      await load()
    })
</script>

<Card>
  <section class="backup-tab">
    <p class="backup-tab__hint">{$currentLocale && t('settings.backup.hint')}</p>

    <div class="backup-tab__field">
      <span class="backup-tab__label">{$currentLocale && t('settings.backup.dirLabel')}</span>
      <div class="backup-tab__row">
        <span class="backup-tab__value">{dir || t('settings.backup.noDir')}</span>
        <Button variant="secondary" size="sm" disabled={busy} onclick={chooseDir}>
          <ActionIcon name="folder" size={14} />
          {$currentLocale && t('settings.backup.chooseDir')}
        </Button>
      </div>
    </div>

    <div class="backup-tab__field">
      <span class="backup-tab__label" id="backup-frequency-label">
        {$currentLocale && t('settings.backup.frequencyLabel')}
      </span>
      <ToolbarMenu label={t('settings.backup.frequencyLabel')} items={frequencyItems}>
        {#snippet trigger(props, { open })}
          <button
            type="button"
            class="backup-tab__select"
            class:backup-tab__select--open={open}
            aria-labelledby="backup-frequency-label backup-frequency-value"
            {...props}
          >
            <span id="backup-frequency-value">{$currentLocale && frequencyLabel(frequency)}</span>
            <ActionIcon name="chevron-down" size={12} />
          </button>
        {/snippet}
      </ToolbarMenu>
    </div>

    <!-- Saved when the field loses focus (Input has no blur prop). -->
    <div class="backup-tab__field" onfocusout={saveKeep}>
      <Input
        bind:value={keep}
        label={$currentLocale && t('settings.backup.keepLabel')}
        hint={t('settings.backup.keepHint')}
      />
    </div>

    <div class="backup-tab__field">
      <div class="backup-tab__row">
        <span class="backup-tab__value">
          {$currentLocale && t('settings.backup.lastAt')}
          {lastAt ? new Date(lastAt).toLocaleString() : t('settings.backup.never')}
        </span>
        <Button variant="primary" size="sm" disabled={busy || !dir} loading={busy} onclick={runNow}>
          {$currentLocale && t('settings.backup.runNow')}
        </Button>
      </div>
      {#if done}
        <p class="backup-tab__done">{t('settings.backup.done')} {done}</p>
      {/if}
      {#if error}
        <p class="backup-tab__error" role="alert">{error}</p>
      {/if}
    </div>
  </section>
</Card>

<style>
  .backup-tab {
    display: flex;
    flex-direction: column;
  }

  .backup-tab__hint {
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
    padding-bottom: var(--space-3);
  }

  .backup-tab__field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    padding: var(--space-3) 0;
  }

  .backup-tab__field + .backup-tab__field {
    border-top: 1px solid var(--color-hairline);
  }

  .backup-tab__label {
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
  }

  .backup-tab__row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
  }

  .backup-tab__value {
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
    overflow-wrap: anywhere;
  }

  .backup-tab__done {
    color: var(--color-text-secondary);
    font-size: var(--font-size-xs);
    overflow-wrap: anywhere;
  }

  .backup-tab__error {
    color: var(--color-danger);
    font-size: var(--font-size-sm);
  }

  /* Same trigger as the Apariencia choices. */
  .backup-tab__select {
    display: inline-flex;
    align-self: flex-start;
    align-items: center;
    gap: var(--space-1);
    min-height: var(--control-height-sm);
    padding: 0 var(--space-2);
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-primary);
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    cursor: pointer;
  }

  .backup-tab__select:hover,
  .backup-tab__select--open {
    background: var(--surface-toolbar);
  }
</style>
