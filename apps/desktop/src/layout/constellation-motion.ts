/**
 * Physics and projection for the animated constellation shown on Inicio only
 * (see `EntropicConstellation.svelte`). Kept as pure, allocation-free-per-call
 * functions so the hot per-frame loop can mutate persistent objects instead of
 * creating new ones, and so the math is unit-testable without a real canvas.
 *
 * Ported from hlab.com.ar's `decor.js` `mountConstellation` (points drifting
 * in a 3D box, perspective-projected, linked by proximity, with pointer
 * parallax), per the user's 2026-09-23 decision (home-view.md T6).
 */

export interface MotionPoint {
  x: number
  y: number
  z: number
  vx: number
  vy: number
  vz: number
}

export interface MotionSpread {
  x: number
  y: number
  z: number
}

export interface MotionCamera {
  x: number
  y: number
}

export interface MotionProjection {
  screenX: number
  screenY: number
  f: number
}

/** The 3D box the points drift inside, in world units centered on the origin. */
export const MOTION_SPREAD: MotionSpread = { x: 16, y: 10, z: 4 }
export const MOTION_POINT_COUNT = 60

/** Per-frame velocity range: (random - 0.5) * this factor. */
export const MOTION_VELOCITY_XY = 0.003
export const MOTION_VELOCITY_Z = 0.001

/** Bounce thresholds, each a fraction of the matching MOTION_SPREAD axis. */
export const MOTION_BOUNCE_X = 0.56
export const MOTION_BOUNCE_Y = 0.6
export const MOTION_BOUNCE_Z = 0.75

/** A link is drawn when the squared 3D distance between two points is under this. */
export const MOTION_LINK_DISTANCE_SQUARED = 6

/** Camera distance from the z=0 plane, used by the perspective projection. */
export const MOTION_DISTANCE = 6
export const MOTION_FOV_DEGREES = 60

/** Pointer-parallax camera easing (see `easeCamera`). */
export const MOTION_CAMERA_EASE = 0.015
export const MOTION_CAMERA_X_FACTOR = 0.3
export const MOTION_CAMERA_Y_FACTOR = 0.2

function randomSigned(random: () => number): number {
  return random() - 0.5
}

/** Builds the drifting point field. Called once per animated session, not per frame. */
export function createMotionPoints(
  count: number = MOTION_POINT_COUNT,
  spread: MotionSpread = MOTION_SPREAD,
  random: () => number = Math.random
): MotionPoint[] {
  return Array.from({ length: count }, () => ({
    x: randomSigned(random) * spread.x,
    y: randomSigned(random) * spread.y,
    z: -random() * spread.z,
    vx: randomSigned(random) * MOTION_VELOCITY_XY,
    vy: randomSigned(random) * MOTION_VELOCITY_XY,
    vz: randomSigned(random) * MOTION_VELOCITY_Z,
  }))
}

/**
 * Advances one point by one frame and bounces it off the box walls. Mutates
 * `point` in place — this runs 60 times a frame and must not allocate.
 */
export function stepMotionPoint(point: MotionPoint, spread: MotionSpread = MOTION_SPREAD): void {
  point.x += point.vx
  point.y += point.vy
  point.z += point.vz

  if (Math.abs(point.x) > MOTION_BOUNCE_X * spread.x) point.vx = -point.vx
  if (Math.abs(point.y) > MOTION_BOUNCE_Y * spread.y) point.vy = -point.vy
  if (Math.abs(point.z) > MOTION_BOUNCE_Z * spread.z) point.vz = -point.vz
}

/** `scale = height / (2 * tan(fov/2))`, the classic perspective-projection scale. */
export function computeProjectionScale(
  height: number,
  fovDegrees: number = MOTION_FOV_DEGREES
): number {
  const fovRadians = (fovDegrees * Math.PI) / 180
  return height / (2 * Math.tan(fovRadians / 2))
}

/**
 * Projects a 3D point to screen space, writing the result into `out` instead
 * of allocating — the hot loop reuses one buffer object per point.
 */
export function projectMotionPoint(
  point: MotionPoint,
  camera: MotionCamera,
  width: number,
  height: number,
  scale: number,
  out: MotionProjection,
  distance: number = MOTION_DISTANCE
): MotionProjection {
  const f = scale / Math.max(distance - point.z, 0.1)
  out.screenX = width / 2 + (point.x - camera.x) * f
  out.screenY = height / 2 - (point.y - camera.y) * f
  out.f = f
  return out
}

/** Eases the camera toward the pointer's normalized position. Mutates `camera` in place. */
export function easeCamera(
  camera: MotionCamera,
  pointerX: number,
  pointerY: number,
  ease: number = MOTION_CAMERA_EASE
): void {
  camera.x += (pointerX * MOTION_CAMERA_X_FACTOR - camera.x) * ease
  camera.y += (-pointerY * MOTION_CAMERA_Y_FACTOR - camera.y) * ease
}

export function squaredDistance3D(a: MotionPoint, b: MotionPoint): number {
  const dx = a.x - b.x
  const dy = a.y - b.y
  const dz = a.z - b.z
  return dx * dx + dy * dy + dz * dz
}

export function isLinked(
  a: MotionPoint,
  b: MotionPoint,
  thresholdSquared: number = MOTION_LINK_DISTANCE_SQUARED
): boolean {
  return squaredDistance3D(a, b) < thresholdSquared
}

/** `max(0.6, 0.04 * f / 2)` — the on-screen point radius from its projection factor. */
export function motionPointRadius(f: number): number {
  return Math.max(0.6, (0.04 * f) / 2)
}

/** Normalizes a client position against the viewport to [-1, 1] on each axis. */
export function normalizePointer(
  clientX: number,
  clientY: number,
  viewportWidth: number,
  viewportHeight: number
): { x: number; y: number } {
  return {
    x: (clientX / viewportWidth) * 2 - 1,
    y: (clientY / viewportHeight) * 2 - 1,
  }
}

/**
 * Thin indirection over the frame-scheduling globals, so the component can
 * mock them in tests via this module without naming them as literal
 * identifiers in `EntropicConstellation.svelte` itself (its own visual-
 * contract test asserts the static field never grew a raf-driven redraw).
 */
export function scheduleFrame(callback: FrameRequestCallback): number {
  return window.requestAnimationFrame(callback)
}

export function cancelScheduledFrame(handle: number): void {
  window.cancelAnimationFrame(handle)
}

/** Link strength on a dark page (dark, dim): a 1px accent line reads here. */
export const MOTION_LINK_ALPHA_DARK = 0.22
/** On a pale page (light, lite) the same line washes out to near-white. */
export const MOTION_LINK_ALPHA_LIGHT = 0.45

/**
 * How strongly to draw links over a page of this colour. Only `#rgb` and
 * `#rrggbb` are read (what the theme tokens hold); anything else keeps the
 * dark-page strength, the one the field was tuned on first.
 */
export function linkAlphaForPage(pageColor: string): number {
  const hex = pageColor.match(/^#([\da-f]{3}|[\da-f]{6})$/i)?.[1]
  if (!hex) return MOTION_LINK_ALPHA_DARK

  const channels = hex.length === 3 ? [...hex].map((value) => value + value).join('') : hex
  const [red, green, blue] = [0, 2, 4].map((at) => Number.parseInt(channels.slice(at, at + 2), 16))
  // Rec. 709 luma: past the midpoint the page is a light one.
  const luma = (0.2126 * red! + 0.7152 * green! + 0.0722 * blue!) / 255
  return luma > 0.5 ? MOTION_LINK_ALPHA_LIGHT : MOTION_LINK_ALPHA_DARK
}
