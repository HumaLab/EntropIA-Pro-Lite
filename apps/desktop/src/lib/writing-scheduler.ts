/**
 * When to persist while someone is writing (plan-editor.md §16.1).
 *
 * Two cadences, not one, because the two writes cost very different things.
 * Spike S6 measured a journal delta at a p95 of 1.17 ms and the whole manuscript
 * at 97.5 ms, so the recovery journal can run often and the canonical save
 * cannot.
 *
 * The rule that matters is the second clause of §16.1: a debounce alone lets
 * someone who never pauses postpone recovery forever. So the journal also has a
 * **maximum interval** — while there is unsaved work, it fires whether or not
 * the typing stopped.
 *
 * This module is deliberately pure: no timers, no Tauri, no editor. It answers
 * "given the clock and what has happened, what is due now and when should I
 * look again?", which is the part worth testing.
 *
 * **The caller must honour `nextCheckInMs`.** Asking only on keystrokes looks
 * like it works and quietly breaks the ceiling: between two keystrokes 200 ms
 * apart there is no moment at 500 ms to notice the ceiling came due, so the
 * first journal write slips to 600 ms and the declared loss window is a lie.
 * Wake on edits *and* on the timer this returns.
 */

export interface SchedulerConfig {
  /** Quiet period after the last edit before journalling. */
  journalDebounceMs: number
  /**
   * Hard ceiling between journal writes while work is pending. S6's measured
   * recommendation; the declared loss window is this plus the worst observed
   * write latency (~60 ms).
   */
  journalMaxIntervalMs: number
  /** Quiet period after the last edit before a canonical save. */
  saveDebounceMs: number
}

export const DEFAULT_SCHEDULER: SchedulerConfig = {
  journalDebounceMs: 250,
  journalMaxIntervalMs: 500,
  saveDebounceMs: 1000,
}

export interface SchedulerState {
  /** When the current run of unsaved edits began; null when everything is committed. */
  pendingSinceAt: number | null
  /** When the most recent edit landed. */
  lastEditAt: number | null
  /** When the journal last accepted a delta, if it has during this run. */
  lastJournalAt: number | null
}

export const CLEAN_STATE: SchedulerState = {
  pendingSinceAt: null,
  lastEditAt: null,
  lastJournalAt: null,
}

export interface SchedulerDecision {
  /** Append the pending delta to the recovery journal now. */
  journal: boolean
  /** Advance the canonical revision now. */
  save: boolean
  /** Why the journal is firing — useful in logs and in tests. */
  journalReason: 'idle' | 'debounce' | 'max-interval'
  /** When to ask again, as a delay in ms. Null when there is nothing pending. */
  nextCheckInMs: number | null
}

const IDLE: SchedulerDecision = {
  journal: false,
  save: false,
  journalReason: 'idle',
  nextCheckInMs: null,
}

/**
 * Records an edit. Starts a pending run if one is not already open.
 */
export function onEdit(state: SchedulerState, now: number): SchedulerState {
  return {
    pendingSinceAt: state.pendingSinceAt ?? now,
    lastEditAt: now,
    lastJournalAt: state.lastJournalAt,
  }
}

/**
 * Records that the journal accepted a delta. The pending run stays open: a
 * journal write is not a canonical save (§16.2), so the work is still unsaved.
 */
export function onJournalled(state: SchedulerState, now: number): SchedulerState {
  return { ...state, lastJournalAt: now }
}

/**
 * Records that persistence acknowledged a new revision. Only this closes the
 * pending run, and only this earns "Guardado" in the UI.
 */
export function onSaved(): SchedulerState {
  return { ...CLEAN_STATE }
}

/** What is due at `now`, and when to look again. */
export function decide(
  state: SchedulerState,
  now: number,
  config: SchedulerConfig = DEFAULT_SCHEDULER
): SchedulerDecision {
  if (state.pendingSinceAt === null || state.lastEditAt === null) return IDLE

  const sinceEdit = now - state.lastEditAt
  // The ceiling counts from the last journal write, or from the start of the
  // pending run if nothing has been journalled yet.
  const journalAnchor = state.lastJournalAt ?? state.pendingSinceAt
  const sinceJournal = now - journalAnchor

  const debounceDue = sinceEdit >= config.journalDebounceMs
  const ceilingDue = sinceJournal >= config.journalMaxIntervalMs
  const saveDue = sinceEdit >= config.saveDebounceMs

  const journalIn = Math.max(0, config.journalDebounceMs - sinceEdit)
  const ceilingIn = Math.max(0, config.journalMaxIntervalMs - sinceJournal)
  const saveIn = Math.max(0, config.saveDebounceMs - sinceEdit)

  const waits = [journalIn, ceilingIn, saveIn].filter((w) => w > 0)

  return {
    journal: debounceDue || ceilingDue,
    save: saveDue,
    // The ceiling is the interesting reason: it means someone is typing without
    // pauses and the debounce alone would never have fired.
    journalReason: ceilingDue && !debounceDue ? 'max-interval' : debounceDue ? 'debounce' : 'idle',
    nextCheckInMs: waits.length > 0 ? Math.min(...waits) : 0,
  }
}

/**
 * The loss window this configuration declares, for §25's criterion 5: the
 * ceiling plus the worst write latency measured in S6.
 */
export function declaredLossWindowMs(
  config: SchedulerConfig = DEFAULT_SCHEDULER,
  worstWriteLatencyMs = 61
): number {
  return config.journalMaxIntervalMs + worstWriteLatencyMs
}
