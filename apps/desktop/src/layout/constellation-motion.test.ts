import { describe, expect, it } from 'vitest'
import {
  MOTION_BOUNCE_X,
  MOTION_BOUNCE_Y,
  MOTION_BOUNCE_Z,
  MOTION_CAMERA_EASE,
  MOTION_LINK_DISTANCE_SQUARED,
  MOTION_POINT_COUNT,
  MOTION_SPREAD,
  computeProjectionScale,
  createMotionPoints,
  easeCamera,
  isLinked,
  linkAlphaForPage,
  motionPointRadius,
  normalizePointer,
  projectMotionPoint,
  squaredDistance3D,
  stepMotionPoint,
  type MotionCamera,
  type MotionPoint,
  type MotionProjection,
} from './constellation-motion'

function point(overrides: Partial<MotionPoint> = {}): MotionPoint {
  return { x: 0, y: 0, z: 0, vx: 0, vy: 0, vz: 0, ...overrides }
}

describe('createMotionPoints', () => {
  it('builds the configured point count inside the spread box', () => {
    const points = createMotionPoints(MOTION_POINT_COUNT, MOTION_SPREAD, () => 0.5)
    expect(points).toHaveLength(MOTION_POINT_COUNT)
    // random() = 0.5 everywhere -> centered x/y, mid-depth z, zero velocity.
    expect(points[0]).toEqual({ x: 0, y: 0, z: -MOTION_SPREAD.z / 2, vx: 0, vy: 0, vz: 0 })
  })

  it('keeps z within [-spread.z, 0]', () => {
    const points = createMotionPoints(20, MOTION_SPREAD, () => 0)
    for (const p of points) {
      expect(p.z).toBeGreaterThanOrEqual(-MOTION_SPREAD.z)
      expect(p.z).toBeLessThanOrEqual(0)
    }
  })
})

describe('stepMotionPoint', () => {
  it('adds velocity to position', () => {
    const p = point({ x: 0, y: 0, z: -1, vx: 0.1, vy: -0.2, vz: 0.05 })
    stepMotionPoint(p, MOTION_SPREAD)
    expect(p.x).toBeCloseTo(0.1)
    expect(p.y).toBeCloseTo(-0.2)
    expect(p.z).toBeCloseTo(-0.95)
  })

  it('bounces (negates velocity) once |x| crosses the bounce threshold', () => {
    const limit = MOTION_BOUNCE_X * MOTION_SPREAD.x
    const p = point({ x: limit - 0.05, vx: 0.2 })
    stepMotionPoint(p, MOTION_SPREAD)
    expect(p.x).toBeGreaterThan(limit)
    expect(p.vx).toBe(-0.2)
  })

  it('bounces on y at its own threshold', () => {
    const limit = MOTION_BOUNCE_Y * MOTION_SPREAD.y
    const p = point({ y: -limit + 0.05, vy: -0.2 })
    stepMotionPoint(p, MOTION_SPREAD)
    expect(p.vy).toBe(0.2)
  })

  it('bounces on z at its own threshold', () => {
    const limit = MOTION_BOUNCE_Z * MOTION_SPREAD.z
    const p = point({ z: limit - 0.01, vz: 0.05 })
    stepMotionPoint(p, MOTION_SPREAD)
    expect(p.vz).toBe(-0.05)
  })

  it('does not bounce while inside the box', () => {
    const p = point({ x: 1, y: 1, z: -1, vx: 0.1, vy: 0.1, vz: 0.01 })
    stepMotionPoint(p, MOTION_SPREAD)
    expect(p.vx).toBe(0.1)
    expect(p.vy).toBe(0.1)
    expect(p.vz).toBe(0.01)
  })
})

describe('computeProjectionScale', () => {
  it('matches height / (2 * tan(fov/2))', () => {
    const scale = computeProjectionScale(800, 60)
    expect(scale).toBeCloseTo(800 / (2 * Math.tan(Math.PI / 6)))
  })
})

