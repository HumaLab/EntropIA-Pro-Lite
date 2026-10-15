<script lang="ts">
  /**
   * The Vercel AI Gateway key (T-45): when OpenRouter is down, rate-limited
   * or out of credits, text requests go there instead, with the same model.
   * Optional; without a key nothing changes. The key lives in the system
   * credential store like the others and is never shown back.
   */
  import { ActionIcon, Button, Card, Input } from '@entropia/ui'
  import { openExternalUrlFromClick } from '$lib/external-links'
  import { locale, t } from '$lib/i18n'
  import {
    SETTINGS_KEYS,
    settingsDelete,
    settingsGet,
    settingsSet,
    testVercelConnection,
  } from '$lib/settings'

  const currentLocale = locale
  // Vercel's own "go to my team's page" link: lands on AI Gateway → API Keys.
  const KEYS_URL = 'https://vercel.com/d?to=%2F%5Bteam%5D%2F%7E%2Fai%2Fapi-keys'
  let key = $state('')
  let hasKey = $state(false)
  let busy = $state(false)
  let error = $state('')
  let ok = $state('')

  $effect(() => {
    settingsGet(SETTINGS_KEYS.VERCEL_AI_GATEWAY_API_KEY)
      .then((value) => (hasKey = Boolean(value)))
      .catch(() => (hasKey = false))
  })

  async function run(action: () => Promise<void>) {
    if (busy) return
    busy = true
    error = ''
    ok = ''
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

  const test = () =>
    run(async () => {
      try {
        await testVercelConnection()
        ok = t('settings.vercel.ok')
      } catch (failure) {
        // Vercel's 403 for an account without a card is a JSON blob; say it plainly.
        throw String(failure).includes('customer_verification_required')
          ? t('settings.vercel.needsCard')
          : failure
      }
    })

  const remove = () =>
    run(async () => {
      await settingsDelete(SETTINGS_KEYS.VERCEL_AI_GATEWAY_API_KEY)
      hasKey = false
    })
</script>

<Card>
  <section class="vercel-card">
    <div class="vercel-card__head">
      <h2>{$currentLocale && t('settings.vercel.title')}</h2>
      <a
        class="vercel-card__link"
        href={KEYS_URL}
        onclick={(event) =>
          openExternalUrlFromClick(event, KEYS_URL).catch((failure: unknown) =>
            console.error('[Settings] No se pudo abrir el enlace de Vercel', failure)
          )}
        aria-label={t('settings.getApiKeyLink', { provider: 'Vercel' })}
      >
        <span>{t('settings.getApiKeyShort')}</span>
        <ActionIcon name="external-link" size={14} />
      </a>
    </div>
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
        <Button variant="secondary" size="sm" disabled={busy} onclick={test}>
          {$currentLocale &&
            (busy ? t('settings.testingConnection') : t('settings.testConnection'))}
        </Button>
        <Button variant="ghost" size="sm" disabled={busy} onclick={remove}>
          {$currentLocale && t('settings.vercel.remove')}
        </Button>
      {/if}
    </div>
    {#if ok}
      <p class="vercel-card__ok" role="status">{ok}</p>
    {/if}
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

  .vercel-card__head {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .vercel-card__link {
    display: inline-flex;
    align-items: center;
    gap: var(--space-1);
    color: var(--color-accent);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-semibold);
    text-decoration: none;
  }

  .vercel-card__link:hover {
    text-decoration: underline;
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

  .vercel-card__ok {
    color: var(--color-success);
    font-size: var(--font-size-sm);
  }

  .vercel-card__error {
    color: var(--color-danger);
    font-size: var(--font-size-sm);
  }
</style>
