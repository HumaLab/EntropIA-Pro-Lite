import type { JSONContent } from '@tiptap/core'
import type { CanonicalDocument } from './document-contract'

/**
 * Sections across a manuscript's versions (plan "historial por apartado").
 *
 * The same rule as `sections.ts`: a section is a top-level heading plus what
 * follows until the next heading of the same level or higher, and the notes
 * container belongs to no section. Here it runs over the stored canonical JSON
 * of each version, so nothing is added to the schema and every old version
 * works as it is.
 *
 * A section has no stored identity. It is found by its title (the nth heading
 * of that level and title, so a moved section is still found) and, when no
 * heading carries that title any more, by its position among the headings of
 * its level (so a renamed section in place is still found). A section that was
 * both renamed and moved is not recognised: it reads as absent.
 */

export interface SectionKey {
  /** The heading's plain text, trimmed. */
  title: string
  level: number
  /** Which heading with this level and title it is: 0 for the first. */
  occurrence: number
  /** Its position among the headings of its level: 0 for the first. */
  ordinal: number
}

export interface DocumentSection extends SectionKey {
  /** First top-level child of the section (its heading). */
  start: number
  /** One past its last top-level child. */
  end: number
  /** Content hash of the section's blocks, heading included. */
  hash: string
}

const DOCUMENT_OWNED = new Set(['footnotes'])

function textOf(node: JSONContent): string {
  if (typeof node.text === 'string') return node.text
  return (node.content ?? []).map(textOf).join('')
}

function levelOf(node: JSONContent): number | null {
  if (node.type !== 'heading') return null
  const level = node.attrs?.level
  return typeof level === 'number' ? level : 1
}

/** FNV-1a, 32 bits: enough to tell whether a section's blocks changed. */
function hashBlocks(blocks: JSONContent[]): string {
  const text = JSON.stringify(blocks)
  let hash = 0x811c9dc5
  for (let i = 0; i < text.length; i++) {
    hash ^= text.charCodeAt(i)
    hash = Math.imul(hash, 0x01000193)
  }
  return (hash >>> 0).toString(16).padStart(8, '0')
}

/** The sections of a document, in order (subsections listed after their parent). */
export function documentSections(document: CanonicalDocument | null): DocumentSection[] {
  const children = document?.doc?.content ?? []
  const sections: DocumentSection[] = []
  const occurrences = new Map<string, number>()
  const ordinals = new Map<number, number>()
  children.forEach((child, start) => {
    const level = levelOf(child)
    if (level === null) return
    let end = start + 1
    while (end < children.length) {
      const next = children[end]!
      if (DOCUMENT_OWNED.has(next.type ?? '')) break
      const other = levelOf(next)
      if (other !== null && other <= level) break
      end++
    }
    const title = textOf(child).trim()
    const titleKey = `${level}\u0000${title}`
    const occurrence = occurrences.get(titleKey) ?? 0
    occurrences.set(titleKey, occurrence + 1)
    const ordinal = ordinals.get(level) ?? 0
    ordinals.set(level, ordinal + 1)
    sections.push({
      title,
      level,
      occurrence,
      ordinal,
      start,
      end,
      hash: hashBlocks(children.slice(start, end)),
    })
  })
  return sections
}

/**
 * The section `key` names among `sections`: by title, else by position.
 *
 * With `origin` (the document `key` was read from), the position fallback only
 * accepts a heading whose title `origin` does not have: a section that exists
 * there under its own title is that section, not the renamed one.
 */
export function findSection(
  sections: DocumentSection[],
  key: SectionKey,
  origin?: CanonicalDocument | null
): DocumentSection | undefined {
  const byTitle = sections.find(
    (s) => s.level === key.level && s.title === key.title && s.occurrence === key.occurrence
  )
  if (byTitle) return byTitle
  const byPosition = sections.find((s) => s.level === key.level && s.ordinal === key.ordinal)
  if (!byPosition) return undefined
  if (origin) {
    const known = documentSections(origin).some(
      (s) => s.level === byPosition.level && s.title === byPosition.title
    )
    if (known) return undefined
  }
  return byPosition
}

/**
 * The versions (oldest first) where the section appears for the first time or
 * its content differs from the previous version. A version without it breaks
 * the chain: the next one that has it again counts as a change.
 */
export function sectionChanges(
  versions: { id: string; document: CanonicalDocument | null }[],
  key: SectionKey
): string[] {
  const changed: string[] = []
  let previous: string | null = null
  for (const { id, document } of versions) {
    const hash = findSection(documentSections(document), key)?.hash ?? null
    if (hash !== null && hash !== previous) changed.push(id)
    previous = hash
  }
  return changed
}

/**
 * `current` with the section `key` replaced by its content in `from`. Null when
 * either document lacks the section: there is nothing to restore, or nowhere to
 * put it. `current` itself is not changed.
 */
export function restoreSection(
  current: CanonicalDocument,
  from: CanonicalDocument,
  key: SectionKey
): CanonicalDocument | null {
  const target = findSection(documentSections(current), key)
  const source = findSection(documentSections(from), key, current)
  if (!target || !source) return null
  const children = [...(current.doc.content ?? [])]
  const replacement = structuredClone((from.doc.content ?? []).slice(source.start, source.end))
  children.splice(target.start, target.end - target.start, ...replacement)
  return { ...current, doc: { ...current.doc, content: children } }
}
