<script lang="ts">
  import { Button, Checkbox } from '@entropia/ui'
  import { locale, t, type I18nKey } from '$lib/i18n'
  import {
    GRAPH_SETTINGS_BOUNDS,
    normalizeGraphSettings,
    type GraphRenderSettings,
    type GraphSettings,
    type GraphStructureSettings,
  } from '$lib/analysis-settings'

  let { graph, onchange }: { graph: GraphSettings; onchange: (graph: GraphSettings) => void } =
    $props()

  const currentLocale = locale
  const translate = $derived.by(() => {
    $currentLocale
    return (key: I18nKey) => t(key)
  })

  type Group = 'structure' | 'render'
  type Field = keyof GraphStructureSettings | keyof GraphRenderSettings
  /**
   * How a stored value is shown: repulsion reads better as a positive force
   * than as d3's negative charge, and the cooldown in seconds than in ms.
   */
  interface NumberControl {
    group: Group
    field: Field
    label: I18nKey
    step: number
    toView?: (stored: number) => number
    fromView?: (shown: number) => number
  }
  interface ToggleControl {
    group: Group
    field: Field
    label: I18nKey
  }
  interface Section {
    title: I18nKey
    hint: I18nKey
    numbers: NumberControl[]
    toggles: ToggleControl[]
  }

  const sections: Section[] = [
    {
      title: 'graphSettings.sectionShown',
      hint: 'graphSettings.sectionShownHint',
      numbers: [
        { group: 'structure', field: 'maxEntities', label: 'graphSettings.maxEntities', step: 10 },
        {
          group: 'structure',
          field: 'maxRelations',
          label: 'graphSettings.maxRelations',
          step: 50,
        },
        { group: 'structure', field: 'minDegree', label: 'graphSettings.minDegree', step: 1 },
        {
          group: 'structure',
          field: 'minFrequency',
          label: 'graphSettings.minFrequency',
          step: 1,
        },
        {
          group: 'structure',
          field: 'minRelationWeight',
          label: 'graphSettings.minRelationWeight',
          step: 1,
        },
      ],
      toggles: [
        {
          group: 'structure',
          field: 'includeIsolated',
          label: 'graphSettings.includeIsolated',
        },
      ],
    },
    {
      title: 'graphSettings.sectionLook',
      hint: 'graphSettings.sectionLookHint',
      numbers: [
        {
          group: 'render',
          field: 'nodeSizeScale',
          label: 'graphSettings.nodeSizeScale',
          step: 0.1,
        },
        { group: 'render', field: 'linkOpacity', label: 'graphSettings.linkOpacity', step: 0.05 },
      ],
      toggles: [
        { group: 'render', field: 'showArrows', label: 'graphSettings.showArrows' },
        { group: 'render', field: 'showLinkLabels', label: 'graphSettings.showLinkLabels' },
      ],
    },
    {
      title: 'graphSettings.sectionExplore',
      hint: 'graphSettings.sectionExploreHint',
      numbers: [
        { group: 'render', field: 'linkDistance', label: 'graphSettings.linkDistance', step: 5 },
        {
          group: 'render',
          field: 'chargeStrength',
          label: 'graphSettings.repulsion',
          step: 10,
          toView: (stored) => -stored,
          fromView: (shown) => -shown,
        },
        {
          group: 'render',
          field: 'cooldownTime',
          label: 'graphSettings.cooldownSeconds',
          step: 1,
          toView: (stored) => stored / 1000,
          fromView: (shown) => shown * 1000,
        },
      ],
      toggles: [{ group: 'render', field: 'autoRotate', label: 'graphSettings.autoRotate' }],
    },
  ]

  function read(group: Group, field: Field): unknown {
    return (graph[group] as unknown as Record<string, unknown>)[field]
  }

  function bounds(control: NumberControl): { min: number; max: number } {
    const bound = (
      GRAPH_SETTINGS_BOUNDS[control.group] as Record<string, { min: number; max: number }>
    )[control.field]!
    const ends = [bound.min, bound.max].map((end) => control.toView?.(end) ?? end)
    return { min: Math.min(...ends), max: Math.max(...ends) }
  }

  function emit(group: Group, field: Field, value: number | boolean) {
    const next = normalizeGraphSettings({
      ...graph,
      [group]: { ...graph[group], [field]: value },
    })
    onchange(next)
    return next
  }

  function onNumber(control: NumberControl, event: Event) {
    const input = event.currentTarget as HTMLInputElement
    const shown = Number(input.value)
    if (input.value.trim() === '' || !Number.isFinite(shown)) {
      input.value = String(view(control))
      return
    }
    const next = emit(control.group, control.field, control.fromView?.(shown) ?? shown)
    const stored = (next[control.group] as unknown as Record<string, number>)[control.field]!
    input.value = String(control.toView?.(stored) ?? stored)
  }

  function view(control: NumberControl): number {
    const stored = read(control.group, control.field) as number
    return control.toView?.(stored) ?? stored
  }
