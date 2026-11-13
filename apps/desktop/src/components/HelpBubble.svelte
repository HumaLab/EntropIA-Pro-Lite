<script lang="ts">
  import { locale, t } from '$lib/i18n'
  import {
    clampBubblePosition,
    defaultBubblePosition,
    exceedsDragThreshold,
    HELP_BUBBLE_SIZE_PX,
    loadBubblePosition,
    saveBubblePosition,
    type BubblePosition,
    type BubbleViewport,
  } from '$lib/help-bubble'
  import HelpView from '../views/HelpView.svelte'
  import { ActionIcon, IconButton, portal } from '@entropia/ui'
  import entropiaMark from '../assets/entropia-mark.png'

  const currentLocale = locale
  const PANEL_ID = 'help-bubble-panel'
  const PANEL_GAP_PX = 12

  function readStorage(): Pick<Storage, 'getItem' | 'setItem'> | null {
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

  function initialPosition(viewport: BubbleViewport): BubblePosition {
    const stored = loadBubblePosition(readStorage())
    return clampBubblePosition(stored ?? defaultBubblePosition(viewport), viewport)
  }

  // Snapshot once: the live viewport below re-reads the window on resize,
  // but the initial position needs the same values the first paint uses.
  const initialViewport = readViewport()
  let viewport = $state<BubbleViewport>(initialViewport)
  let position = $state<BubblePosition>(initialPosition(initialViewport))
  let open = $state(false)
  // The chat mounts on first open and stays alive afterwards: opening the
  // panel never costs a backend call while it stays shut (so shell tests
  // rendering AppShell never see the help bridge), and the conversation
  // survives closing the panel (D4: session memory).
  let dragging = $state(false)
  let pressStart: { x: number; y: number; position: BubblePosition } | null = null
  let suppressClick = false
  let bubbleButton: HTMLButtonElement | undefined = $state()

  // The panel grows toward the inside of the window from wherever the bubble
  // is, so it never hangs off-screen (S13).
  const panelSide = $derived(
    position.x + HELP_BUBBLE_SIZE_PX / 2 > viewport.width / 2 ? 'left' : 'right'
  )
  const panelVertical = $derived(
    position.y + HELP_BUBBLE_SIZE_PX / 2 > viewport.height / 2 ? 'above' : 'below'
  )
  const panelStyle = $derived.by(() => {
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
    position = clampBubblePosition(
      {
        x: pressStart.position.x + (event.clientX - pressStart.x),
        y: pressStart.position.y + (event.clientY - pressStart.y),
      },
      viewport
    )
  }

  function endPress(event: PointerEvent, save: boolean): void {
    if (!pressStart) return
    pressStart = null
    releaseCapture(event.currentTarget as HTMLElement, event.pointerId)
    if (dragging && save) {
      saveBubblePosition(readStorage(), position)
      // Browsers fire a click after a drag: it must not toggle the panel.
      suppressClick = true
    }
    dragging = false
  }

  function handlePointerUp(event: PointerEvent): void {
    endPress(event, true)
  }

  function handlePointerCancel(event: PointerEvent): void {
    endPress(event, false)
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

  function handleWindowKeydown(event: KeyboardEvent): void {
    if (event.key !== 'Escape' || !open) return
    closePanel()
  }

  function handleWindowResize(): void {
    viewport = readViewport()
    // The window shrank under the bubble: pull it back inside (D9).
    position = clampBubblePosition(position, viewport)
  }
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
  role="dialog"
  aria-label={$currentLocale && t('help.title')}
  data-side={panelSide}
  data-vertical={panelVertical}
  style={panelStyle}
  hidden={!open}
  tabindex="-1"
  {@attach portal}
>
  <div class="help-bubble__close">
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
  <div class="help-bubble__body">
    {#if chatMounted}
      <HelpView />
    {/if}
  </div>
</div>

<style>
  .help-bubble {
    position: fixed;
    z-index: 600;
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

  /* The mark is a black-on-transparent PNG: as a mask it takes white, so it
     reads on the accent fill on every theme (same trick as TopBar's title). */
  .help-bubble__logo {
    width: 30px;
    height: 30px;
    background: #fff;
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
    z-index: 600;
    display: flex;
    flex-direction: column;
    width: min(400px, calc(100vw - 32px));
    height: min(560px, calc(100vh - 32px));
    border: 1px solid var(--border-panel);
    border-radius: var(--radius-dialog);
    background: color-mix(in srgb, var(--color-surface-elevated) 96%, var(--color-bg));
    box-shadow: var(--shadow-lg);
  }

  .help-bubble__panel[hidden] {
    display: none;
  }

  .help-bubble__close {
    position: absolute;
    top: var(--space-2);
    right: var(--space-2);
    z-index: 1;
  }

  .help-bubble__body {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    padding: var(--space-4);
    padding-top: var(--space-2);
    overflow: hidden;
  }

  @media (prefers-reduced-motion: reduce) {
    .help-bubble {
      transition: none;
    }
  }
</style>
