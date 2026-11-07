import { key } from './relation-graph'

export interface TextRange {
  start: number
  end: number
}
export interface LabeledRange extends TextRange {
  role: 'subject' | 'predicate' | 'object'
  /** False when only some of the part's words are in the text (rebuilt by the model). */
  literal?: boolean
}
type Role = LabeledRange['role']
/** How a relation part shows up in the passage: as written, only some words, or not at all. */
/** `elsewhere`: not in the passage's sentence but found in the same document. */
export type PartEvidence = 'literal' | 'partial' | 'elsewhere' | 'missing'
export interface OccurrenceContext {
  collectionId: string
  collectionName: string
  itemTitle: string
}
interface TargetAddress extends OccurrenceContext {
  itemId: string
  assetId: string | null
}
export type OccurrenceTarget = TargetAddress &
  (
    | { kind: 'entity'; value: string; offsets?: TextRange[] }
    | { kind: 'relation'; subject: string; predicate: string; object: string }
  )
export interface OccurrenceSession {
  kind: 'entity' | 'relation'
  targets: OccurrenceTarget[]
  index: number
  /** Match within the current target, including its visible assets for item-level rows. */
  matchIndex?: number
}
export interface OccurrencePassage extends TextRange {
  ranges: LabeledRange[]
  approximate: boolean
  /** Relations only: which parts the passage holds literally. */
  evidence?: Record<Role, PartEvidence>
}
export interface EntityRow {
  itemId: string
  assetId: string | null
  value: string
  startOffset: number
  endOffset: number
}
export interface TripleRow {
  id: string
  itemId: string
  assetId: string | null
  subject: string
  predicate: string
  object: string
}
type ContextForItem = (itemId: string) => OccurrenceContext
const compare = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0)
const addressKey = (row: { itemId: string; assetId: string | null }) =>
  JSON.stringify([row.itemId, row.assetId])

/** Store repository rows use camelCase. Dedupe per item/asset, retaining all offset hints. */
export function buildEntityTargets(
  label: string,
  rows: readonly EntityRow[],
  context: ContextForItem
): OccurrenceTarget[] {
  const groups = new Map<string, Extract<OccurrenceTarget, { kind: 'entity' }>>()
  const ordered = [...rows].sort(
    (a, b) =>
      compare(addressKey(a), addressKey(b)) ||
      a.startOffset - b.startOffset ||
      a.endOffset - b.endOffset ||
      compare(a.value, b.value)
  )
  for (const row of ordered) {
    if (!key(label) || key(row.value) !== key(label)) continue
    const id = addressKey(row)
    let target = groups.get(id)
    if (!target) {
      target = {
        ...context(row.itemId),
        itemId: row.itemId,
        assetId: row.assetId,
        kind: 'entity',
        value: label.trim(),
        offsets: [],
      }
      groups.set(id, target)
    }
    const duplicate = target.offsets!.some(
      ({ start, end }) => start === row.startOffset && end === row.endOffset
    )
    if (!duplicate) {
      target.offsets!.push({ start: row.startOffset, end: row.endOffset })
    }
  }
  return [...groups.values()]
}

/**
 * A graph node's documents: its NER rows (which carry offset hints) plus every
 * document whose relations name it. Graph nodes come from relation endpoints,
 * which NER does not always tag, so the triples alone must still lead
 * somewhere; those targets search the text for the label.
 */
export function buildNodeTargets(
  label: string,
  entityRows: readonly EntityRow[],
  tripleRows: readonly TripleRow[],
  context: ContextForItem
): OccurrenceTarget[] {
  const targets = buildEntityTargets(label, entityRows, context)
  const seen = new Set(targets.map(addressKey))
  for (const row of tripleRows) {
    if (!key(label) || (key(row.subject) !== key(label) && key(row.object) !== key(label))) {
      continue
    }
    const id = addressKey(row)
    if (seen.has(id)) continue
    seen.add(id)
    targets.push({
      ...context(row.itemId),
      itemId: row.itemId,
      assetId: row.assetId,
      kind: 'entity',
      value: label.trim(),
      offsets: [],
    })
  }
  return targets.sort((a, b) => compare(addressKey(a), addressKey(b)))
}