</script>

<section class="graph-settings" aria-labelledby="graph-settings-title">
  <div class="graph-settings__head">
    <h3 id="graph-settings-title" class="graph-settings__title">
      {translate('graphSettings.title')}
    </h3>
    <Button
      variant="secondary"
      size="sm"
      onclick={() => onchange(normalizeGraphSettings(undefined))}
      >{translate('graphSettings.restoreDefaults')}</Button
    >
  </div>
  <p class="graph-settings__hint">{translate('graphSettings.hint')}</p>

  {#each sections as section (section.title)}
    <fieldset class="graph-settings__section">
      <legend class="graph-settings__legend">{translate(section.title)}</legend>
      <p class="graph-settings__hint">{translate(section.hint)}</p>
      <div class="graph-settings__grid">
        {#each section.numbers as control (control.field)}
          {@const id = `graph-setting-${control.field}`}
          {@const range = bounds(control)}
          <label class="graph-settings__field" for={id}>
            <span class="graph-settings__label">{translate(control.label)}</span>
            <input
              {id}
              class="graph-settings__input"
              type="number"
              min={range.min}
              max={range.max}
              step={control.step}
              value={view(control)}
              onchange={(event) => onNumber(control, event)}
            />
          </label>
        {/each}
      </div>
      {#each section.toggles as control (control.field)}
        <Checkbox
          class="graph-settings__toggle"
          checked={read(control.group, control.field) as boolean}
          onchange={(checked) => emit(control.group, control.field, checked)}
          >{translate(control.label)}</Checkbox
        >
      {/each}
    </fieldset>
  {/each}
</section>

<style>
  .graph-settings {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding-top: var(--space-3);
    border-top: 1px solid var(--border-subtle);
  }

  .graph-settings__head {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
  }

  .graph-settings__title {
    margin: 0;
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-bold);
    color: var(--color-text-primary);
  }

  .graph-settings__section {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin: 0;
    padding: var(--space-3);
    min-width: 0;
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-input);
  }

  .graph-settings__legend {
    padding: 0 var(--space-1);
    font-size: var(--font-size-sm);
    font-weight: var(--font-weight-bold);
    color: var(--color-text-primary);
  }

  .graph-settings__grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(11rem, 1fr));
    gap: var(--space-2) var(--space-3);
  }

  .graph-settings__field {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .graph-settings__label {
    font-size: var(--font-size-sm);
    color: var(--color-text-primary);
  }

  .graph-settings__input {
    max-width: 120px;
    padding: var(--space-2);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-input);
    background: var(--surface-input);
    color: var(--color-text-primary);
    font-size: var(--font-size-sm);
    font-family: inherit;
    font-variant-numeric: tabular-nums;
  }

  .graph-settings__input:focus-visible {
    outline: none;
    border-color: var(--border-focus);
    box-shadow: var(--focus-ring);
  }

  .graph-settings__hint {
    margin: 0;
    font-size: var(--font-size-xs);
    color: var(--color-text-muted);
  }
</style>
