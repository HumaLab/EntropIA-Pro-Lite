<script lang="ts">
  /**
   * A menu opened from a toolbar trigger: a list of commands, arbitrary
   * content, or both.
   *
   * # Why it floats in <body>
   *
   * Toolbars clip (the writing toolbar is `overflow: hidden` so it can never
   * spill out of its panel), and so do the panels around them. The menu is
   * moved to the end of <body> and positioned `fixed` from the trigger's box,
   * which is what lets it escape every clipping ancestor and keep itself inside
   * the window. The theme tokens live on <html>, so it paints the same there.
   *
   * # Keyboard
   *
   * Opening puts the focus on the first item (ArrowUp on the trigger: the
   * last). Arrows, Home and End move among the enabled items and wrap; Enter
   * and Space activate; Escape and Tab close and give the focus back to the
   * trigger. Anything carrying a menuitem role takes part, which is how free
   * content (a swatch grid, a set of options) gets the same behaviour.
   *
   * Visuals are the TopBar menus' own tokens: the zoom and language menus sit
   * on the same surface, border, radius and shadow.
   */
  import ToolbarMenuList from './ToolbarMenuList.svelte'
  import { placeMenu, type MenuPlacement } from './menu-placement'
  import type {
    ToolbarMenuCloseReason,
    ToolbarMenuEntry,
    ToolbarMenuProps,
    ToolbarMenuTriggerProps,
  } from './ToolbarMenu.types'

  let {
    label,
    open = $bindable(false),
    items = [],
    children,
    trigger,
    align = 'start',
    onclose,
  }: ToolbarMenuProps = $props()

  const uid = $props.id()
  const menuId = `${uid}-menu`

  const ITEM_SELECTOR = '[role="menuitem"], [role="menuitemcheckbox"], [role="menuitemradio"]'

  let triggerEl: HTMLSpanElement | undefined = $state()
  let menuEl: HTMLDivElement | undefined = $state()
  let placement = $state<MenuPlacement>({ top: 0, left: 0, maxHeight: 0 })
  /** Where the focus lands when the menu next opens. */
  let landing: 'first' | 'last' = 'first'

  function triggerButton(): HTMLElement | null {
    return triggerEl?.querySelector<HTMLElement>('button, [tabindex]') ?? null
  }

  function enabledItems(): HTMLElement[] {
    if (!menuEl) return []
    return [...menuEl.querySelectorAll<HTMLElement>(ITEM_SELECTOR)].filter(
      (item) => !item.hasAttribute('disabled') && item.getAttribute('aria-disabled') !== 'true'
    )
  }

  function openAt(where: 'first' | 'last') {
    landing = where
    open = true
  }

  function close(reason: ToolbarMenuCloseReason, returnFocus = false) {
    if (!open) return
    open = false
    onclose?.(reason)
    if (returnFocus) triggerButton()?.focus()
  }

  function select(item: ToolbarMenuEntry) {
    if (item.disabled) return
    close('select')
    item.onselect()
    // A command that takes the focus (an editor re-focusing itself) keeps it;
    // TipTap does so a frame later, hence the wait. One that does not would
    // leave the focus on an item that has since vanished, so it goes back to
    // the trigger.
    requestAnimationFrame(() => {
      const focused = document.activeElement
      if (!focused || focused === document.body || !focused.isConnected) triggerButton()?.focus()
    })
  }

  function place() {
    if (!menuEl || !triggerEl) return
    const box = menuEl.getBoundingClientRect()
    placement = placeMenu({
      anchor: triggerEl.getBoundingClientRect(),
      menu: { width: box.width, height: Math.max(box.height, menuEl.scrollHeight) },
      viewport: { width: window.innerWidth, height: window.innerHeight },
      align,
    })
  }

  /** Moves the menu to the end of <body>; see the header. */
  function portal(node: HTMLElement) {
    document.body.appendChild(node)
    return () => node.remove()
  }

  $effect(() => {
    if (!open || !menuEl) return
    place()
    const list = enabledItems()
    const target = landing === 'last' ? list.at(-1) : list[0]
    ;(target ?? menuEl).focus()

    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as Node | null
      if (target && (menuEl?.contains(target) || triggerEl?.contains(target))) return
      close('outside')
    }
    document.addEventListener('pointerdown', onPointerDown, true)
    window.addEventListener('resize', place)
    window.addEventListener('scroll', place, true)
    return () => {
      document.removeEventListener('pointerdown', onPointerDown, true)
      window.removeEventListener('resize', place)
      window.removeEventListener('scroll', place, true)
    }
  })

  function onMenuKeydown(event: KeyboardEvent) {
    const list = enabledItems()
    const at = list.indexOf(document.activeElement as HTMLElement)
    const focusAt = (index: number) => {
      event.preventDefault()
      list[(index + list.length) % list.length]?.focus()
    }

    switch (event.key) {
      case 'ArrowDown':
        return focusAt(at + 1)
      case 'ArrowUp':
        return focusAt(at === -1 ? -1 : at - 1)
      case 'Home':
        return focusAt(0)
      case 'End':
        return focusAt(-1)
      case 'Escape':
      case 'Tab':
        // Tab too: the menu lives at the end of <body>, so letting the focus
        // move on from it would carry it out of the application's order.
        event.preventDefault()
        event.stopPropagation()
        return close(event.key === 'Escape' ? 'escape' : 'blur', true)
      case 'Enter':
      case ' ': {
        const item = (event.target as HTMLElement).closest<HTMLElement>(ITEM_SELECTOR)
        if (!item) return
        event.preventDefault()
        item.click()
      }
    }
  }

  /**
   * Focus leaving for somewhere else closes the menu. Only a known destination
   * counts: WebKit does not focus a button on click, so a click on the trigger
   * blurs with no destination, and closing on that would reopen the menu on
   * the same click. Presses outside are the pointer handler's job.
   */
  function onMenuFocusOut(event: FocusEvent) {
    const next = event.relatedTarget as Node | null
    if (!next || menuEl?.contains(next) || triggerEl?.contains(next)) return
    close('blur')
  }

  /** A press on an item keeps the focus where it is, so no blur races the click. */
  function onMenuMouseDown(event: MouseEvent) {
    if ((event.target as HTMLElement).closest(ITEM_SELECTOR)) event.preventDefault()
  }

  const triggerProps: ToolbarMenuTriggerProps = $derived({
    'aria-haspopup': 'menu',
    'aria-expanded': open ? 'true' : 'false',
    'aria-controls': open ? menuId : undefined,
    'aria-pressed': undefined,
    onclick: () => (open ? close('toggle') : openAt('first')),
    onkeydown: (event: KeyboardEvent) => {
      if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return
      event.preventDefault()
      openAt(event.key === 'ArrowUp' ? 'last' : 'first')
    },
  })
