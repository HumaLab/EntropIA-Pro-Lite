import { describe, expect, it } from 'vitest'
import {
  CLEAN_STATE,
  DEFAULT_SCHEDULER,
  declaredLossWindowMs,
  decide,
  onEdit,
  onJournalled,
  onSaved,
  type SchedulerState,
} from './writing-scheduler'

const cfg = DEFAULT_SCHEDULER

describe('writing scheduler — nothing pending', () => {
  it('asks for nothing while the document is clean', () => {
    const d = decide(CLEAN_STATE, 1000, cfg)
    expect(d).toMatchObject({ journal: false, save: false, nextCheckInMs: null })
  })

  it('goes quiet again once persistence acknowledged a revision', () => {
    let s = onEdit(CLEAN_STATE, 0)
    s = onJournalled(s, 300)
    s = onSaved()
    expect(decide(s, 5000, cfg).nextCheckInMs).toBeNull()
  })
})

describe('writing scheduler — a pause', () => {
  it('waits out the debounce before journalling', () => {
    const s = onEdit(CLEAN_STATE, 0)
    expect(decide(s, cfg.journalDebounceMs - 1, cfg).journal).toBe(false)
    expect(decide(s, cfg.journalDebounceMs, cfg)).toMatchObject({
      journal: true,
      journalReason: 'debounce',
    })
  })

  it('waits longer before the canonical save, because it costs more', () => {
    const s = onEdit(CLEAN_STATE, 0)
    expect(decide(s, cfg.journalDebounceMs, cfg).save).toBe(false)
    expect(decide(s, cfg.saveDebounceMs, cfg).save).toBe(true)
    expect(cfg.saveDebounceMs).toBeGreaterThan(cfg.journalDebounceMs)
  })
})

describe('writing scheduler — continuous typing', () => {
  /**
   * Someone typing steadily, never pausing long enough for the debounce.
   *
   * The caller wakes on two things, as the contract requires: every edit, and
   * the timer `nextCheckInMs` asks for. Checking only on edits is what lets the
   * ceiling overshoot, which is exactly why the decision carries that number.
   */
  function typeWithoutPausing(untilMs: number, stepMs: number) {
    let state: SchedulerState = CLEAN_STATE
    const journalledAt: number[] = []
    let now = 0
    let nextEditAt = 0

    while (now <= untilMs) {
      if (now >= nextEditAt) {
        state = onEdit(state, now)
        nextEditAt = now + stepMs
      }
      const d = decide(state, now, cfg)
      if (d.journal) {
        journalledAt.push(now)
        state = onJournalled(state, now)
      }
      const wake = decide(state, now, cfg).nextCheckInMs
      const nextWakeAt = wake === null ? nextEditAt : now + Math.max(wake, 1)
      now = Math.min(nextEditAt, nextWakeAt)
    }
    return { journalledAt, state }
  }

  it('still journals, because the ceiling fires when the debounce never can', () => {
    const step = cfg.journalDebounceMs - 50 // always shorter than the debounce
    const { journalledAt } = typeWithoutPausing(3000, step)

    expect(journalledAt.length).toBeGreaterThan(0)
    const reason = (() => {
      let s: SchedulerState = CLEAN_STATE
      for (let now = 0; now <= cfg.journalMaxIntervalMs; now += step) s = onEdit(s, now)
      return decide(s, cfg.journalMaxIntervalMs, cfg).journalReason
    })()
    expect(reason).toBe('max-interval')
  })

  it('never lets more than the declared ceiling pass between journal writes', () => {
    const { journalledAt } = typeWithoutPausing(5000, cfg.journalDebounceMs - 50)
    const gaps = journalledAt.slice(1).map((t, i) => t - journalledAt[i]!)
    for (const gap of gaps) {
      expect(gap).toBeLessThanOrEqual(cfg.journalMaxIntervalMs)
    }
    // And the first write lands within the ceiling of the run starting.
    expect(journalledAt[0]).toBeLessThanOrEqual(cfg.journalMaxIntervalMs)
  })

  it('does not save canonically while the typing never stops', () => {
    let state: SchedulerState = CLEAN_STATE
    let saves = 0
    for (let now = 0; now <= 5000; now += cfg.saveDebounceMs - 100) {
      state = onEdit(state, now)
      if (decide(state, now, cfg).save) saves += 1
    }
    expect(saves).toBe(0)
  })
})

describe('writing scheduler — journalling is not saving (§16.2)', () => {
  it('keeps the run pending after a journal write', () => {
    let s = onEdit(CLEAN_STATE, 0)
    s = onJournalled(s, 250)
    expect(s.pendingSinceAt).not.toBeNull()
    expect(decide(s, 10_000, cfg).save).toBe(true)
  })

  it('restarts the ceiling from the last journal write, not from the run start', () => {
    let s = onEdit(CLEAN_STATE, 0)
    s = onJournalled(s, 400)
    s = onEdit(s, 450)
    // 500 ms after the run began, but only 100 ms since the journal accepted it.
    expect(decide(s, 500, cfg).journal).toBe(false)
    expect(decide(s, 400 + cfg.journalMaxIntervalMs, cfg).journal).toBe(true)
  })
})

describe('writing scheduler — the declared loss window', () => {
  it('is the ceiling plus the worst write latency S6 measured', () => {
    expect(declaredLossWindowMs(cfg, 61)).toBe(cfg.journalMaxIntervalMs + 61)
    expect(declaredLossWindowMs()).toBeLessThanOrEqual(600)
  })
})
