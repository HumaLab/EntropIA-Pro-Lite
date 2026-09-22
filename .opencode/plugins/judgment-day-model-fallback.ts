/**
 * judgment-day-model-fallback.ts
 *
 * Local per-judge model fallback for the judgment-day protocol.
 *
 * Why this plugin exists:
 * - Official OpenCode schema accepts only a single string in each agent's
 *   `model` field, so per-agent ordered chains cannot be expressed in
 *   `opencode.json`. This plugin performs a conservative client-side
 *   re-prompt when a judge call fails transiently. It does NOT claim that
 *   OpenCode core supports fallback arrays.
 *
 * Auto-discovery:
 * - Placed under `.opencode/plugins/`, which OpenCode auto-discovers.
 * - No `opencode.json` change is needed for discovery.
 * - No npm dependency is required; this file is self-contained.
 *
 * Legacy hook-object API:
 * - Exports a Plugin-compatible async function that receives the plugin
 *   context (`{ client, directory, ... }`) and returns a hook object with
 *   `event` and `chat.message` hooks, matching the repository's current
 *   config style and the existing third-party fallback pattern.
 * - `event` observes `session.created` / `session.error`.
 * - `chat.message` records the last user prompt per session as a safe
 *   fallback; before retrying, the plugin prefers a fresh read via
 *   `client.session.messages` so fallback does not depend only on the hook
 *   payload.
 * - Retries dispatch through `client.session.promptAsync` with the selected
 *   model in the request body as `{ providerID, modelID }`, plus `parts`,
 *   the preserved `agent`, and any message ID. Agent/model are never placed
 *   in the query; the query only preserves the plugin directory when the
 *   SDK requires it.
 *
 * Behavior:
 * - Distinct ordered chains keyed by agent name (see FALLBACK_CHAINS).
 * - `jd-fix-agent` has one configured backup, using the OpenAI Codex model
 *   `openai/gpt-6-astra`.
 * - Only transient failures are retried (rate limit / quota / overload /
 *   service unavailable / 5xx and equivalents). Validation, auth, and other
 *   non-transient errors are never retried.
 * - Retries advance through each chain in order, preserve the agent name,
 *   and allow at most (chain length - 1) retries per session.
 * - Duplicate concurrent retries for the same session are suppressed.
 * - The plugin's own retry model is tagged via `expectRetryModel` and
 *   compared in normalized form (string or `{ providerID, modelID }`), so
 *   it is never mistaken for a manual model switch. Genuine manual switches
 *   re-anchor (or park) the session instead of triggering fallback.
 * - Primary recovery is bounded: the plugin never switches a live session
 *   back to primary mid-run and never runs background timers. New sessions
 *   always start at the configured primary; a per-agent cooldown timestamp
 *   only records when the primary last failed for observability.
 */

// ---------------------------------------------------------------------------
// Ordered fallback chains (primary first). Keep model IDs exact.
// ---------------------------------------------------------------------------

const FALLBACK_CHAINS: Record<string, readonly string[]> = {
  'jd-judge-a': ['openai/gpt-5.6-sol', 'opencode-go/muse-spark-1.3', 'xai/grok-4.6'],
  'jd-judge-b': [
    'zai-coding-plan/glm-5.3',
    'opencode-go/glm-5.3',
    'deepseek/deepseek-v4-flash-vision-exp',
  ],
  'jd-fix-agent': ['opencode/claude-fable-5-1', 'openai/gpt-6-astra'],
}

// How long a primary failure is remembered for observability. There is no
// background recovery loop; this bound only documents recency of the last
// primary failure per agent. New sessions always start at the primary.
const PRIMARY_COOLDOWN_MS = 10 * 60 * 1000

// Upper bound for tracked sessions; oldest entries are evicted past this.
const MAX_TRACKED_SESSIONS = 200

// ---------------------------------------------------------------------------
// Minimal structural types (self-contained; no imports).
// ---------------------------------------------------------------------------

type AnyRecord = Record<string, any>

