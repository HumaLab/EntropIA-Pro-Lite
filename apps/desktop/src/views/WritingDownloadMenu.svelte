<script lang="ts">
  import { ActionIcon, IconButton, ToolbarMenu, tooltip, type ActionIconName } from '@entropia/ui'
  import { t, type I18nKey } from '$lib/i18n'
  import type { ExportFormat } from '$lib/export-fidelity'

  /**
   * The download button in the writing bar (plan-editor.md §17), and the three
   * formats it opens.
   *
   * # Two clicks, and nothing else
   *
   * The button asks the one question that changes from one download to the
   * next — the format — and the choice starts the export. How citations are
   * written and whether a bibliography follows were chosen in the Export tab,
   * and are not asked again here.
   *
   * # Why icons
   *
   * Three words in a row would make a menu wider than the bar it hangs from.
   * Each icon carries its name for assistive technology and the same words as
   * a tooltip, so nothing is only recognisable by its picture.
   *
   * Keyboard and dismissal are ToolbarMenu's: Escape, a click outside and a
   * choice all close it. The row adds only the left and right arrows, which a
   * horizontal row is expected to answer to.
   */

  interface Props {
    ondownload: (format: ExportFormat) => void
    /** An export is running; a second one waits for it. */
    busy?: boolean
  }

  let { ondownload, busy = false }: Props = $props()

  const FORMATS: { id: ExportFormat; label: I18nKey; icon: ActionIconName }[] = [
    { id: 'markdown', label: 'writing.downloadMarkdown', icon: 'file-markdown' },
    { id: 'html', label: 'writing.downloadHtml', icon: 'file-html' },
    { id: 'docx', label: 'writing.downloadDocx', icon: 'file-docx' },
  ]

  function onRowKeydown(event: KeyboardEvent) {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return
    const items = [
      ...(event.currentTarget as HTMLElement).querySelectorAll<HTMLElement>('[role="menuitem"]'),
    ]
    const at = items.indexOf(document.activeElement as HTMLElement)
    const step = event.key === 'ArrowRight' ? 1 : -1
    event.preventDefault()
    items[(at + step + items.length) % items.length]?.focus()
  }
</script>

<ToolbarMenu label={t('writing.downloadFormats')} align="end">
  {#snippet trigger(props, { open })}
    <IconButton
      size="sm"
      variant="ghost"
      label={busy ? t('writing.exportRunning') : t('writing.download')}
      title={busy ? t('writing.exportRunning') : t('writing.download')}
      active={open}
      disabled={busy}
      {...props}
    >
      <ActionIcon name={busy ? 'loader' : 'download'} size={14} />
    </IconButton>
  {/snippet}
  {#snippet children({ select })}
    <!-- The keys arrive from the items, bubbling; the row itself is never
         focused, as in ColorPalette's grid. -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="download-menu" onkeydown={onRowKeydown}>
      {#each FORMATS as format (format.id)}
        <button
          type="button"
          class="download-menu__item"
          role="menuitem"
          tabindex="-1"
          aria-label={t(format.label)}
          use:tooltip={t(format.label)}
          onclick={() =>
            select({
              id: format.id,
              label: t(format.label),
              onselect: () => ondownload(format.id),
            })}
        >
          <ActionIcon name={format.icon} size={20} />
        </button>
      {/each}
    </div>
  {/snippet}
</ToolbarMenu>

<style>
  /* Centred: the menu surface has a floor width wider than three icons. */
  .download-menu {
    display: flex;
    justify-content: center;
    gap: var(--space-1);
  }

  /* The menu entries' own surface (ToolbarMenuList.svelte), square. */
  .download-menu__item {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 36px;
    height: 36px;
    padding: 0;
    border: none;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--color-text-secondary);
    cursor: pointer;
    transition:
      background-color var(--transition-base),
      color var(--transition-base);
  }

  .download-menu__item:hover {
    background: var(--surface-toolbar);
    color: var(--color-text-primary);
  }

  .download-menu__item:focus-visible {
    outline: none;
    background: var(--surface-toolbar);
    color: var(--color-text-primary);
    box-shadow: var(--focus-ring);
  }
</style>
