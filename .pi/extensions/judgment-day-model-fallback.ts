/**
 * Judgment Day model fallback — project-local Pi extension.
 *
 * Installs ordered provider/model fallback chains around the primary routes
 * used by Judgment Day subagents. When a primary attempt fails with a
 * transient upstream error (rate limit, quota, overload, service
 * unavailable, timeout, network, 5xx), the next candidate model is tried.
 * Non-transient failures (auth, validation, billing/payment, unsupported
 * model, context overflow, cancellation, unknown errors) fail immediately.
 *
 * Constraints implemented here:
 * - No external runtime imports: the only import below is type-only, so
 *   module resolution never depends on globally installed packages.
 * - Scope-aware installation: Judgment Day chains are installed only in
 *   Gentle Agents child sessions; the orchestrator chain is installed only in
 *   non-child sessions and matches one exact parent model route.
 * - Only the primary provider/model routes are wrapped. Every other model
 *   keeps the stock provider pipeline via pass-through to the provider
 *   captured before the override was installed.
 * - Every attempt's events are buffered; the caller sees either the
 *   successful attempt's events (verbatim, so the assistant message keeps
 *   the winning candidate's provider/model identity) or exactly one
 *   terminal failure event. Partial events of failed attempts never leak.
 * - Candidate attempts are resolved through the Pi model registry so each
 *   provider's own authentication is used; the primary attempt reuses the
 *   request options already prepared by the runtime.
 * - No timers and no background recovery: exactly one async driver per
 *   stream, driven by stream consumption.
 * - Idempotent installation: wrappers are tagged and re-checked before
 *   registering, so /reload or repeated session_start events never
 *   double-wrap a provider.
 */

import type { ExtensionAPI } from '@earendil-works/pi-coding-agent'

declare const process: {
  env: Record<string, string | undefined>
}

// ---------------------------------------------------------------------------
// Minimal structural types (kept local so no runtime values are imported)
// ---------------------------------------------------------------------------

interface ModelLike {
  id: string
  provider: string
  api: string
  [key: string]: unknown
}

interface AssistantMessageLike {
  role: 'assistant'
  content: unknown[]
  api: string
  provider: string
  model: string
  usage: {
    input: number
    output: number
    cacheRead: number
    cacheWrite: number
    totalTokens: number
    cost: {
      input: number
      output: number
      cacheRead: number
      cacheWrite: number
      total: number
    }
  }
  stopReason: string
  errorMessage?: string
  timestamp: number
}

type StreamEvent =
  | { type: 'start'; partial: AssistantMessageLike }
  | { type: string; [key: string]: unknown }
  | { type: 'done'; reason: string; message: AssistantMessageLike }
  | { type: 'error'; reason: 'aborted' | 'error'; error: AssistantMessageLike }

interface EventStreamLike extends AsyncIterable<StreamEvent> {
  result?: () => Promise<unknown>
}

interface StreamOptionsLike {
  signal?: { readonly aborted?: unknown }
  apiKey?: string
  headers?: unknown
  env?: unknown
  [key: string]: unknown
}

interface ProviderLike {
  id?: string
  streamSimple?: (
    model: ModelLike,
    context: unknown,
    options?: StreamOptionsLike
  ) => EventStreamLike
  [key: string]: unknown
}

interface RegistryLike {
  getProvider?: (provider: string) => ProviderLike | undefined
  find?: (provider: string, modelId: string) => ModelLike | undefined
  streamSimple?: (
    model: ModelLike,
    context: unknown,
    options?: StreamOptionsLike
  ) => EventStreamLike
  getRegisteredProviderConfig?: (
    provider: string
  ) => { streamSimple?: { [key: string]: unknown } } | undefined
}

// ---------------------------------------------------------------------------
// Fallback routes (exact ordered chains; missing candidates are skipped or aliased)
// ---------------------------------------------------------------------------

interface CandidateRoute {
  readonly provider: string
  readonly model: string
}

type FallbackScope = 'child' | 'parent'