/** Directed subject → object pair, with a stable representative predicate per asset. */
export function buildRelationTargets(
  subject: string,
  object: string,
  rows: readonly TripleRow[],
  context: ContextForItem
): OccurrenceTarget[] {
  const targets = new Map<string, OccurrenceTarget>()
  const ordered = [...rows].sort(
    (a, b) => compare(addressKey(a), addressKey(b)) || compare(a.id, b.id)
  )
  for (const row of ordered) {
    if (
      !key(subject) ||
      !key(object) ||
      key(row.subject) !== key(subject) ||
      key(row.object) !== key(object)
    ) {
      continue
    }
    const id = addressKey(row)
    if (!targets.has(id)) {
      targets.set(id, {
        ...context(row.itemId),
        itemId: row.itemId,
        assetId: row.assetId,
        kind: 'relation',
        subject: row.subject,
        predicate: row.predicate,
        object: row.object,
      })
    }
  }
  return [...targets.values()]
}

/** Fold for searching, retaining the original UTF-16 span of every folded unit. */
function searchable(raw: string) {
  let text = ''
  const starts: number[] = []
  const ends: number[] = []
  let offset = 0
  for (const char of raw) {
    const end = offset + char.length
    const folded = char.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase()
    if (!folded && ends.length) ends[ends.length - 1] = end
    for (const unit of folded) {
      if (/\s/u.test(unit) && text.endsWith(' ')) {
        ends[ends.length - 1] = end
      } else {
        const value = /\s/u.test(unit) ? ' ' : unit
        text += value
        for (let i = 0; i < value.length; i++) {
          starts.push(offset)
          ends.push(end)
        }
      }
    }
    offset = end
  }
  return { text, starts, ends }
}

function matches(raw: string, value: string): TextRange[] {
  const source = searchable(raw)
  const needle = searchable(value).text.trim()
  if (!needle) return []
  const result: TextRange[] = []
  let from = 0
  while (from <= source.text.length - needle.length) {
    const start = source.text.indexOf(needle, from)
    if (start < 0) break
    const end = start + needle.length
    const before = Array.from(source.text.slice(0, start)).at(-1) ?? ''
    const after = Array.from(source.text.slice(end))[0] ?? ''
    if (!/[\p{L}\p{N}_]/u.test(before) && !/[\p{L}\p{N}_]/u.test(after)) {
      result.push({ start: source.starts[start]!, end: source.ends[end - 1]! })
    }
    from = end
  }
  return result
}

/** Empty list means none. All offsets are half-open JS string (UTF-16) coordinates. */
export function locateOccurrences(raw: string, target: OccurrenceTarget): OccurrencePassage[] {
  if (target.kind === 'entity') {
    const found = matches(raw, target.value)
    for (const hint of target.offsets ?? []) {
      // Stored offsets may be absent or use another coordinate convention; only
      // trust them when the raw slice validates. 0/0 is an absent hint.
      if (
        Number.isInteger(hint.start) &&
        Number.isInteger(hint.end) &&
        hint.start >= 0 &&
        hint.end > hint.start &&
        hint.end <= raw.length &&
        key(raw.slice(hint.start, hint.end)) === key(target.value) &&
        !found.some(({ start, end }) => start === hint.start && end === hint.end)
      ) {
        found.push(hint)
      }
    }
    return found
      .sort((a, b) => a.start - b.start || a.end - b.end)
      .map((range) => ({ ...range, ranges: [], approximate: false }))
  }
  return locateRelation(raw, target)
}

const ROLES: readonly Role[] = ['subject', 'predicate', 'object']
/** Longest passage shown as one occurrence; longer sentences are split at whitespace. */
const MAX_PASSAGE = 400
/** Words too common to count as evidence that a part was rebuilt from a sentence. */
const STOPWORDS = new Set(
  (
    'el la los las lo un una unos unas de del al a en y e o u que con por para sobre su sus se ' +
    'le les es son fue fueron era como mas muy sin entre hacia hasta desde este esta estos estas ' +
    'ese esa esos esas the of and to in on for with by at from as is was were an'
  ).split(' ')
)