type SessionState = {
  agent: string
  /** Index into FALLBACK_CHAINS[agent] for the model currently in use. */
  index: number
  lastUserText?: string
  lastParts?: any[]
  lastMessageID?: string
  retryInFlight: boolean
  /** Normalized model ID used by our own in-flight retry; never a manual switch. */
  expectRetryModel: string | null
  exhausted: boolean
  createdAt: number
}

// ---------------------------------------------------------------------------
// Module-level state (per plugin instance).
// ---------------------------------------------------------------------------

const sessions = new Map<string, SessionState>()
const primaryFailedAt = new Map<string, number>()

function chainFor(agent: string | undefined | null): readonly string[] | undefined {
  if (!agent) return undefined
  return FALLBACK_CHAINS[agent]
}

function rememberPrimaryFailure(agent: string): void {
  primaryFailedAt.set(agent, Date.now())
}

function isPrimaryCoolingDown(agent: string): boolean {
  const at = primaryFailedAt.get(agent)
  if (!at) return false
  return Date.now() - at < PRIMARY_COOLDOWN_MS
}

function trackSession(id: string, state: SessionState): void {
  if (sessions.has(id)) {
    sessions.delete(id) // refresh insertion order
  }
  sessions.set(id, state)
  if (sessions.size > MAX_TRACKED_SESSIONS) {
    const oldest = sessions.keys().next().value as string | undefined
    if (oldest) sessions.delete(oldest)
  }
}

function forgetSession(id: string): void {
  sessions.delete(id)
}

// ---------------------------------------------------------------------------
// Model normalization.
//
// `chatInput.model` and event models may be either a combined string
// ("provider/model") or an object (`{ providerID, modelID }`). All chain
// comparisons use the normalized combined string; retries dispatch the
// split object form in the promptAsync body.
// ---------------------------------------------------------------------------

function normalizeModelId(value: any): string | undefined {
  if (typeof value === 'string' && value.length > 0) return value
  if (value && typeof value === 'object') {
    const providerID = value.providerID ?? value.providerId ?? value.provider ?? value.provider_id
    const modelID = value.modelID ?? value.modelId ?? value.model ?? value.model_id ?? value.id
    if (
      typeof providerID === 'string' &&
      providerID.length > 0 &&
      typeof modelID === 'string' &&
      modelID.length > 0
    ) {
      if (modelID.includes('/')) return modelID // already combined
      return `${providerID}/${modelID}`
    }
    // One level of nesting (e.g. `{ model: { providerID, modelID } }`).
    if (value.model && typeof value.model === 'object') {
      return normalizeModelId(value.model)
    }
  }
  return undefined
}

function splitModelId(combined: string): { providerID: string; modelID: string } {
  const slash = combined.indexOf('/')
  if (slash < 0) return { providerID: '', modelID: combined }
  return {
    providerID: combined.slice(0, slash),
    modelID: combined.slice(slash + 1),
  }
}

// ---------------------------------------------------------------------------
// Defensive extractors (tolerate minor shape differences in hook payloads,
// including nested `properties.info` / `info` session payloads).
// ---------------------------------------------------------------------------

function extractSessionId(value: AnyRecord | undefined | null): string | undefined {
  if (!value || typeof value !== 'object') return undefined
  const info = (value as AnyRecord).properties?.info ?? (value as AnyRecord).info
  const payloadInfo = (value as AnyRecord).payload?.info
  const candidates = [
    value.sessionID,
    value.sessionId,
    value.session_id,
    value.id,
    value.session?.id,
    value.properties?.sessionID,
    value.properties?.sessionId,
    value.properties?.session_id,
    value.properties?.id,
    info?.id,
    info?.sessionID,
    info?.sessionId,
    payloadInfo?.id,
    payloadInfo?.sessionID,
    payloadInfo?.sessionId,
    value.payload?.sessionID,
    value.payload?.sessionId,
    value.payload?.id,
  ]
  for (const c of candidates) {
    if (typeof c === 'string' && c.length > 0) return c
  }
  return undefined
}

