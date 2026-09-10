<script lang="ts">
  import { onMount } from 'svelte'

  interface EntropicNode {
    id: number
    x: number
    y: number
    size: number
    alpha: number
    depth: number
  }

  const MIN_NODES = 180
  const MAX_NODES = 520
  const NODE_DENSITY = 4200
  const LINK_DISTANCE = 104
  const GRID_SIZE = LINK_DISTANCE
  const MAX_DEVICE_PIXEL_RATIO = 1.35
  const CANVAS_OVERSCAN = 140

  let canvas: HTMLCanvasElement
  let ctx: CanvasRenderingContext2D | null = null
  let nodes: EntropicNode[] = []
  let width = 0
  let height = 0
  let deviceScale = 1
  let reducedMotion = false
  let resizeTimer: ReturnType<typeof setTimeout> | null = null

  function clamp(value: number, min: number, max: number) {
    return Math.min(max, Math.max(min, value))
  }

  function randomBetween(min: number, max: number) {
    return min + Math.random() * (max - min)
  }

  function readThemeColor(localVariable: string, ...tokenFallbacks: string[]) {
    const elementStyle = getComputedStyle(canvas)
    const rootStyle = getComputedStyle(document.documentElement)

    for (const variable of [localVariable, ...tokenFallbacks]) {
      const style = variable === localVariable ? elementStyle : rootStyle
      const color = style.getPropertyValue(variable).trim()
      if (color && !color.includes('var(')) return color
    }

    return 'Canvas'
  }

  function colorWithAlpha(color: string, alpha: number) {
    const hex = color.match(/^#([\da-f]{3}|[\da-f]{6})$/i)?.[1]
    if (!hex) return color

    const channels = hex.length === 3 ? [...hex].map((value) => value + value).join('') : hex
    const red = Number.parseInt(channels.slice(0, 2), 16)
    const green = Number.parseInt(channels.slice(2, 4), 16)
    const blue = Number.parseInt(channels.slice(4, 6), 16)

    return `rgba(${red}, ${green}, ${blue}, ${alpha})`
  }

  function generateNodes(nextWidth: number, nextHeight: number): EntropicNode[] {
    const area = nextWidth * nextHeight
    const count = clamp(Math.floor(area / NODE_DENSITY), MIN_NODES, MAX_NODES)
    const anchorCount = clamp(Math.floor(nextWidth / 210), 7, 18)
    const anchors = Array.from({ length: anchorCount }, (_, index) => {
      const normalized = anchorCount === 1 ? 0.5 : index / (anchorCount - 1)
      return {
        x: normalized * nextWidth + randomBetween(-nextWidth * 0.06, nextWidth * 0.06),
        y: nextHeight * randomBetween(0.12, 0.9),
      }
    })

    return Array.from({ length: count }, (_, index) => {
      const usesAnchor = Math.random() > 0.18
      const anchor = anchors[index % anchors.length] ?? { x: nextWidth / 2, y: nextHeight / 2 }
      const x = usesAnchor
        ? clamp(anchor.x + randomBetween(-nextWidth * 0.13, nextWidth * 0.13), 0, nextWidth)
        : Math.random() * nextWidth
      const y = usesAnchor
        ? clamp(anchor.y + randomBetween(-nextHeight * 0.18, nextHeight * 0.18), 0, nextHeight)
        : Math.random() * nextHeight
      const depth = Math.random()

      return {
        id: index,
        x,
        y,
        size: randomBetween(0.25, 0.78) + depth * 0.22,
        alpha: randomBetween(0.018, 0.075) + depth * 0.028,
        depth,
      }
    })
  }

  function resizeCanvas() {
    if (!canvas) return

    width = Math.max(1, window.innerWidth + CANVAS_OVERSCAN * 2)
    height = Math.max(1, window.innerHeight + CANVAS_OVERSCAN * 2)
    deviceScale = Math.min(window.devicePixelRatio || 1, MAX_DEVICE_PIXEL_RATIO)

    canvas.width = Math.floor(width * deviceScale)
    canvas.height = Math.floor(height * deviceScale)
    canvas.style.width = `${width}px`
    canvas.style.height = `${height}px`
    canvas.style.left = `${-CANVAS_OVERSCAN}px`
    canvas.style.top = `${-CANVAS_OVERSCAN}px`

    ctx = canvas.getContext('2d', { alpha: false })
    ctx?.setTransform(deviceScale, 0, 0, deviceScale, 0, 0)
    nodes = generateNodes(width, height)
    renderConstellation()
  }

  function scheduleResize() {
    if (resizeTimer) clearTimeout(resizeTimer)
    resizeTimer = setTimeout(resizeCanvas, 120)
  }

  function drawBackground(context: CanvasRenderingContext2D) {
    const gradient = context.createLinearGradient(0, 0, width, height)
    gradient.addColorStop(
      0,
      readThemeColor('--constellation-bg-start', '--surface-app', '--color-bg')
    )
    gradient.addColorStop(
      0.46,
      readThemeColor('--constellation-bg-mid', '--color-bg-ambient', '--color-surface')
    )
    gradient.addColorStop(
      1,
      readThemeColor('--constellation-bg-end', '--surface-panel', '--color-surface')
    )
    context.fillStyle = gradient
    context.fillRect(0, 0, width, height)

    const haze = context.createRadialGradient(
      width * 0.5,
      height * 0.52,
      0,
      width * 0.5,
      height * 0.52,
      width * 0.72
    )
    haze.addColorStop(
      0,
      colorWithAlpha(readThemeColor('--constellation-haze-core', '--color-accent'), 0.035)
    )
    haze.addColorStop(
      0.48,
      colorWithAlpha(
        readThemeColor('--constellation-haze-mid', '--surface-glass', '--color-surface-glass'),
        0.022
      )
    )
    haze.addColorStop(
      1,
      colorWithAlpha(readThemeColor('--constellation-haze-edge', '--surface-app', '--color-bg'), 0)
    )
    context.fillStyle = haze
    context.fillRect(0, 0, width, height)
  }

  function buildSpatialGrid() {
    const grid = new Map<string, EntropicNode[]>()
    for (const node of nodes) {
      const cellX = Math.floor(node.x / GRID_SIZE)
      const cellY = Math.floor(node.y / GRID_SIZE)
      const key = `${cellX}:${cellY}`
      const bucket = grid.get(key)
      if (bucket) bucket.push(node)
      else grid.set(key, [node])
    }
    return grid
  }

  function drawLinks(context: CanvasRenderingContext2D, grid: Map<string, EntropicNode[]>) {
    context.lineWidth = 0.45

    for (const [key, bucket] of grid) {
      const [rawX, rawY] = key.split(':')
      const cellX = Number(rawX)
      const cellY = Number(rawY)

      for (let dx = -1; dx <= 1; dx++) {
        for (let dy = -1; dy <= 1; dy++) {
          const neighbor = grid.get(`${cellX + dx}:${cellY + dy}`)
          if (!neighbor) continue

          for (const a of bucket) {
            for (const b of neighbor) {
              if (b.id <= a.id) continue
              const diffX = a.x - b.x
              const diffY = a.y - b.y
              const distance = Math.hypot(diffX, diffY)
              if (distance > LINK_DISTANCE) continue

              const alpha =
                (1 - distance / LINK_DISTANCE) * 0.018 * (0.75 + (a.depth + b.depth) * 0.24)
              context.beginPath()
              context.moveTo(a.x, a.y)
              context.lineTo(b.x, b.y)
              context.strokeStyle = `rgba(146, 154, 178, ${alpha})`
              context.stroke()
            }
          }
        }
      }
    }
  }

  function drawNodes(context: CanvasRenderingContext2D) {
    for (const node of nodes) {
      context.beginPath()
      context.arc(node.x, node.y, node.size, 0, Math.PI * 2)
      context.fillStyle = `rgba(190, 197, 214, ${node.alpha})`
      context.fill()
    }
  }

  function renderConstellation() {
    if (!ctx || !canvas) return

    drawBackground(ctx)
    const grid = buildSpatialGrid()
    drawLinks(ctx, grid)
    drawNodes(ctx)
  }

  onMount(() => {
    reducedMotion = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false
    resizeCanvas()
    window.addEventListener('resize', scheduleResize)

    return () => {
      if (resizeTimer) clearTimeout(resizeTimer)
      window.removeEventListener('resize', scheduleResize)
    }
  })
</script>

<canvas
  bind:this={canvas}
  class="constellation"
  class:constellation--motion={!reducedMotion}
  aria-hidden="true"
></canvas>

<style>
  .constellation {
    --constellation-bg-start: var(--surface-app, var(--color-bg));
    --constellation-bg-mid: var(--color-bg-ambient, var(--surface-panel));
    --constellation-bg-end: var(--surface-panel, var(--color-surface));
    --constellation-haze-core: var(--color-accent);
    --constellation-haze-mid: var(--surface-glass, var(--color-surface-glass));
    --constellation-haze-edge: var(--surface-app, var(--color-bg));

    position: fixed;
    left: 0;
    top: 0;
    z-index: 0;
    pointer-events: none;
    transform-origin: center;
    background:
      radial-gradient(
        circle at 50% 50%,
        color-mix(in srgb, var(--constellation-haze-core) 12%, transparent),
        transparent 58%
      ),
      linear-gradient(
        135deg,
        var(--constellation-bg-start) 0%,
        var(--constellation-bg-mid) 48%,
        var(--constellation-bg-end) 100%
      );
    will-change: transform, opacity;
  }

  .constellation--motion {
    animation: none;
  }

  @keyframes entropic-drift {
    0% {
      opacity: 0.92;
      transform: translate3d(-6px, -2px, 0) rotate(-0.12deg) scale(1.006);
    }
    50% {
      opacity: 0.98;
      transform: translate3d(5px, 4px, 0) rotate(0.1deg) scale(1.01);
    }
    100% {
      opacity: 0.94;
      transform: translate3d(11px, -5px, 0) rotate(0.22deg) scale(1.008);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .constellation--motion {
      animation: none;
    }
  }
</style>
