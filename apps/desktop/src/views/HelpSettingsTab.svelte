<script module lang="ts">
  import {
    DEFAULT_HELP_CONTEXT_CHARS,
    DEFAULT_HELP_HISTORY_TURNS,
    DEFAULT_HELP_LOCAL_ANSWERS,
    DEFAULT_HELP_MAX_ROUNDS,
    DEFAULT_HELP_MAX_TOKENS,
    DEFAULT_HELP_MODEL,
    DEFAULT_HELP_TEMPERATURE,
  } from '$lib/settings'

  /**
   * Valores editables de la solapa «Ayuda» (S14). Los numéricos viajan como
   * texto, igual que los RAG Params: el padre valida, fija a los rangos y
   * persiste la forma canónica.
   */
  export type EditableHelpParams = {
    model: string
    contextChars: string
    maxTokens: string
    temperature: string
    historyTurns: string
    maxRounds: string
    localAnswers: boolean
  }

  /** Defaults del formulario; el modelo hereda el general vigente. */
  export function defaultHelpParams(inheritedModel: string): EditableHelpParams {
    return {
      model: inheritedModel || DEFAULT_HELP_MODEL,
      contextChars: DEFAULT_HELP_CONTEXT_CHARS,
      maxTokens: DEFAULT_HELP_MAX_TOKENS,
      temperature: DEFAULT_HELP_TEMPERATURE,
      historyTurns: DEFAULT_HELP_HISTORY_TURNS,
      maxRounds: DEFAULT_HELP_MAX_ROUNDS,
      localAnswers: DEFAULT_HELP_LOCAL_ANSWERS,
    }
  }
</script>

<script lang="ts">
  import { Button, Card, Checkbox, Input, NumberField } from '@entropia/ui'
  import { locale, t } from '$lib/i18n'

  type Props = {
    form: EditableHelpParams
    /** Modelo general vigente: lo que se usa cuando el campo queda vacío. */
    inheritedModel: string
    error?: string | null
    onrestore: () => void
  }

  let { form = $bindable(), inheritedModel, error = null, onrestore }: Props = $props()

  const currentLocale = locale
</script>

<Card>
  <section class="help-settings-tab" aria-label={$currentLocale && t('settings.help.title')}>
    <div class="help-settings-tab__copy">
      <h2>{t('settings.help.title')}</h2>
      <p>{t('settings.help.description')}</p>
    </div>

    {#if error}
      <p class="surface-message surface-message--error" role="alert">{error}</p>
    {/if}

    <div class="settings__params-grid">
      <div class="settings__field settings__field--stacked settings__param-card">
        <div class="settings__param-card-grid">
          <div class="settings__field--wide">
            <Input
              label={t('settings.help.label.model')}
              hint={t('settings.help.hint.model')}
              type="text"
              bind:value={form.model}
              placeholder={inheritedModel}
            />
          </div>
          <label class="help-settings-tab__field">
            <span class="help-settings-tab__label number-field-label"
              >{t('settings.help.label.contextChars')}</span
            >
            <NumberField
              bind:value={form.contextChars}
              min="4000"
              max="60000"
              step="1000"
              label={t('settings.help.label.contextChars')}
              decrementLabel={t('numberField.decrease')}
              incrementLabel={t('numberField.increase')}
            />
            <span class="help-settings-tab__hint">{t('settings.help.hint.contextChars')}</span>
          </label>
          <label class="help-settings-tab__field">
            <span class="help-settings-tab__label number-field-label"
              >{t('settings.help.label.maxTokens')}</span
            >
            <NumberField
              bind:value={form.maxTokens}
              min="200"
              max="8000"
              step="100"
              label={t('settings.help.label.maxTokens')}
              decrementLabel={t('numberField.decrease')}
              incrementLabel={t('numberField.increase')}
            />
            <span class="help-settings-tab__hint">{t('settings.help.hint.maxTokens')}</span>
          </label>
          <label class="help-settings-tab__field">
            <span class="help-settings-tab__label number-field-label"
              >{t('settings.help.label.temperature')}</span
            >
            <NumberField
              bind:value={form.temperature}
              min="0"
              max="1"
              step="0.1"
              label={t('settings.help.label.temperature')}
              decrementLabel={t('numberField.decrease')}
              incrementLabel={t('numberField.increase')}
            />
            <span class="help-settings-tab__hint">{t('settings.help.hint.temperature')}</span>
          </label>
          <label class="help-settings-tab__field">
            <span class="help-settings-tab__label number-field-label"
              >{t('settings.help.label.historyTurns')}</span
            >
            <NumberField
              bind:value={form.historyTurns}
              min="0"
              max="12"
              step="1"
              label={t('settings.help.label.historyTurns')}
              decrementLabel={t('numberField.decrease')}
              incrementLabel={t('numberField.increase')}
            />
            <span class="help-settings-tab__hint">{t('settings.help.hint.historyTurns')}</span>
          </label>
          <label class="help-settings-tab__field">
            <span class="help-settings-tab__label number-field-label"
              >{t('settings.help.label.maxRounds')}</span
            >
            <NumberField
              bind:value={form.maxRounds}
              min="1"
              max="20"
              step="1"
              label={t('settings.help.label.maxRounds')}
              decrementLabel={t('numberField.decrease')}
              incrementLabel={t('numberField.increase')}
            />
            <span class="help-settings-tab__hint">{t('settings.help.hint.maxRounds')}</span>
          </label>
        </div>
        <div class="help-settings-tab__switch">
          <Checkbox bind:checked={form.localAnswers} label={t('settings.help.localAnswers')}>
            <span class="help-settings-tab__copy">
              <span class="help-settings-tab__value">{t('settings.help.localAnswers')}</span>
              <span class="help-settings-tab__meta">{t('settings.help.localAnswersHint')}</span>
            </span>
          </Checkbox>
        </div>
        <p class="help-settings-tab__note">{t('settings.help.paidNote')}</p>
        <Button variant="secondary" size="sm" onclick={onrestore}
          >{t('settings.help.restoreDefaults')}</Button
        >
      </div>
    </div>
  </section>
</Card>

<style>
  .help-settings-tab {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }

  .help-settings-tab__copy {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .help-settings-tab__copy h2 {
    margin: 0;
    color: var(--color-text-primary);
    font-size: var(--font-size-lg);
    font-weight: var(--font-weight-semibold);
  }

  .help-settings-tab__copy p {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }

  .help-settings-tab__field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .help-settings-tab__label {
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-medium);
  }

  .help-settings-tab__hint {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .help-settings-tab__switch {
    display: flex;
    flex-direction: column;
  }

  .help-settings-tab__value {
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
    overflow-wrap: anywhere;
  }

  .help-settings-tab__meta {
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }

  .help-settings-tab__note {
    margin: 0;
    color: var(--color-text-secondary);
    font-size: var(--font-size-sm);
  }
</style>