/** Sentence ranges in raw coordinates: end punctuation or a blank line closes one. */
function sentences(raw: string): TextRange[] {
  const result: TextRange[] = []
  const push = (start: number, end: number) => {
    while (start < end && /\s/u.test(raw[start]!)) start++
    while (end > start && /\s/u.test(raw[end - 1]!)) end--
    while (end - start > MAX_PASSAGE) {
      const cut = raw.lastIndexOf(' ', start + MAX_PASSAGE)
      const at = cut > start ? cut : start + MAX_PASSAGE
      push(start, at)
      start = at
      while (start < end && /\s/u.test(raw[start]!)) start++
    }
    if (end > start) result.push({ start, end })
  }
  const boundary = /([.!?…]+["'»”)\]]*)(?=\s)|\n[ \t]*\n/gu
  let start = 0
  for (const match of raw.matchAll(boundary)) {
    const end = match.index + (match[1]?.length ?? 0)
    push(start, end)
    start = match.index + match[0].length
  }
  push(start, raw.length)
  return result
}

/** A content word of a part: its six-letter stem and the whole folded word. */
interface Token {
  stem: string
  full: string
}

/** Content words of a part, folded like the text, without repeats. */
function contentTokens(part: string): Token[] {
  const words = searchable(part).text.split(/[^\p{L}\p{N}]+/u)
  const tokens = new Map<string, Token>()
  for (const word of words) {
    if (!word || STOPWORDS.has(word) || (word.length < 4 && !/\p{N}/u.test(word))) continue
    const stem = word.slice(0, 6)
    if (!tokens.has(stem)) tokens.set(stem, { stem, full: word })
  }
  return [...tokens.values()]
}

/** Edit distance, stopping early once it exceeds `limit`. */
function withinEdits(a: string, b: string, limit: number): boolean {
  if (Math.abs(a.length - b.length) > limit) return false
  let previous = Array.from({ length: b.length + 1 }, (_, index) => index)
  for (let i = 1; i <= a.length; i++) {
    const current = [i]
    let best = i
    for (let j = 1; j <= b.length; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1
      current[j] = Math.min(previous[j]! + 1, current[j - 1]! + 1, previous[j - 1]! + cost)
      best = Math.min(best, current[j]!)
    }
    if (best > limit) return false
    previous = current
  }
  return previous[b.length]! <= limit
}

/**
 * Whether a text word stands for a token: same stem (inflections), or the
 * whole word within one or two letters of it, the usual OCR misreading.
 */
function wordMatches(word: string, token: Token): boolean {
  if (word.startsWith(token.stem)) return true
  if (token.full.length < 5) return false
  return withinEdits(word, token.full, token.full.length >= 9 ? 2 : 1)
}

/** Folded words of one sentence with their raw spans. */
function sentenceWords(raw: string, sentence: TextRange) {
  const folded = searchable(raw.slice(sentence.start, sentence.end))
  return [...folded.text.matchAll(/[\p{L}\p{N}]+/gu)].map((match) => ({
    word: match[0],
    start: sentence.start + folded.starts[match.index]!,
    end: sentence.start + folded.ends[match.index + match[0].length - 1]!,
  }))
}

type SentenceWord = ReturnType<typeof sentenceWords>[number]

/** The tokens a sentence holds and the words that hold them. */
function tokenHits(words: readonly SentenceWord[], tokens: readonly Token[]) {
  const hits = tokens.filter((token) => words.some(({ word }) => wordMatches(word, token)))
  const marked = words.filter(({ word }) => hits.some((token) => wordMatches(word, token)))
  return { hits, marked }
}

const overlaps = (a: TextRange, b: TextRange) => a.start < b.end && b.start < a.end
const inside = (range: TextRange, sentence: TextRange) =>
  range.start >= sentence.start && range.end <= sentence.end
const distance = (a: TextRange, b: TextRange) =>
  a.end <= b.start ? b.start - a.end : b.end <= a.start ? a.start - b.end : 0

interface ScoredSentence extends OccurrencePassage {
  score: number
  parts: number
  evidence: Record<Role, PartEvidence>
}

/**
 * The model often rewrites a triple (the full name for «la Federación», a
 * paraphrased link) and sometimes joins pieces read far apart (a signatory
 * line, «firmados por:» above it, the document's title). A relation is
 * located by the sentence that best supports it: each part counts as literal
 * (the whole phrase is there) or partial (some of its content words, OCR
 * misreadings tolerated), and a sentence needs two parts to count. Any part
 * that sentence lacks is then looked for in the rest of the document, so
 * every part the triple names is shown where it is, with what is literal
 * and what was rebuilt kept apart.
 */
function locateRelation(
  raw: string,
  target: Extract<OccurrenceTarget, { kind: 'relation' }>
): OccurrencePassage[] {
  const literalMatches = Object.fromEntries(
    ROLES.map((role) => [role, matches(raw, target[role])])
  ) as Record<Role, TextRange[]>
  const tokens = Object.fromEntries(
    ROLES.map((role) => [role, contentTokens(target[role])])
  ) as Record<Role, Token[]>
  const all = sentences(raw)
  const scored: ScoredSentence[] = []
  for (const sentence of all) {
    const ranges: LabeledRange[] = []
    const evidence = {} as Record<Role, PartEvidence>
    let score = 0
    let parts = 0
    for (const role of ROLES) {
      const found = literalMatches[role].filter((range) => inside(range, sentence))
      ranges.push(...found.map((range) => ({ ...range, role, literal: true })))
      evidence[role] = found.length ? 'literal' : 'missing'
      if (found.length) {
        score += 1
        parts++
      }
    }
    const literalRanges = [...ranges]
    const words = sentenceWords(raw, sentence).filter(
      (word) => !literalRanges.some((range) => overlaps(range, word))
    )
    for (const role of ROLES) {
      if (evidence[role] === 'literal' || !tokens[role].length) continue
      const { hits, marked } = tokenHits(words, tokens[role])
      if (!hits.length) continue
      evidence[role] = 'partial'
      score += (0.8 * hits.length) / tokens[role].length
      parts++
      ranges.push(...marked.map(({ start, end }) => ({ start, end, role, literal: false })))
    }
    if (!parts) continue
    scored.push({
      ...sentence,
      ranges,
      approximate: ROLES.some((role) => evidence[role] !== 'literal'),
      evidence,
      score,
      parts,
    })
  }
  const supported = scored.filter((sentence) => sentence.parts >= 2 && sentence.score >= 1.5)
  let chosen: ScoredSentence[]
  if (supported.length) {
    const best = Math.max(...supported.map((sentence) => sentence.score))
    chosen = supported.filter((sentence) => sentence.score >= 0.75 * best)
  } else {
    // Nothing holds two parts: show the one sentence that best holds a part,
    // flagged, rather than every place a name happens to appear.
    const fallback = scored
      .filter((sentence) => sentence.score >= 0.5)
      .sort((a, b) => b.score - a.score || a.start - b.start)[0]
    chosen = fallback ? [{ ...fallback, approximate: true }] : []
  }
  return chosen.map((sentence) => completeElsewhere(raw, all, sentence, literalMatches, tokens))
}

/**
 * Adds the parts a passage lacks from the rest of the document: the nearest
 * literal occurrence, else the sentence holding most of the part's words
 * (at least half of them, nearest first). Found parts are `elsewhere`.
 */
function completeElsewhere(
  raw: string,
  all: readonly TextRange[],
  passage: ScoredSentence,
  literalMatches: Record<Role, TextRange[]>,
  tokens: Record<Role, Token[]>
): OccurrencePassage {
  const ranges = [...passage.ranges]
  const evidence = { ...passage.evidence }
  for (const role of ROLES) {
    if (evidence[role] !== 'missing') continue
    const literal = [...literalMatches[role]].sort(
      (a, b) => distance(a, passage) - distance(b, passage)
    )[0]
    if (literal) {
      ranges.push({ ...literal, role, literal: true })
      evidence[role] = 'elsewhere'
      continue
    }
    if (!tokens[role].length) continue
    let best: { fraction: number; gap: number; marked: SentenceWord[] } | null = null
    for (const sentence of all) {
      if (overlaps(sentence, passage)) continue
      const words = sentenceWords(raw, sentence).filter(
        (word) => !ranges.some((range) => overlaps(range, word))
      )
      const { hits, marked } = tokenHits(words, tokens[role])
      const fraction = hits.length / tokens[role].length
      const gap = distance(sentence, passage)
      if (fraction < 0.5) continue
      if (!best || fraction > best.fraction || (fraction === best.fraction && gap < best.gap)) {
        best = { fraction, gap, marked }
      }
    }
    if (!best) continue
    ranges.push(...best.marked.map(({ start, end }) => ({ start, end, role, literal: false })))
    evidence[role] = 'elsewhere'
  }
  return {
    start: passage.start,
    end: passage.end,
    ranges: ranges.sort((a, b) => a.start - b.start || a.end - b.end),
    approximate: passage.approximate || ROLES.some((role) => evidence[role] !== 'literal'),
    evidence,
  }
}
