import { key } from './relation-graph'

export interface TextRange {
  start: number
  end: number
}
export interface LabeledRange extends TextRange {
  role: 'subject' | 'predicate' | 'object'
}
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
  const subjects = matches(raw, target.subject)
  const objects = matches(raw, target.object)
  const predicates = matches(raw, target.predicate)
  const passages: OccurrencePassage[] = []
  for (const subject of subjects) {
    const object = objects
      .filter(
        (candidate) =>
          Math.max(subject.end, candidate.end) - Math.min(subject.start, candidate.start) <= 400
      )
      .sort((a, b) => Math.abs(a.start - subject.start) - Math.abs(b.start - subject.start))[0]
    if (!object) continue
    const start = Math.min(subject.start, object.start)
    const end = Math.max(subject.end, object.end)
    const ranges: LabeledRange[] = [
      { ...subject, role: 'subject' },
      ...predicates
        .filter((range) => range.start >= start && range.end <= end)
        .map((range): LabeledRange => ({ ...range, role: 'predicate' })),
      { ...object, role: 'object' },
    ]
    passages.push({ start, end, ranges, approximate: false })
  }
  if (passages.length) return passages.sort((a, b) => a.start - b.start || a.end - b.end)
  return [
    ...subjects.map((range) => ({ ...range, role: 'subject' as const })),
    ...objects.map((range) => ({ ...range, role: 'object' as const })),
  ]
    .sort((a, b) => a.start - b.start)
    .map((range) => ({
      start: range.start,
      end: range.end,
      ranges: [range],
      approximate: true,
    }))
}