interface FallbackChain {
  /** Session scope in which this chain may be installed. */
  readonly scope: FallbackScope
  /** Provider whose primary route this chain wraps. */
  readonly provider: string
  /** Model id of the primary route. */
  readonly primary: string
  /** API family of the primary route; the override only applies to it. */
  readonly api: string
  /** Ordered candidates, starting with the primary route itself. */
  readonly candidates: readonly CandidateRoute[]
}

const FALLBACK_CHAINS: readonly FallbackChain[] = [
  {
    scope: 'parent',
    provider: 'openai-codex',
    primary: 'gpt-5.6-luna',
    api: 'openai-codex-responses',
    candidates: [
      { provider: 'openai-codex', model: 'gpt-5.6-luna' },
      { provider: 'xai', model: 'grok-4.7' },
      { provider: 'zai', model: 'glm-5.3' },
    ],
  },
  {
    scope: 'child',
    provider: 'openai-codex',
    primary: 'gpt-5.6-sol',
    api: 'openai-codex-responses',
    candidates: [
      { provider: 'openai-codex', model: 'gpt-5.6-sol' },
      { provider: 'opencode', model: 'muse-spark-1.3' },
      { provider: 'xai', model: 'grok-4.6' },
    ],
  },
  {
    scope: 'child',
    provider: 'zai',
    primary: 'glm-5.3',
    api: 'openai-completions',
    candidates: [
      { provider: 'zai', model: 'glm-5.3' },
      { provider: 'opencode-go', model: 'glm-5.3' },
      { provider: 'opencode-go', model: 'deepseek-v4-flash-vision-exp' },
    ],
  },
  {
    scope: 'child',
    provider: 'opencode',
    primary: 'claude-fable-5-1',
    api: 'anthropic-messages',
    candidates: [
      { provider: 'opencode', model: 'claude-fable-5-1' },
      { provider: 'openai-codex', model: 'gpt-6-astra' },
    ],
  },
]

const routeKey = (provider: string, model: string): string => `${provider}/${model}`

const PRIMARY_ROUTES = new Map<string, FallbackChain>(
  FALLBACK_CHAINS.map((chain) => [routeKey(chain.provider, chain.primary), chain])
)

/**
 * xAI documents grok-4.7, but this Pi catalog currently stops at grok-4.6.
 * Use the known model metadata only as a dispatch template until the catalog
 * catches up; do not replace or mutate Pi's built-in xAI model list.
 */
const FALLBACK_MODEL_ALIASES = new Map<string, CandidateRoute>([
  [routeKey('xai', 'grok-4.7'), { provider: 'xai', model: 'grok-4.6' }],
])

function resolveCandidateModel(
  registry: RegistryLike,
  candidate: CandidateRoute
): ModelLike | undefined {
  const found = registry.find?.(candidate.provider, candidate.model)
  if (found) return found

  const alias = FALLBACK_MODEL_ALIASES.get(routeKey(candidate.provider, candidate.model))
  if (!alias) return undefined
  const template = registry.find?.(alias.provider, alias.model)
  if (!template) return undefined
  return {
    ...template,
    provider: candidate.provider,
    id: candidate.model,
    name: candidate.model,
  }
}

// ---------------------------------------------------------------------------
// Failure classification
// ---------------------------------------------------------------------------

/**
 * Failures that never advance to the next candidate. Checked before the
 * transient list so a billing-flavored quota error stays terminal.
 */
