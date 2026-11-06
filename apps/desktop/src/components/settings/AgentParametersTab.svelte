<script lang="ts">
  /**
   * Solapa «Agente» de Configuración: edita todos los parámetros del motor de
   * investigación (`ParametrosAgente`).
   *
   * Los campos, el orden y los valores por defecto salen de la respuesta del
   * motor (`research_parameters_defaults`): acá no se repite ningún número ni
   * ningún rango. La validación también es del motor: el mensaje que se muestra
   * es el suyo, palabra por palabra, y lo pone `error`.
   */
  import { t } from '$lib/i18n'
  import { agentParameterText, type AgentParametersForm } from '$lib/research'
  import { Button, Card, Input } from '@entropia/ui'

  type Props = {
    /** Valores en edición; el contenedor los guarda y valida al guardar. */
    form: AgentParametersForm
    /** Respuesta del motor: qué campos hay y con qué valores por defecto. */
    defaults: Record<string, unknown>
    /** El mensaje exacto del motor cuando el último guardado no validó. */
    error?: string | null
    /** Vuelve el formulario a los defaults del motor. */
    onrestore?: () => void
  }

  let { form = $bindable(), defaults, error = null, onrestore }: Props = $props()

  /** El campo del contrato que se edita como un tope por perfil de informe. */
  const QUOTA_FIELD = 'max_consultas'

  /**
   * Las secciones y sus campos, con los nombres del contrato JSON del motor.
   * Todos los campos del contrato están acá una vez.
   */
  const SECTIONS: Array<{ title: string; fields: string[] }> = [
    {
      title: 'settings.agentParams.section.model',
      fields: ['max_tokens_salida', 'temperatura'],
    },
    {
      title: 'settings.agentParams.section.network',
      fields: ['timeout_s', 'esperas_reintento_s'],
    },
    {
      title: 'settings.agentParams.section.archive',
      fields: ['lotes_archivo_bytes', 'fragmento_archivo_bytes'],
    },
    {
      title: 'settings.agentParams.section.retrieval',
      fields: ['rerank_depth', 'leg_k', 'rrf_k', 'snippet_max', 'ventana_cita'],
    },
    {
      title: 'settings.agentParams.section.questions',
      fields: ['preguntas_minimas', QUOTA_FIELD],
    },
  ]

  const profiles = $derived(Object.keys(form.maxConsultas))

  /** El valor por defecto del motor para un campo, como se lee en el formulario. */
  function defaultText(key: string, profile?: string): string {
    if (key === QUOTA_FIELD) {
      const quotas = defaults[QUOTA_FIELD] as Record<string, unknown> | undefined
      return agentParameterText(profile ? quotas?.[profile] : undefined)
    }
    return agentParameterText(defaults[key])
  }

  function labelOf(key: string): string {
    return t(`settings.agentParams.label.${key}`)
  }

  function hintOf(key: string): string {
    return t(`settings.agentParams.hint.${key}`)
  }
</script>

<Card>
  <section class="agent-params">
    <div class="agent-params__copy">
      <h2>{t('settings.agentParams.title')}</h2>
      <p>{t('settings.agentParams.description')}</p>
    </div>

    {#if error}
      <p class="surface-message surface-message--error" role="alert">{error}</p>
    {/if}

    {#each SECTIONS as section (section.title)}
      <fieldset class="agent-params__section">
        <legend class="agent-params__legend">{t(section.title)}</legend>
        <div class="agent-params__grid">
          {#each section.fields as key (key)}
            {#if key === QUOTA_FIELD}
              <div class="agent-params__field agent-params__field--wide">
                <span class="agent-params__label">{labelOf(key)}</span>
                <p class="agent-params__hint">{hintOf(key)}</p>
                <div class="agent-params__quotas">
                  {#each profiles as profile (profile)}
                    <Input
                      label={profile}
                      hint={t('settings.agentParams.default', {
                        value: defaultText(QUOTA_FIELD, profile),
                      })}
                      type="text"
                      bind:value={form.maxConsultas[profile]}
                    />
                  {/each}
                </div>
              </div>
            {:else}
              <div class="agent-params__field">
                <Input
                  label={labelOf(key)}
                  hint={hintOf(key)}
                  type="text"
                  bind:value={form.values[key]}
                />
                <p class="agent-params__default">
                  {t('settings.agentParams.default', { value: defaultText(key) })}
                </p>
              </div>
            {/if}
          {/each}
        </div>
      </fieldset>
    {/each}

    <div class="agent-params__actions">
      <Button variant="secondary" size="sm" onclick={() => onrestore?.()}>
        {t('settings.agentParams.restoreDefaults')}
      </Button>
    </div>
  </section>
</Card>

<style>
  .agent-params {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    padding: var(--space-5);
  }

  .agent-params__copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .agent-params__copy h2 {
    margin: 0;
    font-family: var(--font-ui);
    font-size: var(--font-size-lg);
    color: var(--color-text-primary);
  }

  .agent-params__copy p {
    margin: 0;
    font-family: var(--font-ui);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
  }

  .agent-params__section {
    margin: 0;
    padding: var(--space-4) 0 0;
    border: 0;
    border-top: 1px solid var(--color-hairline);
  }

  .agent-params__legend {
    padding: 0;
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--color-text-muted);
  }

  .agent-params__grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));
    gap: var(--space-4);
    margin-top: var(--space-4);
    align-items: start;
  }

  .agent-params__field {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .agent-params__field--wide {
    grid-column: 1 / -1;
  }

  .agent-params__label {
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    font-weight: var(--font-weight-medium);
    color: var(--color-text-secondary);
  }

  .agent-params__hint {
    margin: 0;
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    color: var(--color-text-secondary);
  }

  .agent-params__default {
    margin: 0;
    font-family: var(--font-ui);
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }

  .agent-params__quotas {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: var(--space-3);
  }

  .agent-params__actions {
    display: flex;
    justify-content: flex-end;
  }
</style>
