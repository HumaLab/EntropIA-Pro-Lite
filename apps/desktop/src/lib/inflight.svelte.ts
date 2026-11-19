/**
 * Processes still running, kept for the whole app session.
 *
 * A button that starts a long process used to keep its "in progress" flag in
 * component state, which is lost when the user leaves the screen. Coming back
 * showed the button as available while the process kept running. This
 * registry outlives every view: a view marks a key when it starts the work
 * and reads it back on mount.
 *
 * An entry clears in one of two ways:
 * - an awaited action (`trackInflight`) clears when its promise settles;
 * - a queued job (`markInflight`) clears when the backend emits its
 *   completion or error event (see `INFLIGHT_CLEARING_EVENTS`).
 *
 * The registry lives in frontend memory: reloading the app clears it.
 */
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { SvelteMap } from 'svelte/reactivity'

type InflightEntry = {
  settled: Promise<void>
  resolve: () => void
}

const entries = new SvelteMap<string, InflightEntry>()

function createEntry(): InflightEntry {
  let resolve: () => void = () => {}
  const settled = new Promise<void>((done) => {
    resolve = done
  })
  return { settled, resolve }
}

/** Whether the process behind `key` is still running. Reactive. */
export function isInflight(key: string): boolean {
  return entries.has(key)
}

/** Records a queued job until its completion event clears it. */
export function markInflight(key: string): void {
  if (entries.has(key)) return
  entries.set(key, createEntry())
  void startInflightEventTracking()
}

/** Forgets a running process and wakes up anyone waiting for it. */
export function clearInflight(key: string): void {
  const entry = entries.get(key)
  if (!entry) return
  entries.delete(key)
  entry.resolve()
}

/**
 * Runs an awaited action under `key`, which stays in progress until the
 * promise settles, even if the view that started it is gone.
 */
export async function trackInflight<T>(key: string, run: () => Promise<T>): Promise<T> {
  const entry = createEntry()
  entries.set(key, entry)
  try {
    return await run()
  } finally {
    if (entries.get(key) === entry) entries.delete(key)
    entry.resolve()
  }
}

/**
 * Keeps the registry in step with a component flag. Call it during component
 * setup, with the flag initialised from `isInflight(key)`: a flag that starts
 * false would otherwise clear a process that is still running.
 */
export function mirrorInflight(key: string, running: () => boolean): void {
  $effect(() => {
    if (running()) markInflight(key)
    else clearInflight(key)
  })
}

/**
 * Resolves when the process behind `key` finishes, or `null` when nothing is
 * running. A remounted view uses it to reload its data afterwards.
 */
export function whenInflightSettles(key: string): Promise<void> | null {
  return entries.get(key)?.settled ?? null
}

// ─────────────────────────────────────────────────────────────────────────────
// Keys
// ─────────────────────────────────────────────────────────────────────────────

/** NLP job (`embed`, `ner`, `fts`) for an item, optionally scoped to an asset. */
export function nlpJobKey(job: string, itemId: string, assetId?: string | null): string {
  return `nlp:${job}:${itemId}:${assetId ?? ''}`
}

/** LLM job (`extract_triples`, `summarize`, `correct_ocr`) for an item or asset id. */
export function llmJobKey(job: string, targetId: string): string {
  return `llm:${job}:${targetId}`
}

export function transcriptionJobKey(assetId: string): string {
  return `transcription:${assetId}`
}

/** One of the model downloads started from Settings. */
export function downloadKey(model: string): string {
  return `download:${model}`
}

// ─────────────────────────────────────────────────────────────────────────────
// Backend events that end a queued job
// ─────────────────────────────────────────────────────────────────────────────

type NlpPayload = { item_id: string; asset_id?: string | null; job: string }
type LlmPayload = { id: string; job: string }
type TranscriptionPayload = { asset_id: string }