const TERMINAL_PATTERNS: readonly RegExp[] = [
  // Cancellation / abort
  /\babort(?:ed|ing)?\b/i,
  /\bcancell?ed\b/i,
  // Authentication / authorization
  /\b(?:401|403)\b/,
  /unauthorized/i,
  /forbidden/i,
  /invalid\s+(?:api[-\s]?key|token|credential|bearer)/i,
  /(?:api[-\s]?key|token|credential)s?\s+(?:invalid|expired|missing|revoked)/i,
  /authenticat/i,
  /\boauth\b/i,
  /permission\s+denied/i,
  // Billing / payment (terminal even when quota-flavored)
  /\b402\b/,
  /billing/i,
  /payment/i,
  /insufficient\s+(?:credits?|funds)/i,
  /credit\s+balance/i,
  // Request validation
  /\b(?:400|422)\b/,
  /invalid[._\s-]?request/i,
  /bad\s+request/i,
  /\bvalidation\b/i,
  /invalid\s+(?:parameter|argument|value|messages?|tool)/i,
  /unprocessable/i,
  // Unsupported / unknown model
  /\b404\b/,
  /model[_\s-]*(?:not\s+found|does\s+not\s+exist|not\s+available|unsupported)/i,
  /no\s+such\s+model/i,
  /unknown\s+model/i,
  /unsupported[._\s-]?model/i,
  /model_not_found/i,
  // Context overflow (aligned with pi-ai's overflow detection phrases)
  /context[_\s]?length[_\s]?exceeded/i,
  /prompt is too long/i,
  /request_too_large/i,
  /exceeds the context window/i,
  /maximum context length/i,
  /maximum allowed input length/i,
  /token limit exceeded/i,
  /too many tokens/i,
  /context window/i,
  /reduce the length of the messages/i,
]

/** Failures that advance to the next candidate. */
const TRANSIENT_PATTERNS: readonly RegExp[] = [
  // Rate limiting / throttling
  /\b429\b/,
  /rate[._\s-]?limit/i,
  /too many requests/i,
  /throttl/i,
  /backoff/i,
  // Quota exhaustion (billing-flavored quota is caught above)
  /\bquota\b/i,
  // Overload / capacity
  /over[._\s-]?load/i,
  /over[._\s-]?capacity/i,
  /at\s+capacity/i,
  /capacity\s+(?:limit|exceeded)/i,
  /load\s+shedding/i,
  // Service availability / 5xx
  /\b50[0234]\b/,
  /internal server error/i,
  /server (?:had an )?error/i,
  /server_error/i,
  /bad gateway/i,
  /gateway\s+time[-\s]?out/i,
  /service\s+unavailable/i,
  /temporarily unavailable/i,
  /currently unavailable/i,
  /try again (?:later|soon)/i,
  // Timeouts
  /\betimeout\b/i,
  /time[-\s]?out/i,
  /timed\s+out/i,
  // Network transport
  /\b(?:econnreset|econnrefused|ehostunreach|enetunreach|epipe|eai_again|enotfound)\b/i,
  /socket hang up/i,
  /network(?:\s+error|\s+request\s+failed)?/i,
  /fetch failed/i,
  /connection\s+(?:reset|refused|closed|error|timed out)/i,
]

const messageOf = (error: unknown): string =>
  error instanceof Error ? error.message : String(error ?? '')

/** True when the failure may advance to the next candidate. */
function isTransientFailure(message: string): boolean {
  const text = message ?? ''
  if (text.length === 0) return false
  for (const pattern of TERMINAL_PATTERNS) {
    if (pattern.test(text)) return false
  }
  for (const pattern of TRANSIENT_PATTERNS) {
    if (pattern.test(text)) return true
  }
  // Arbitrary unknown errors never advance.
  return false
}

// ---------------------------------------------------------------------------
// Local async event-stream adapter (mirrors pi-ai's EventStream semantics)
// ---------------------------------------------------------------------------

class BufferedEventStream implements AsyncIterable<StreamEvent> {
  private queue: StreamEvent[] = []
  private waiters: Array<(result: IteratorResult<StreamEvent>) => void> = []
  private finished = false
  private resultResolved = false
  private readonly finalResult: Promise<unknown>
  private resolveFinalResult!: (value: unknown) => void

  constructor() {
    this.finalResult = new Promise((resolve) => {
      this.resolveFinalResult = resolve
    })
  }

  private resolveResultOnce(value: unknown): void {
    if (this.resultResolved) return
    this.resultResolved = true
    this.resolveFinalResult(value)
  }

