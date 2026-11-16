<script lang="ts">
  import { onMount } from 'svelte'
  import { locale, t } from '$lib/i18n'
  import {
    clampBubbleOffsets,
    clearStoredBubblePosition,
    defaultBubbleOffsetsFromAnchor,
    exceedsDragThreshold,
    fallbackBubbleOffsets,
    HELP_BUBBLE_ANCHOR_RETRIES,
    HELP_BUBBLE_ANCHOR_SELECTOR,
    HELP_BUBBLE_SIZE_PX,
    loadBubbleMaximized,
    offsetsToPosition,
    positionToOffsets,
    saveBubbleMaximized,
    type BubbleAnchor,
    type BubbleOffsets,
    type BubbleViewport,
  } from '$lib/help-bubble'
  import HelpView from '../views/HelpView.svelte'
  import { ActionIcon, IconButton, portal } from '@entropia/ui'
  import entropiaMark from '../assets/entropia-mark.png'

  const currentLocale = locale
  const PANEL_ID = 'help-bubble-panel'
  const PANEL_GAP_PX = 12
  /** Maximized panel air: a small margin around the window… */
  const PANEL_MAXIMIZED_MARGIN_PX = 16
  /** …with the status-bar clearance below, like the bubble's own default. */
  const PANEL_MAXIMIZED_BOTTOM_PX = 46

  function readStorage(): Pick<Storage, 'getItem' | 'setItem' | 'removeItem'> | null {
    try {
      return globalThis.localStorage ?? null
    } catch {
      return null
    }
  }

  function readViewport(): BubbleViewport {
    if (typeof window === 'undefined') return { width: 1024, height: 768 }
    return { width: window.innerWidth, height: window.innerHeight }
  }

  function initialOffsets(viewport: BubbleViewport): BubbleOffsets {
    // T16: every launch starts at the default, whatever
    // was stored before. Clear the stale position once so old values can
    // never come back; a drag lasts for the session only.
    clearStoredBubblePosition(readStorage())
    return defaultOffsetsFor(viewport)
  }

  /**
   * Reads the status-bar anchor (T17): its horizontal centre plus the top
   * of its bar (the closest footer/.statusbar). Null when the anchor is
   * not in the DOM yet or has no laid-out box (mount order, hidden shell).
   */
  function measureAnchor(): BubbleAnchor | null {
    if (typeof document === 'undefined') return null
    const anchor = document.querySelector(HELP_BUBBLE_ANCHOR_SELECTOR)
    if (!(anchor instanceof Element)) return null
    const rect = anchor.getBoundingClientRect()
    if (rect.width <= 0 || rect.height <= 0) return null
    const bar = anchor.closest('footer, .statusbar')
    const barTop = bar?.getBoundingClientRect().top ?? rect.top
    if (!Number.isFinite(barTop)) return null
    return { centerX: rect.left + rect.width / 2, barTop }
  }

  function defaultOffsetsFor(viewport: BubbleViewport): BubbleOffsets {
    // T17: the default is centred on the GitHub anchor, just above the
    // status bar; without an anchor (or before it mounts) the bubble
    // waits window-centred, just above the bottom.
    const anchor = measureAnchor()
    if (anchor) return defaultBubbleOffsetsFromAnchor(anchor, viewport)
    return fallbackBubbleOffsets(viewport)
  }

  // Snapshot once: the live viewport below re-reads the window on resize,
  // but the initial offsets need the same values the first paint uses.
  const initialViewport = readViewport()
  let viewport = $state<BubbleViewport>(initialViewport)
  // T17: the anchor owns the default only while the user has not dragged
  // in this session; after a drag the offsets below are the bubble's
  // distance to the window's right/bottom edges (T14 math).
  let userDragged = $state(false)
  // Anchored to the window's right/bottom edges (T14): the derived position
  // below follows the corner across resizes and maximize/restore once the
  // user drags, and the anchor default while they have not (T17).
  let offsets = $state<BubbleOffsets>(initialOffsets(initialViewport))
  let maximized = $state<boolean>(loadBubbleMaximized(readStorage()))
  let open = $state(false)
  // The chat mounts on first open and stays alive afterwards: opening the
  // panel never costs a backend call while it stays shut (so shell tests
  // rendering AppShell never see the help bridge), and the conversation
  // survives closing the panel (D4: session memory). Maximizing never
  // remounts it either, so a typed draft survives the toggle.
  let dragging = $state(false)
  let pressStart: { x: number; y: number; position: { x: number; y: number } } | null = null
  let suppressClick = false
  let bubbleButton: HTMLButtonElement | undefined = $state()

  const position = $derived(offsetsToPosition(offsets, viewport))

  // The panel grows toward the inside of the window from wherever the bubble
  // is, so it never hangs off-screen (S13). Maximized it fills the window
  // instead, independent of the bubble position (T14).
  const panelSide = $derived(
    position.x + HELP_BUBBLE_SIZE_PX / 2 > viewport.width / 2 ? 'left' : 'right'
  )
  const panelVertical = $derived(
    position.y + HELP_BUBBLE_SIZE_PX / 2 > viewport.height / 2 ? 'above' : 'below'
  )
  const panelStyle = $derived.by(() => {
    if (maximized) {
      return (
        `left: ${PANEL_MAXIMIZED_MARGIN_PX}px; ` +
        `right: ${PANEL_MAXIMIZED_MARGIN_PX}px; ` +
        `top: ${PANEL_MAXIMIZED_MARGIN_PX}px; ` +
        `bottom: ${PANEL_MAXIMIZED_BOTTOM_PX}px`
      )
    }
    const parts: string[] = []
    if (panelSide === 'left') {
      parts.push(`right: ${viewport.width - position.x + PANEL_GAP_PX}px`)
    } else {
      parts.push(`left: ${position.x + HELP_BUBBLE_SIZE_PX + PANEL_GAP_PX}px`)
    }
    if (panelVertical === 'above') {
      parts.push(`bottom: ${viewport.height - position.y + PANEL_GAP_PX}px`)
    } else {
      parts.push(`top: ${position.y + HELP_BUBBLE_SIZE_PX + PANEL_GAP_PX}px`)
    }
    return parts.join('; ')
  })

  function takeCapture(node: HTMLElement, pointerId: number): void {
    try {
      node.setPointerCapture?.(pointerId)
    } catch {
      // Inactive pointer or no implementation: window-less button handlers
      // still track the drag, capture is only the robust path.
    }
  }

  function releaseCapture(node: HTMLElement, pointerId: number): void {
    try {
      if (node.hasPointerCapture?.(pointerId)) node.releasePointerCapture(pointerId)
    } catch {
      // Best effort: releasing a capture that is already gone is harmless.
    }
  }

  function handlePointerDown(event: PointerEvent): void {
    if (event.pointerType === 'mouse' && event.button !== 0) return
    pressStart = { x: event.clientX, y: event.clientY, position }
    dragging = false
    takeCapture(event.currentTarget as HTMLElement, event.pointerId)
  }

  function handlePointerMove(event: PointerEvent): void {
    if (!pressStart) return
    if (!dragging && !exceedsDragThreshold(pressStart, { x: event.clientX, y: event.clientY })) {
      return
    }
    dragging = true
    // Session-only (T16): from here on the anchor no longer owns the
    // position; resizes keep this dragged distance to the corner (T14).
    userDragged = true
    const moved = clampBubbleOffsets(
      positionToOffsets(
        {
          x: pressStart.position.x + (event.clientX - pressStart.x),
          y: pressStart.position.y + (event.clientY - pressStart.y),
        },
        viewport
      ),
      viewport
    )
    // Keep the Svelte 5 state object reference stable for the linter: assign
    // fields rather than replacing the whole object.
    offsets.right = moved.right
    offsets.bottom = moved.bottom
  }

  function endPress(event: PointerEvent): void {
    if (!pressStart) return
    pressStart = null
    releaseCapture(event.currentTarget as HTMLElement, event.pointerId)
    if (dragging) {
      // Session-only (T16): the drag stays in memory, nothing is stored.
      // Browsers fire a click after a drag: it must not toggle the panel.
      suppressClick = true
    }
    dragging = false
  }

  function handlePointerUp(event: PointerEvent): void {
    endPress(event)
  }

  function handlePointerCancel(event: PointerEvent): void {
    endPress(event)
  }

  // Whether the chat was ever opened: once mounted, HelpView stays mounted
  // (hidden with the panel) so the conversation is kept for the session.
  let chatMounted = $state(false)

  function handleClick(): void {
    if (suppressClick) {
      suppressClick = false
      return
    }
    if (!open) chatMounted = true
    open = !open
  }

  function closePanel(): void {
    open = false
    bubbleButton?.focus()
  }

  function toggleMaximized(): void {
    maximized = !maximized
    saveBubbleMaximized(readStorage(), maximized)
  }

  function handleWindowKeydown(event: KeyboardEvent): void {
    if (event.key !== 'Escape' || !open) return
    closePanel()
  }

  function handleWindowResize(): void {
    viewport = readViewport()
    if (userDragged) {
      // The window moved the corner under the bubble: keep its distance to the
      // corner, pulling it back inside when the window shrank (T14).
      const clamped = clampBubbleOffsets(offsets, viewport)
      offsets.right = clamped.right
      offsets.bottom = clamped.bottom
      return
    }
    // Not dragged: the anchor owns the default, so follow it (or the
    // window-centred fallback) instead of the corner (T17).
    const next = defaultOffsetsFor(viewport)
    offsets.right = next.right
    offsets.bottom = next.bottom
  }

  // The shell mounts the bubble before the status bar, so the anchor is
  // usually missing on the first paint: retry on the next frames a few
  // times, then settle on the fallback (a later resize re-measures
  // anyway). A drag in between wins over every retry.
  onMount(() => {
    if (typeof requestAnimationFrame === 'undefined') return
    if (measureAnchor()) return
    let attempts = 0
    let cancelled = false
    function tick(): void {
      if (cancelled || userDragged) return
      if (measureAnchor()) {
        const next = defaultOffsetsFor(readViewport())
        offsets.right = next.right
        offsets.bottom = next.bottom
        return
      }
      attempts += 1
      if (attempts < HELP_BUBBLE_ANCHOR_RETRIES) requestAnimationFrame(tick)
    }
    requestAnimationFrame(tick)
    return () => {
      cancelled = true
    }
  })
