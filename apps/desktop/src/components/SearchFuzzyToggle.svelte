<script lang="ts">
  import { Checkbox } from '@entropia/ui'
  import { t } from '$lib/i18n'

  /**
   * The "Incluir coincidencias aproximadas" switch. One shared preference for
   * every search in the app (search-preferences.ts); one component so every
   * tab shows it the same way.
   */
  interface Props {
    checked: boolean
    onchange: (checked: boolean) => void
  }

  let { checked, onchange }: Props = $props()
</script>

<span class="fuzzy-toggle">
  <Checkbox {checked} {onchange}>
    {t('writing.corpusFuzzy')}
  </Checkbox>
</span>

<style>
  .fuzzy-toggle {
    display: block;
    padding: 0 var(--space-1);
    color: var(--color-text-muted);
    font-size: var(--font-size-xs);
  }
</style>