function extractAgent(value: AnyRecord | undefined | null): string | undefined {
  if (!value || typeof value !== 'object') return undefined
  const info = (value as AnyRecord).properties?.info ?? (value as AnyRecord).info
  const payloadInfo = (value as AnyRecord).payload?.info
  const candidates = [
    value.agent,
    value.session?.agent,
    value.message?.agent,
    value.properties?.agent,
    info?.agent,
    payloadInfo?.agent,
    value.payload?.agent,
  ]
  for (const c of candidates) {
    if (typeof c === 'string' && c.length > 0) return c
  }
  return undefined
}

function extractModelId(value: AnyRecord | undefined | null): string | undefined {
  if (!value || typeof value !== 'object') return undefined
  const info = (value as AnyRecord).properties?.info ?? (value as AnyRecord).info
  const payloadInfo = (value as AnyRecord).payload?.info
  const candidates = [
    value.model,
    value.chatInput?.model,
    value.session?.model,
    value.message?.model,
    value.properties?.model,
    info?.model,
    payloadInfo?.model,
    value.payload?.model,
  ]
  for (const c of candidates) {
    const normalized = normalizeModelId(c)
    if (normalized) return normalized
  }
  return undefined
}

function extractErrorValue(event: AnyRecord): any {
  return (
    event.error ??
    event.properties?.error ??
    event.properties?.info?.error ??
    event.payload?.error ??
    event.info?.error ??
    event.data
  )
}

function extractErrorTextAndStatus(errorValue: any): { text: string; status?: number } {
  let status: number | undefined
  try {
    const direct =
      errorValue?.status ??
      errorValue?.statusCode ??
      errorValue?.code ??
      errorValue?.response?.status ??
      errorValue?.error?.status ??
      errorValue?.error?.statusCode
    if (typeof direct === 'number' && Number.isFinite(direct)) {
      status = direct
    } else if (typeof direct === 'string' && /^\d{3}$/.test(direct.trim())) {
      status = Number.parseInt(direct.trim(), 10)
    }
  } catch {
    // ignore extractor failures; fall through to text matching
  }

  let text = ''
  try {
    const parts = [
      errorValue?.message,
      errorValue?.error?.message,
      errorValue?.data?.message,
      errorValue?.response?.data?.message,
      typeof errorValue === 'string' ? errorValue : '',
    ]
    text = parts.filter((p) => typeof p === 'string' && p.length > 0).join(' | ')
    if (!text) {
      text = JSON.stringify(errorValue ?? '').slice(0, 2000)
    }
  } catch {
    try {
      text = String(errorValue ?? '')
    } catch {
      text = ''
    }
  }
  return { text, status }
}

// ---------------------------------------------------------------------------
// Transient-only classifier.
// Retries: 429 / 502 / 503 / 504 / other 5xx, rate limit, quota, overload,
// service/temporarily unavailable, timeouts, connection resets, provider
// capacity errors. Never retries: 400 / 401 / 403 / 404 / 405 / 422 and
// validation / auth / permission / not-found signals.
// ---------------------------------------------------------------------------

const TRANSIENT_PATTERNS: RegExp[] = [
  /rate.?limit/i,
  /\bquota\b/i,
  /overload/i,
  /overloaded/i,
  /capacity/i,
  /service.?unavailable/i,
  /temporarily.?unavailable/i,
  /server.?error/i,
  /try.?again/i,
  /timed?.?out/i,
  /timeout/i,
  /ECONNRESET/i,
  /ETIMEDOUT/i,
  /EAI_AGAIN/i,
  /fetch.?failed/i,
  /provider.?unavailable/i,
  /bad.?gateway/i,
  /gateway.?timeout/i,
  /\b429\b/,
  /\b502\b/,
  /\b503\b/,
  /\b504\b/,
  /\b529\b/,
  /\b5\d\d\b/,
]