</script>

<svelte:window onkeydown={handleWindowKeydown} onresize={handleWindowResize} />

<button
  type="button"
  bind:this={bubbleButton}
  class="help-bubble"
  class:help-bubble--dragging={dragging}
  style:left={`${position.x}px`}
  style:top={`${position.y}px`}
  aria-label={$currentLocale && t('help.title')}
  aria-haspopup="dialog"
  aria-expanded={open}
  aria-controls={PANEL_ID}
  onpointerdown={handlePointerDown}
  onpointermove={handlePointerMove}
  onpointerup={handlePointerUp}
  onpointercancel={handlePointerCancel}
  onclick={handleClick}
  {@attach portal}
>
  <!-- The EntropIA mark, decorative: the button's accessible name carries the meaning. -->
  <img
    src={entropiaMark}
    alt=""
    aria-hidden="true"
    class="help-bubble__logo"
    draggable="false"
    style={`mask-image: url(${entropiaMark}); -webkit-mask-image: url(${entropiaMark});`}
  />
</button>

<div
  id={PANEL_ID}
  class="help-bubble__panel"
  class:help-bubble__panel--maximized={maximized}
  role="dialog"
  aria-label={$currentLocale && t('help.title')}
  data-side={panelSide}
  data-vertical={panelVertical}
  data-maximized={maximized}
  style={panelStyle}
  hidden={!open}
  tabindex="-1"
  {@attach portal}