  push(event: StreamEvent): void {
    if (this.finished) return
    if (event.type === 'done' || event.type === 'error') {
      this.finished = true
      const terminal = event as
        | { type: 'done'; message: AssistantMessageLike }
        | { type: 'error'; error: AssistantMessageLike }
      this.resolveResultOnce(terminal.type === 'done' ? terminal.message : terminal.error)
    }
    const waiter = this.waiters.shift()
    if (waiter) {
      waiter({ value: event, done: false })
    } else {
      this.queue.push(event)
    }
  }

  end(): void {
    this.finished = true
    // Guarantee result() settles even on a defensive path with no terminal
    // event, so a consumer mirroring pi-ai's forwardStream never hangs.
    this.resolveResultOnce(undefined)
    while (this.waiters.length > 0) {
      this.waiters.shift()!({ value: undefined, done: true })
    }
  }

  result(): Promise<unknown> {
    return this.finalResult
  }

  [Symbol.asyncIterator](): AsyncIterator<StreamEvent> {
    return {
      next: (): Promise<IteratorResult<StreamEvent>> => {
        const queued = this.queue.shift()
        if (queued !== undefined) {
          return Promise.resolve({ value: queued, done: false })
        }
        if (this.finished) {
          return Promise.resolve({ value: undefined, done: true })
        }
        return new Promise((resolve) => {
          this.waiters.push(resolve)
        })
      },
    }
  }
}

// ---------------------------------------------------------------------------
// Fallback driver
// ---------------------------------------------------------------------------

interface AttemptFailure {
  readonly provider: string
  readonly model: string
  readonly message: string
  readonly aborted: boolean
}

function synthErrorMessage(
  model: ModelLike,
  message: string,
  aborted: boolean
): AssistantMessageLike {
  return {
    role: 'assistant',
    content: [],
    api: model.api,
    provider: model.provider,
    model: model.id,
    usage: {
      input: 0,
      output: 0,
      cacheRead: 0,
      cacheWrite: 0,
      totalTokens: 0,
      cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
    },
    stopReason: aborted ? 'aborted' : 'error',
    errorMessage: message,
    timestamp: Date.now(),
  }
}

/** Push one terminal error event and close the stream. */
function emitFailure(
  out: BufferedEventStream,
  model: ModelLike,
  message: string,
  aborted: boolean
): void {
  const error = synthErrorMessage(model, message, aborted)
  out.push({ type: 'error', reason: aborted ? 'aborted' : 'error', error })
  out.end()
}

/**
 * Strip provider-bound auth from the prepared options so the registry
 * re-resolves authentication for the candidate's own provider. Neutral
 * options (signal, reasoning, sampling, retries) are preserved.
 */
function candidateOptions(options: StreamOptionsLike | undefined): StreamOptionsLike | undefined {
  if (!options) return undefined
  const next: StreamOptionsLike = { ...options }
  delete next.apiKey
  delete next.headers
  delete next.env
  return next
}

/** Transparently forward a non-primary model through the original provider. */
async function forwardPassThrough(
  out: BufferedEventStream,
  original: ProviderLike,
  model: ModelLike,
  context: unknown,
  options: StreamOptionsLike | undefined
): Promise<void> {
  try {
    const source = original.streamSimple!(model, context, options)
    try {
      for await (const event of source) {
        out.push(event)
      }
      out.end()
    } catch (error) {
      emitFailure(out, model, messageOf(error), options?.signal?.aborted === true)
    }
  } catch (error) {
    // Synchronous stream setup error: surface it without crashing the child.
    emitFailure(out, model, messageOf(error), options?.signal?.aborted === true)
  }
}