</script>

<span class="toolbar-menu__trigger" data-menu-trigger="" bind:this={triggerEl}>
  {@render trigger(triggerProps, { open })}
</span>

{#if open}
  <div
    bind:this={menuEl}
    {@attach portal}
    id={menuId}
    class="toolbar-menu"
    role="menu"
    aria-label={label}
    tabindex="-1"
    style:position="fixed"
    style:top="{placement.top}px"
    style:left="{placement.left}px"
    style:max-height={placement.maxHeight > 0 ? `${placement.maxHeight}px` : undefined}
    onkeydown={onMenuKeydown}
    onfocusout={onMenuFocusOut}
    onmousedown={onMenuMouseDown}
  >
    <ToolbarMenuList {items} onselect={select} />
    {@render children?.({
      close: (options) => close('select', options?.returnFocus ?? true),
      select,
    })}
  </div>
{/if}

<style>
  /* Wraps the caller's button: it never flexes, and centres the button
     rather than stretching it to a tall row. */
  .toolbar-menu__trigger {
    display: inline-flex;
    align-items: center;
    flex: none;
  }

  /* Same surface as the zoom, language and typography menus in the top bar. */
  .toolbar-menu {
    z-index: 210;
    display: grid;
    gap: var(--space-1);
    min-width: 156px;
    max-width: calc(100vw - 2 * var(--space-2));
    box-sizing: border-box;
    overflow-y: auto;
    padding: var(--space-1);
    border: 1px solid var(--border-panel);
    border-radius: var(--radius-dialog);
    background: color-mix(in srgb, var(--color-surface-elevated) 96%, var(--color-bg));
    box-shadow: var(--shadow-lg);
    outline: none;
  }

  /* The entries' own styles live with them, in ToolbarMenuList.svelte. */
</style>