const NON_TRANSIENT_PATTERNS: RegExp[] = [
  /invalid.?api.?key/i,
  /unauthoriz/i,
  /forbidden/i,
  /permission.?denied/i,
  /authentication/i,
  /invalid.?request/i,
  /validation/i,
  /bad.?request/i,
  /model.?not.?found/i,
  /\bnot.?found\b/i,
  /\b400\b/,
  /\b401\b/,
  /\b403\b/,
  /\b404\b/,
  /\b405\b/,
  /\b422\b/,
]

const TRANSIENT_STATUS = new Set([408, 425, 429, 502, 503, 504, 529])
const NON_TRANSIENT_STATUS = new Set([400, 401, 402, 403, 404, 405, 422])

function isTransientFailure(errorValue: any): boolean {
  const { text, status } = extractErrorTextAndStatus(errorValue)
  if (typeof status === 'number') {
    if (NON_TRANSIENT_STATUS.has(status)) return false
    if (TRANSIENT_STATUS.has(status)) return true
    if (status >= 500 && status < 600) return true
    if (status >= 400 && status < 500) return false
  }
  // Explicit deny first so auth/validation text never falls through.
  for (const re of NON_TRANSIENT_PATTERNS) {
    if (re.test(text)) return false
  }
  for (const re of TRANSIENT_PATTERNS) {
    if (re.test(text)) return true
  }
  return false
}

// ---------------------------------------------------------------------------
// User-prompt extraction (user text only), shared by the chat hook cache and
// the pre-retry refresh from `client.session.messages`.
// ---------------------------------------------------------------------------

type CachedPrompt = { text?: string; parts?: any[]; messageID?: string }

function coerceMessageRecord(raw: any): AnyRecord | undefined {
  if (!raw || typeof raw !== 'object') return undefined
  // SDK list rows may wrap the record under `info` or `message`.
  const record = raw.info ?? raw.message ?? raw
  if (!record || typeof record !== 'object') return undefined
  return record as AnyRecord
}

function readPromptFromRecord(record: AnyRecord): CachedPrompt {
  const role = record.role
  if (role && role !== 'user' && role !== 'human') return {}
  const parts = Array.isArray(record.parts) ? record.parts : undefined
  const messageID =
    (typeof record.id === 'string' && record.id) ||
    (typeof record.messageID === 'string' && record.messageID) ||
    (typeof record.messageId === 'string' && record.messageId) ||
    undefined
  if (parts && parts.length > 0) {
    const text = parts
      .filter((p: any) => p && (p.type === 'text' || typeof p?.text === 'string'))
      .map((p: any) => (typeof p?.text === 'string' ? p.text : ''))
      .filter(Boolean)
      .join('\n')
    return { text: text || undefined, parts, messageID }
  }
  const text =
    (typeof record.text === 'string' && record.text) ||
    (typeof record.content === 'string' && record.content) ||
    undefined
  return text || messageID ? { text, messageID } : {}
}

function extractUserPrompt(input: AnyRecord): CachedPrompt {
  const record = coerceMessageRecord(input?.message ?? input?.payload?.message ?? input)
  if (!record) return {}
  const prompt = readPromptFromRecord(record)
  // Message IDs sometimes live on the wrapper rather than the record.
  if (!prompt.messageID) {
    const wrapperID =
      (typeof input?.messageID === 'string' && input.messageID) ||
      (typeof input?.messageId === 'string' && input.messageId) ||
      (typeof input?.message?.id === 'string' && input.message.id) ||
      undefined
    if (wrapperID) return { ...prompt, messageID: wrapperID }
  }
  return prompt
}

// ---------------------------------------------------------------------------
// Plugin export (legacy hook-object API).
// ---------------------------------------------------------------------------