/** Run the ordered chain for a primary route, buffering each attempt. */
async function driveFallback(
  out: BufferedEventStream,
  chain: FallbackChain,
  registry: RegistryLike,
  original: ProviderLike,
  model: ModelLike,
  context: unknown,
  options: StreamOptionsLike | undefined
): Promise<void> {
  let lastFailure: AttemptFailure | undefined
  const missingCandidates: string[] = []

  for (let index = 0; index < chain.candidates.length; index += 1) {
    const candidate = chain.candidates[index]
    if (options?.signal?.aborted === true) {
      const failure = lastFailure ?? {
        provider: model.provider,
        model: model.id,
        message: 'aborted before attempt',
        aborted: true,
      }
      emitFailure(
        out,
        index === 0 ? model : { ...model, provider: candidate.provider, id: candidate.model },
        `[judgment-day-fallback] request aborted on route ${routeKey(chain.provider, chain.primary)}; last failure from ${failure.provider}/${failure.model}: ${failure.message}`,
        true
      )
      return
    }

    // The primary attempt reuses the prepared model and options: the
    // runtime already resolved this provider's auth into them. Candidates
    // are resolved through the registry so their own provider auth is used.
    let attemptModel: ModelLike = model
    let attemptOptions = options
    let attemptStream: EventStreamLike
    try {
      if (index === 0) {
        attemptStream = original.streamSimple!(model, context, options)
      } else {
        const found = resolveCandidateModel(registry, candidate)
        if (!found) {
          // Missing candidate model: skip it without failing the chain.
          missingCandidates.push(routeKey(candidate.provider, candidate.model))
          continue
        }
        attemptModel = found
        attemptOptions = candidateOptions(options)
        attemptStream = registry.streamSimple!(found, context, attemptOptions)
      }
    } catch (error) {
      const message = messageOf(error)
      lastFailure = {
        provider: attemptModel.provider,
        model: attemptModel.id,
        message,
        aborted: options?.signal?.aborted === true,
      }
      if (!isTransientFailure(message)) {
        emitFailure(
          out,
          attemptModel,
          `[judgment-day-fallback] non-transient setup failure on ${attemptModel.provider}/${attemptModel.id}: ${message}`,
          lastFailure.aborted
        )
        return
      }
      continue
    }

    // Buffer the attempt: on transient failure nothing leaks and the next
    // candidate starts; on success the buffer replays verbatim so the
    // assistant message keeps the winning candidate's identity.
    const buffered: StreamEvent[] = []
    let transientFailure = false
    try {
      for await (const event of attemptStream) {
        if (event.type === 'error') {
          const error = (event.error ?? {}) as AssistantMessageLike
          const message = error.errorMessage ?? ''
          const aborted = event.reason === 'aborted' || options?.signal?.aborted === true
          lastFailure = {
            provider: attemptModel.provider,
            model: attemptModel.id,
            message,
            aborted,
          }
          if (aborted || !isTransientFailure(message)) {
            // Terminal failure: emit exactly one failure event, keeping
            // the original error text (pi's overflow detection reads it)
            // and the failing candidate's identity.
            const terminal =
              event.error !== undefined
                ? event
                : {
                    type: 'error' as const,
                    reason: aborted ? ('aborted' as const) : ('error' as const),
                    error: synthErrorMessage(attemptModel, message, aborted),
                  }
            out.push(terminal)
            out.end()
            return
          }
          transientFailure = true
          break // transient: discard the buffer and advance
        }
        if (event.type === 'done') {
          for (const bufferedEvent of buffered) {
            out.push(bufferedEvent)
          }
          out.push(event)
          out.end()
          return
        }
        buffered.push(event)
      }
      if (!transientFailure) {
        // The stream ended without a done/error event. That is a protocol
        // violation, not a transient condition: fail terminally.
        emitFailure(
          out,
          attemptModel,
          `[judgment-day-fallback] stream from ${attemptModel.provider}/${attemptModel.id} ended without a terminal event`,
          options?.signal?.aborted === true
        )
        return
      }
    } catch (error) {
      const message = messageOf(error)
      lastFailure = {
        provider: attemptModel.provider,
        model: attemptModel.id,
        message,
        aborted: options?.signal?.aborted === true,
      }
      if (!isTransientFailure(message)) {
        emitFailure(
          out,
          attemptModel,
          `[judgment-day-fallback] non-transient failure on ${attemptModel.provider}/${attemptModel.id}: ${message}`,
          lastFailure.aborted
        )
        return
      }
    }
  }

  // Every candidate failed transiently (or was missing): report the last
  // observed failure as the terminal outcome for the whole chain.
  const tail = lastFailure ?? {
    provider: model.provider,
    model: model.id,
    message:
      missingCandidates.length > 0
        ? `no registered fallback candidate was available: ${missingCandidates.join(', ')}`
        : 'no attempt produced a failure report',
    aborted: false,
  }
  emitFailure(
    out,
    model,
    `[judgment-day-fallback] exhausted ${chain.candidates.length} candidates for route ${routeKey(chain.provider, chain.primary)}; last failure from ${tail.provider}/${tail.model}: ${tail.message}`,
    tail.aborted
  )
}

