/**
 * Wordle-style spiral word-cloud layout. Pure and deterministic:
 * rotation and spiral start angle derive from a hash of the word
 * (no Math.random), so the same input always yields the same layout
 * and the cloud does not jump between re-renders.
 */

export interface CloudWordInput {
  word: string
  count: number
}

export interface PlacedCloudWord {
  word: string
  count: number
  x: number
  y: number
  fontSize: number
  rotated: boolean
}

export interface CloudLayoutOptions {
  width: number
  height: number
  minFontSize?: number
  maxFontSize?: number
  /** Gap in px kept around each word box. */
  padding?: number
  /** Approximate fraction of words rotated 90° (0 disables rotation). */
  rotationRatio?: number
  measure?: (word: string, fontSizePx: number) => { width: number; height: number }
}

/** Font stack used both for measuring and for rendering the cloud. */
export const CLOUD_FONT_STACK = 'system-ui, -apple-system, sans-serif'
export const CLOUD_FONT_WEIGHT = 600

const DEFAULT_MIN_FONT = 11
const DEFAULT_MAX_FONT = 44
const DEFAULT_PADDING = 2
const DEFAULT_ROTATION_RATIO = 0.25
/** Radius gained per full spiral turn, in px. */
const SPIRAL_PITCH = 7
/** Distance between consecutive spiral samples, in px of arc. */
const SPIRAL_ARC_STEP = 9
/** Vertical squish so the spiral fills a landscape canvas. */
const SPIRAL_ECCENTRICITY = 0.72

function hashWord(word: string): number {
  let hash = 5381
  for (let i = 0; i < word.length; i++) {
    hash = ((hash << 5) + hash + word.charCodeAt(i)) >>> 0
  }
  return hash
}

function fontSizeFor(
  count: number,
  minCount: number,
  maxCount: number,
  minFont: number,
  maxFont: number
): number {
  if (maxCount <= minCount) return (minFont + maxFont) / 2
  const ratio = Math.sqrt((count - minCount) / (maxCount - minCount))
  return minFont + (maxFont - minFont) * ratio
}

let measureCtx: CanvasRenderingContext2D | null | undefined

function defaultMeasure(word: string, fontSizePx: number): { width: number; height: number } {
  if (measureCtx === undefined) {
    try {
      measureCtx = document.createElement('canvas').getContext('2d')
    } catch {
      measureCtx = null
    }
  }
  if (measureCtx) {
    measureCtx.font = `${CLOUD_FONT_WEIGHT} ${fontSizePx}px ${CLOUD_FONT_STACK}`
    return { width: measureCtx.measureText(word).width, height: fontSizePx * 1.15 }
  }
  // Headless environments (jsdom) have no 2d context — rough estimate.
  return { width: word.length * fontSizePx * 0.6, height: fontSizePx * 1.15 }
}

interface Box {
  x0: number
  y0: number
  x1: number
  y1: number
}

function intersects(a: Box, b: Box): boolean {
  return a.x0 < b.x1 && a.x1 > b.x0 && a.y0 < b.y1 && a.y1 > b.y0
}

/** Shrink factor applied between placement retries of the same word. */
const SHRINK_FACTOR = 0.85
/**
 * Max fraction of the canvas the summed word boxes may request. Above
 * this, all font sizes are scaled down globally so dense clouds (many
 * terms, long words) still fit instead of dropping the tail.
 */
const TARGET_DENSITY = 0.62

/**
 * Place words (largest first) on an Archimedean spiral from the center.
 * A word that cannot fit at its scaled size retries at progressively
 * smaller sizes (down to minFontSize) before being dropped, so dense
 * clouds degrade by shrinking the tail instead of losing words.
 */
export function layoutWordCloud(
  words: CloudWordInput[],
  opts: CloudLayoutOptions
): PlacedCloudWord[] {
  const { width, height } = opts
  const minFont = opts.minFontSize ?? DEFAULT_MIN_FONT
  const maxFont = opts.maxFontSize ?? DEFAULT_MAX_FONT
  const padding = opts.padding ?? DEFAULT_PADDING
  const rotationRatio = opts.rotationRatio ?? DEFAULT_ROTATION_RATIO
  const measure = opts.measure ?? defaultMeasure

  if (words.length === 0) return []

  const sorted = [...words].sort((a, b) => b.count - a.count || a.word.localeCompare(b.word, 'es'))
  const maxCount = sorted[0]!.count
  const minCount = sorted[sorted.length - 1]!.count
  const rotationModulo =
    rotationRatio <= 0 ? Number.POSITIVE_INFINITY : Math.max(1, Math.round(1 / rotationRatio))

  const cx = width / 2
  const cy = height / 2
  const placed: PlacedCloudWord[] = []
  const boxes: Box[] = []

  function tryPlace(word: string, fontSize: number, rotated: boolean, hash: number) {
    const metrics = measure(word, fontSize)
    const boxW = (rotated ? metrics.height : metrics.width) + padding * 2
    const boxH = (rotated ? metrics.width : metrics.height) + padding * 2

    if (boxW > width || boxH > height) return null

    // Constant arc-length sampling: angular step shrinks as the radius
    // grows, so coverage stays uniform instead of leaving gaps far from
    // the center.
    const maxRadius = Math.hypot(width, height) / 2
    let angle = (hash % 360) * (Math.PI / 180)
    let radius = 0
    while (radius <= maxRadius) {
      const x = cx + radius * Math.cos(angle)
      const y = cy + radius * SPIRAL_ECCENTRICITY * Math.sin(angle)
      const candidate: Box = {
        x0: x - boxW / 2,
        y0: y - boxH / 2,
        x1: x + boxW / 2,
        y1: y + boxH / 2,
      }
      const inBounds =
        candidate.x0 >= 0 && candidate.y0 >= 0 && candidate.x1 <= width && candidate.y1 <= height
      if (inBounds && !boxes.some((box) => intersects(candidate, box))) {
        boxes.push(candidate)
        return { x, y }
      }
      const angleStep = Math.min(0.5, SPIRAL_ARC_STEP / Math.max(radius, 1))
      angle += angleStep
      radius += (SPIRAL_PITCH * angleStep) / (2 * Math.PI)
    }
    return null
  }

  // Global auto-fit: when the requested boxes exceed the packable
  // density, scale every font down (sqrt keeps relative proportions).
  let requestedArea = 0
  const scaledSizes = sorted.map(({ word, count }) => {
    const fontSize = fontSizeFor(count, minCount, maxCount, minFont, maxFont)
    const metrics = measure(word, fontSize)
    requestedArea += (metrics.width + padding * 2) * (metrics.height + padding * 2)
    return fontSize
  })
  const capacity = width * height * TARGET_DENSITY
  const globalScale = requestedArea > capacity ? Math.sqrt(capacity / requestedArea) : 1

  for (let i = 0; i < sorted.length; i++) {
    const { word, count } = sorted[i]!
    const targetSize = Math.max(minFont, scaledSizes[i]! * globalScale)
    const hash = hashWord(word)
    const rotated = Number.isFinite(rotationModulo) && hash % rotationModulo === 0

    for (let fontSize = targetSize; fontSize >= minFont - 0.01; fontSize *= SHRINK_FACTOR) {
      const position = tryPlace(word, fontSize, rotated, hash)
      if (position) {
        placed.push({ word, count, x: position.x, y: position.y, fontSize, rotated })
        break
      }
    }
  }

  return placed
}