function nlpKeys(payload: unknown): string[] {
  const { item_id, asset_id, job } = payload as NlpPayload
  // An item-level run ends with per-asset events too: clear both.
  const keys = [nlpJobKey(job, item_id, asset_id), nlpJobKey(job, item_id)]
  // Queued triples report back as an NLP job, not as the LLM job that started them.
  if (job === 'triples') {
    keys.push(llmJobKey('extract_triples', item_id))
    if (asset_id) keys.push(llmJobKey('extract_triples', asset_id))
  }
  return keys
}

function llmKeys(payload: unknown): string[] {
  const { id, job } = payload as LlmPayload
  return [llmJobKey(job, id)]
}

function transcriptionKeys(payload: unknown): string[] {
  return [transcriptionJobKey((payload as TranscriptionPayload).asset_id)]
}

function downloadKeys(model: string): () => string[] {
  return () => [downloadKey(model)]
}

/** Each event, and the registry keys it ends. */
export const INFLIGHT_CLEARING_EVENTS: ReadonlyArray<
  readonly [event: string, keys: (payload: unknown) => string[]]
> = [
  ['nlp:complete', nlpKeys],
  ['nlp:error', nlpKeys],
  ['llm:complete', llmKeys],
  ['llm:error', llmKeys],
  ['transcription:complete', transcriptionKeys],
  ['transcription:error', transcriptionKeys],
]

/** Adds the complete/error events of a Settings model download. */
export function inflightDownloadEvents(
  model: string,
  completeEvent: string,
  errorEvent: string
): ReadonlyArray<readonly [string, (payload: unknown) => string[]]> {
  return [
    [completeEvent, downloadKeys(model)],
    [errorEvent, downloadKeys(model)],
  ]
}

// GLM-OCR and Qwen send one `complete` per file, so only an error ends them
// here; the views end them once the inventory says both files are on disk.
const TWO_FILE_DOWNLOADS = new Set(['glm-ocr', 'qwen-correct'])

/** Registers the events that end each model download, by event scope. */
export function registerModelDownloads(scopes: Iterable<string>): void {
  registerInflightEvents(
    [...scopes].flatMap((scope) =>
      inflightDownloadEvents(
        scope,
        TWO_FILE_DOWNLOADS.has(scope) ? `${scope}:download_error` : `${scope}:download_complete`,
        `${scope}:download_error`
      )
    )
  )
}

let tracking: Promise<void> | null = null
let unlisteners: UnlistenFn[] = []
const extraEvents: Array<readonly [string, (payload: unknown) => string[]]> = []

/**
 * Registers more clearing events (model downloads). Safe to call before or
 * after tracking starts; an event is registered once.
 */
export function registerInflightEvents(
  events: ReadonlyArray<readonly [string, (payload: unknown) => string[]]>
): void {
  for (const event of events) {
    if (extraEvents.some(([name]) => name === event[0])) continue
    extraEvents.push(event)
    if (tracking) void tracking.then(() => listenTo(event))
  }
}

async function listenTo([event, keysOf]: readonly [string, (payload: unknown) => string[]]) {
  const unlisten = await listen(event, (e) => {
    for (const key of keysOf(e.payload)) clearInflight(key)
  })
  unlisteners.push(unlisten)
}

/**
 * Listens, once for the whole session, to the events that end queued jobs.
 * AppShell starts it at launch; `markInflight` starts it too, as a fallback.
 */
export function startInflightEventTracking(): Promise<void> {
  tracking ??= Promise.all(
    [...INFLIGHT_CLEARING_EVENTS, ...extraEvents].map((event) => listenTo(event))
  ).then(
    () => undefined,
    (error) => {
      console.error('[inflight] event tracking failed:', error)
      tracking = null
    }
  )
  return tracking
}

/** Test-only: forgets every entry and listener. */
export function resetInflightForTests(): void {
  for (const unlisten of unlisteners) unlisten()
  unlisteners = []
  tracking = null
  extraEvents.length = 0
  for (const key of [...entries.keys()]) clearInflight(key)
}