>
  <div class="help-bubble__header">
    <span class="help-bubble__title">{$currentLocale && t('help.title')}</span>
    <div class="help-bubble__actions">
      <IconButton
        size="sm"
        variant="ghost"
        label={$currentLocale && t(maximized ? 'help.restorePanel' : 'help.maximizePanel')}
        title={$currentLocale && t(maximized ? 'help.restorePanel' : 'help.maximizePanel')}
        onclick={toggleMaximized}
      >
        <ActionIcon name={maximized ? 'restore' : 'expand'} size={14} />
      </IconButton>
      <IconButton
        size="sm"
        variant="ghost"
        label={$currentLocale && t('help.closePanel')}
        title={$currentLocale && t('help.closePanel')}
        onclick={closePanel}
      >
        <ActionIcon name="close" size={14} />
      </IconButton>
    </div>
  </div>
  <div class="help-bubble__body">
    {#if chatMounted}
      <HelpView embedded {maximized} />
    {/if}
  </div>
</div>

<style>
  .help-bubble {
    position: fixed;
    z-index: var(--z-assistant);
    display: flex;
    align-items: center;
    justify-content: center;
    width: 56px;
    height: 56px;
    padding: 0;
    border: 1px solid color-mix(in srgb, var(--color-accent) 40%, transparent);
    border-radius: 50%;
    background: color-mix(in srgb, var(--color-accent) 88%, var(--color-bg));
    box-shadow: var(--shadow-lg);
    cursor: grab;
    touch-action: none;
  }

  .help-bubble--dragging {
    cursor: grabbing;
  }

  .help-bubble:focus-visible {
    outline: none;
    box-shadow: var(--shadow-lg), var(--focus-ring);
  }

  /* The mark is a black-on-transparent PNG: as a mask it takes --color-on-accent, so it
     reads on the accent fill on every theme (same trick as TopBar's title). */
  .help-bubble__logo {
    width: 30px;
    height: 30px;
    background: var(--color-on-accent);
    object-fit: contain;
    mask-position: center;
    mask-repeat: no-repeat;
    mask-size: contain;
    -webkit-mask-position: center;
    -webkit-mask-repeat: no-repeat;
    -webkit-mask-size: contain;
    pointer-events: none;
  }

  .help-bubble__panel {
    position: fixed;
    z-index: var(--z-assistant);
    display: flex;
    flex-direction: column;
    width: min(400px, calc(100vw - 32px));
    height: min(560px, calc(100vh - 32px));
    border: 1px solid var(--border-panel);
    border-radius: var(--radius-dialog);
    background: color-mix(in srgb, var(--color-surface-elevated) 96%, var(--color-bg));
    box-shadow: var(--shadow-lg);
  }

  .help-bubble__panel--maximized {
    width: auto;
    height: auto;
  }

  .help-bubble__panel[hidden] {
    display: none;
  }

  /* Real header row (T15): always visible, never over the chat content. */
  .help-bubble__header {
    display: flex;
    flex-shrink: 0;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-4) var(--space-2);
    border-bottom: 1px solid var(--border-subtle);
  }

  .help-bubble__title {
    flex: 1;
    min-width: 0;
    margin: 0;
    overflow: hidden;
    color: var(--color-text-primary);
    font-size: var(--font-size-md);
    font-weight: var(--font-weight-semibold);
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .help-bubble__actions {
    display: flex;
    flex-shrink: 0;
    gap: var(--space-1);
  }

  /* The body fills the panel below the header; HelpView inside owns the
     scrolling middle and the fixed footer, so nothing here may clip. */
  .help-bubble__body {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    padding: var(--space-1) var(--space-4) var(--space-4);
  }

  @media (prefers-reduced-motion: reduce) {
    .help-bubble {
      transition: none;
    }
  }
</style>