describe('projectMotionPoint', () => {
  it('projects a centered point to the screen center', () => {
    const camera: MotionCamera = { x: 0, y: 0 }
    const out: MotionProjection = { screenX: 0, screenY: 0, f: 0 }
    projectMotionPoint(point({ x: 0, y: 0, z: 0 }), camera, 1000, 800, 693, out, 6)
    expect(out.screenX).toBeCloseTo(500)
    expect(out.screenY).toBeCloseTo(400)
    expect(out.f).toBeCloseTo(693 / 6)
  })

  it('writes into the provided buffer without allocating a new object', () => {
    const camera: MotionCamera = { x: 0, y: 0 }
    const out: MotionProjection = { screenX: 0, screenY: 0, f: 0 }
    const result = projectMotionPoint(point({ x: 1, y: 1, z: -1 }), camera, 1000, 800, 693, out, 6)
    expect(result).toBe(out)
  })

  it('clamps the depth denominator to at least 0.1', () => {
    const camera: MotionCamera = { x: 0, y: 0 }
    const out: MotionProjection = { screenX: 0, screenY: 0, f: 0 }
    // distance - z = 6 - 8 = -2, which must clamp to 0.1, not go negative.
    projectMotionPoint(point({ z: 8 }), camera, 1000, 800, 693, out, 6)
    expect(out.f).toBeCloseTo(693 / 0.1)
  })
})

describe('easeCamera', () => {
  it('eases x toward pointerX * 0.3 and y toward -pointerY * 0.2', () => {
    const camera: MotionCamera = { x: 0, y: 0 }
    easeCamera(camera, 1, 1, MOTION_CAMERA_EASE)
    expect(camera.x).toBeCloseTo((1 * 0.3 - 0) * MOTION_CAMERA_EASE)
    expect(camera.y).toBeCloseTo((-1 * 0.2 - 0) * MOTION_CAMERA_EASE)
  })

  it('converges toward the target over repeated calls', () => {
    const camera: MotionCamera = { x: 0, y: 0 }
    for (let i = 0; i < 500; i++) easeCamera(camera, 1, -1, MOTION_CAMERA_EASE)
    expect(camera.x).toBeCloseTo(0.3, 2)
    expect(camera.y).toBeCloseTo(0.2, 2)
  })
})

describe('squaredDistance3D / isLinked', () => {
  it('computes the squared euclidean distance', () => {
    expect(squaredDistance3D(point({ x: 0, y: 0, z: 0 }), point({ x: 1, y: 2, z: 2 }))).toBe(9)
  })

  it('links points under the threshold and not points at or over it', () => {
    const a = point({ x: 0, y: 0, z: 0 })
    const near = point({ x: 1, y: 1, z: 1 }) // squared distance 3 < 6
    const far = point({ x: 3, y: 0, z: 0 }) // squared distance 9 >= 6
    expect(isLinked(a, near, MOTION_LINK_DISTANCE_SQUARED)).toBe(true)
    expect(isLinked(a, far, MOTION_LINK_DISTANCE_SQUARED)).toBe(false)
  })
})

describe('motionPointRadius', () => {
  it('floors at 0.6 for small projection factors', () => {
    expect(motionPointRadius(1)).toBe(0.6)
  })

  it('scales as 0.04 * f / 2 above the floor', () => {
    expect(motionPointRadius(100)).toBeCloseTo((0.04 * 100) / 2)
  })
})

describe('normalizePointer', () => {
  it('maps the viewport center to [0, 0]', () => {
    expect(normalizePointer(500, 400, 1000, 800)).toEqual({ x: 0, y: 0 })
  })

  it('maps the top-left corner to [-1, -1] and bottom-right to [1, 1]', () => {
    expect(normalizePointer(0, 0, 1000, 800)).toEqual({ x: -1, y: -1 })
    expect(normalizePointer(1000, 800, 1000, 800)).toEqual({ x: 1, y: 1 })
  })
})

describe('linkAlphaForPage', () => {
  // A 1px link in the accent at 0.22 reads on a dark page but washes out to
  // near-white on a pale one (light theme: (231,233,248) on white).
  it('keeps the dark-page strength on the dark and warm pages', () => {
    expect(linkAlphaForPage('#07080c')).toBe(0.22)
    expect(linkAlphaForPage('#15130f')).toBe(0.22)
  })

  it('raises the link strength on the light and lite pages', () => {
    expect(linkAlphaForPage('#ffffff')).toBe(0.45)
    expect(linkAlphaForPage('#f7f9f8')).toBe(0.45)
  })

  it('falls back to the dark-page strength for a colour it cannot read', () => {
    expect(linkAlphaForPage('Canvas')).toBe(0.22)
    expect(linkAlphaForPage('rgb(255, 255, 255)')).toBe(0.22)
  })
})