// ---------------------------------------------------------------------------
// Provider wrapping and installation
// ---------------------------------------------------------------------------

const WRAPPER_TAG = '__judgmentDayModelFallback'

function makeWrapper(
  original: ProviderLike,
  registry: RegistryLike
): (model: ModelLike, context: unknown, options?: StreamOptionsLike) => BufferedEventStream {
  const wrapper = (
    model: ModelLike,
    context: unknown,
    options?: StreamOptionsLike
  ): BufferedEventStream => {
    const out = new BufferedEventStream()
    const route = PRIMARY_ROUTES.get(routeKey(model.provider, model.id))
    if (!route) {
      // Not a wrapped primary route: transparent pass-through.
      void forwardPassThrough(out, original, model, context, options).catch(() => {
        emitFailure(out, model, 'pass-through driver failed', false)
      })
      return out
    }
    void driveFallback(out, route, registry, original, model, context, options).catch(
      (error: unknown) => {
        // Defensive: never let a driver bug crash the child session.
        emitFailure(
          out,
          model,
          `[judgment-day-fallback] driver failed: ${messageOf(error)}`,
          options?.signal?.aborted === true
        )
      }
    )
    return out
  }
  // Tag for idempotency checks across /reload boundaries.
  ;(wrapper as unknown as Record<string, unknown>)[WRAPPER_TAG] = true
  return wrapper
}

function isAlreadyInstalled(registry: RegistryLike, provider: string): boolean {
  const config = registry.getRegisteredProviderConfig?.(provider)
  const streamSimple = config?.streamSimple as Record<string, unknown> | undefined
  return streamSimple?.[WRAPPER_TAG] === true
}

/**
 * Capture the pristine provider and register the wrapper. Runs on
 * session_start: factory-time registrations flush before this event, so the
 * registry still exposes the un-wrapped provider here, while registrations
 * from an event handler apply immediately without a /reload.
 */
function installFallbacks(pi: ExtensionAPI, registry: RegistryLike): void {
  const scope: FallbackScope = process.env.GENTLE_PI_AGENTS_CHILD === '1' ? 'child' : 'parent'
  if (
    typeof registry.getProvider !== 'function' ||
    typeof registry.find !== 'function' ||
    typeof registry.streamSimple !== 'function'
  ) {
    return
  }
  for (const chain of FALLBACK_CHAINS) {
    if (chain.scope !== scope || isAlreadyInstalled(registry, chain.provider)) continue
    const original = registry.getProvider(chain.provider)
    if (!original || typeof original.streamSimple !== 'function') {
      // Provider absent from this Pi install: leave it untouched.
      continue
    }
    try {
      pi.registerProvider(chain.provider, {
        api: chain.api,
        streamSimple: makeWrapper(original, registry),
      } as unknown as Parameters<ExtensionAPI['registerProvider']>[1])
    } catch {
      // A conflicting registration from another extension loses the race;
      // the stock provider behavior stays active for this chain.
    }
  }
}

// ---------------------------------------------------------------------------
// Extension entry point
// ---------------------------------------------------------------------------

export default function judgmentDayModelFallback(pi: ExtensionAPI): void {
  pi.on('session_start', (_event, ctx) => {
    const registry = ctx.modelRegistry as unknown as RegistryLike | undefined
    if (!registry) return
    try {
      installFallbacks(pi, registry)
    } catch {
      // Installation failures must not break the child session startup.
    }
  })
}
