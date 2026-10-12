<script lang="ts">
  /**
   * The Vercel AI Gateway key (T-45): when OpenRouter is down, rate-limited
   * or out of credits, text requests go there instead, with the same model.
   * Optional; without a key nothing changes. The key lives in the system
   * credential store like the others and is never shown back.
   */
  import { Button, Card, Input } from '@entropia/ui'
  import { locale, t } from '$lib/i18n'
  import { SETTINGS_KEYS, settingsDelete, settingsGet, settingsSet } from '$lib/settings'

  const currentLocale = locale
  let key = $state('')
  let hasKey = $state(false)
  let busy = $state(false)
  let error = $state('')

  $effect(() => {
    settingsGet(SETTINGS_KEYS.VERCEL_AI_GATEWAY_API_KEY)
      .then((value) => (hasKey = Boolean(value)))
      .catch(() => (hasKey = false))
  })

  async function run(action: () => Promise<void>) {
    if (busy) return
    busy = true
    error = ''
    try {
      await action()
    } catch (failure) {
      error = String(failure)
    } finally {
      busy = false
    }
  }

  const save = () =>
    run(async () => {
      await settingsSet(SETTINGS_KEYS.VERCEL_AI_GATEWAY_API_KEY, key.trim())
      key = ''
      hasKey = true
    })

  const remove = () =>
    run(async () => {
      await settingsDelete(SETTINGS_KEYS.VERCEL_AI_GATEWAY_API_KEY)
      hasKey = false
    })
</script>

<Card>
  <section class="vercel-card">
    <h2>{$currentLocale && t('settings.vercel.title')}</h2>
    <p class="vercel-card__text">{$currentLocale && t('settings.vercel.description')}</p>
    <div class="vercel-card__row">
      <Input
        type="password"
        bind:value={key}
        placeholder={hasKey ? t('settings.vercel.saved') : t('settings.vercel.placeholder')}
      />
      <Button variant="secondary" size="sm" disabled={busy || !key.trim()} onclick={save}>
        {$currentLocale && t('settings.vercel.save')}
      </Button>
      {#if hasKey}
        <Button variant="ghost" size="sm" disabled={busy} onclick={remove}>
          {$currentLocale && t('settings.vercel.remove')}
        </Button>
      {/if}
    </div>
    {#if error}
      <p class="vercel-card__error" role="alert">{error}</p>
    {/if}
  </section>
</Card>

<style>
  .vercel-card {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .vercel-card h2 {
    font-size: var(--font-size-md);
  }

  .vercel-card__text {
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .vercel-card__row {
    display: flex;
    align-items: flex-end;
    gap: var(--space-2);
  }

  .vercel-card__row :global(.input-field) {
    flex: 1;
  }

  .vercel-card__error {
    color: var(--color-danger);
    font-size: var(--font-size-sm);
  }
</style>