export default async function judgmentDayModelFallback(context: AnyRecord): Promise<AnyRecord> {
  const client = context?.client
  const directory = context?.directory ?? context?.worktree ?? undefined
  const directoryQuery = typeof directory === 'string' && directory ? { directory } : {}

  async function loadLatestUserPrompt(
    sessionID: string,
    fallback: CachedPrompt
  ): Promise<CachedPrompt> {
    // Prefer a fresh read so fallback replays exactly what the judge was
    // asked, even if the chat hook payload was missed or truncated. The
    // hook-cached prompt remains the safe fallback when the fetch is
    // unavailable or fails.
    try {
      const fetcher = client?.session?.messages
      if (typeof fetcher === 'function') {
        const response = await fetcher({
          path: { id: sessionID },
          query: { ...directoryQuery },
        })
        const list = Array.isArray(response)
          ? response
          : Array.isArray(response?.data)
            ? response.data
            : Array.isArray(response?.messages)
              ? response.messages
              : Array.isArray(response?.items)
                ? response.items
                : undefined
        if (list) {
          for (let i = list.length - 1; i >= 0; i--) {
            const record = coerceMessageRecord(list[i])
            if (!record) continue
            const prompt = readPromptFromRecord(record)
            if (prompt.text || prompt.parts) return prompt
          }
        }
      }
    } catch {
      // fall through to the hook-cached prompt
    }
    return fallback
  }

  async function retryWithNextModel(sessionID: string): Promise<void> {
    const state = sessions.get(sessionID)
    if (!state || !client) return
    if (state.retryInFlight || state.exhausted) return
    const chain = chainFor(state.agent)
    if (!chain) return
    const nextIndex = state.index + 1
    if (nextIndex >= chain.length) {
      state.exhausted = true
      return
    }
    const prompt = await loadLatestUserPrompt(sessionID, {
      text: state.lastUserText,
      parts: state.lastParts,
      messageID: state.lastMessageID,
    })
    const parts = prompt.parts ?? (prompt.text ? [{ type: 'text', text: prompt.text }] : undefined)
    if (!parts || parts.length === 0) return // nothing to replay
    const nextModel = chain[nextIndex]

    // Single-flight guard: concurrent error events for the same session
    // collapse into one retry.
    state.retryInFlight = true
    state.expectRetryModel = nextModel
    try {
      // Body-based dispatch, matching the existing third-party fallback
      // pattern: the session path identifies the judge session; the body
      // carries the preserved agent, the split model object, the replayed
      // parts, and the originating message ID when known. Agent/model are
      // intentionally NOT placed in the query.
      const body: AnyRecord = {
        agent: state.agent,
        model: splitModelId(nextModel),
        parts,
      }
      const messageID = prompt.messageID ?? state.lastMessageID
      if (typeof messageID === 'string' && messageID.length > 0) {
        body.messageID = messageID
      }
      await client.session.promptAsync({
        path: { id: sessionID },
        query: { ...directoryQuery },
        body,
      })
      if (state.index === 0) rememberPrimaryFailure(state.agent)
      state.index = nextIndex
      // Refresh the cache from what was actually replayed.
      if (prompt.text) state.lastUserText = prompt.text
      if (prompt.parts) state.lastParts = prompt.parts
      if (prompt.messageID) state.lastMessageID = prompt.messageID
      if (state.index >= chain.length - 1) {
        // Last model in use; further transient errors must surface.
        state.exhausted = true
      }
    } catch {
      // Retry dispatch itself failed: clear the expectation so a later
      // genuine message is not misread, and release the single-flight lock
      // so a subsequent error event may try the same next model once.
      state.expectRetryModel = null
    } finally {
      state.retryInFlight = false
    }
  }

  return {
    event: async (input: AnyRecord): Promise<void> => {
      const event = (input?.event ?? input) as AnyRecord
      if (!event || typeof event !== 'object') return
      const type: string = event.type ?? event.name ?? ''

      if (type === 'session.created') {
        const sessionID = extractSessionId(event) ?? extractSessionId(input as AnyRecord)
        const agent = extractAgent(event) ?? extractAgent(input as AnyRecord)
        const modelId = extractModelId(event) ?? extractModelId(input as AnyRecord)
        if (!sessionID || !agent) return
        if (!chainFor(agent)) return // jd-fix-agent and others: no tracking
        const chain = chainFor(agent) as readonly string[]
        const index = modelId ? chain.indexOf(modelId) : 0
        trackSession(sessionID, {
          agent,
          index: index >= 0 ? index : 0,
          retryInFlight: false,
          expectRetryModel: null,
          exhausted: false,
          createdAt: Date.now(),
        })
        return
      }

      if (type === 'session.error' || type === 'session.session.error') {
        const sessionID = extractSessionId(event) ?? extractSessionId(input as AnyRecord)
        if (!sessionID) return
        const state = sessions.get(sessionID)
        if (!state) return
        if (!chainFor(state.agent)) return
        const errorValue = extractErrorValue(event)
        if (!isTransientFailure(errorValue)) return // never retry validation/auth
        if (state.retryInFlight || state.exhausted) return // single-flight / terminal
        if (state.index + 1 >= (chainFor(state.agent) as readonly string[]).length) {
          state.exhausted = true
          return
        }
        await retryWithNextModel(sessionID)
        return
      }

      if (type === 'session.deleted' || type === 'session.closed') {
        const sessionID = extractSessionId(event) ?? extractSessionId(input as AnyRecord)
        if (sessionID) forgetSession(sessionID)
        return
      }
    },

    'chat.message': async (input: AnyRecord, _output: AnyRecord): Promise<void> => {
      if (!input || typeof input !== 'object') return
      const sessionID = extractSessionId(input)
      if (!sessionID) return
      const agent = extractAgent(input)
      const modelId = extractModelId(input)

      // Lazily track judge sessions first seen here (e.g. if the
      // session.created event was missed); never track jd-fix-agent.
      let state = sessions.get(sessionID)
      if (!state) {
        if (!agent || !chainFor(agent)) return
        const chain = chainFor(agent) as readonly string[]
        const index = modelId ? chain.indexOf(modelId) : 0
        state = {
          agent,
          index: index >= 0 ? index : 0,
          retryInFlight: false,
          expectRetryModel: null,
          exhausted: false,
          createdAt: Date.now(),
        }
        trackSession(sessionID, state)
      }

      // Our own retry prompt (string or `{ providerID, modelID }` form):
      // consume the expectation, keep the advanced index, and record the
      // replayed prompt for potential further fallback. Never re-anchor it
      // as a manual switch.
      if (modelId && state.expectRetryModel && modelId === state.expectRetryModel) {
        state.expectRetryModel = null
        const replayed = extractUserPrompt(input)
        if (replayed.text) state.lastUserText = replayed.text
        if (replayed.parts) state.lastParts = replayed.parts
        if (replayed.messageID) state.lastMessageID = replayed.messageID
        return
      }

      // Genuine manual model switch: re-anchor when the normalized model
      // belongs to the agent's chain; park (no auto-retry) when it does not.
      // This keeps a human override from triggering automatic fallback. The
      // branch above guarantees our own retry never reaches this logic.
      if (modelId) {
        const chain = chainFor(state.agent) as readonly string[]
        const manualIndex = chain.indexOf(modelId)
        if (manualIndex >= 0 && manualIndex !== state.index && !state.retryInFlight) {
          state.index = manualIndex
          state.exhausted = manualIndex >= chain.length - 1
        } else if (manualIndex < 0 && !state.retryInFlight) {
          state.exhausted = true
        }
      }

      const prompt = extractUserPrompt(input)
      if (prompt.text) state.lastUserText = prompt.text
      if (prompt.parts) state.lastParts = prompt.parts
      if (prompt.messageID) state.lastMessageID = prompt.messageID

      // Documented cooldown note: when the primary recently failed, a brand
      // new session still starts at the primary (per opencode.json) and will
      // simply fail over again if the outage persists. No mid-session
      // switch-back is ever attempted; recovery is the next session after
      // the cooldown elapses.
      void isPrimaryCoolingDown
    },
  }
}
