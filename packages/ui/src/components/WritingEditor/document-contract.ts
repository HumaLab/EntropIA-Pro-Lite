import { getSchema } from '@tiptap/core'
import type { JSONContent } from '@tiptap/core'
import { WRITING_EXTENSIONS } from './extensions'

/**
 * The canonical manuscript contract (plan-editor.md §8.1, §8.3).
 *
 * # Why this module exists
 *
 * Spike S1 measured what Tiptap does with a document it cannot parse: handing
 * it one unknown node *or* one unknown mark does not drop that element — it
 * empties the entire document. 400 bytes become 47. Tiptap catches
 * ProseMirror's `RangeError`, logs a warning, and substitutes a blank document;
 * nothing throws where the caller could notice.
 *
 * With autosave running, the next write then puts that blank document over the
 * real manuscript. That is the whole failure: a version skew or a half-applied
 * migration turns into silent, permanent loss.
 *
 * So nothing reaches the editor without passing through here first. §8.3 asks
 * for "capacidad de informar un error sin sobrescribir el documento original";
 * this is that capacity, and it is cheap — building the schema needs no DOM.
 */

/** Bumped only for a change the old reader cannot understand (§8.3). */
export const WRITING_SCHEMA_VERSION = 1

export interface CanonicalDocument {
  schemaVersion: number
  doc: JSONContent
}

export type ValidationFailure = {
  ok: false
  code: 'unknown-node' | 'unknown-mark' | 'invalid-structure' | 'unsupported-schema-version'
  message: string
}

export type ValidationResult = { ok: true } | ValidationFailure

export type ParseResult = ({ ok: true } & { document: CanonicalDocument }) | ValidationFailure

/** A fresh manuscript: one empty paragraph, at the current schema version. */
export function emptyDocument(): CanonicalDocument {
  return {
    schemaVersion: WRITING_SCHEMA_VERSION,
    doc: { type: 'doc', content: [{ type: 'paragraph' }] },
  }
}

// Built once: the schema is derived purely from the extension list.
let cachedSchema: ReturnType<typeof getSchema> | null = null
function schema() {
  cachedSchema ??= getSchema(WRITING_EXTENSIONS)
  return cachedSchema
}

function isEnvelope(input: unknown): input is CanonicalDocument {
  if (typeof input !== 'object' || input === null) return false
  const candidate = input as Record<string, unknown>
  if (typeof candidate.schemaVersion !== 'number') return false
  const doc = candidate.doc
  return typeof doc === 'object' && doc !== null && (doc as JSONContent).type === 'doc'
}

/**
 * Classifies ProseMirror's complaint. Its messages are stable enough to read,
 * and the distinction matters: an unknown node means the document was written
 * by something that knows more than this build, which is a different
 * conversation from a malformed file.
 */
function classify(error: unknown): ValidationFailure {
  const message = error instanceof Error ? error.message : String(error)
  if (/Unknown node type/i.test(message)) {
    return { ok: false, code: 'unknown-node', message }
  }
  // ProseMirror words this one differently from the node case:
  // "There is no mark type provenanceMark in this schema".
  if (/no mark type/i.test(message)) {
    return { ok: false, code: 'unknown-mark', message }
  }
  return { ok: false, code: 'invalid-structure', message }
}

/** Whether this build can mount `input` without losing any of it. */
export function validateCanonical(input: unknown): ValidationResult {
  if (!isEnvelope(input)) {
    return {
      ok: false,
      code: 'invalid-structure',
      message: 'not a canonical document envelope: expected { schemaVersion, doc }',
    }
  }
  if (input.schemaVersion > WRITING_SCHEMA_VERSION) {
    return {
      ok: false,
      code: 'unsupported-schema-version',
      message: `document is at schema version ${input.schemaVersion}; this build reads up to ${WRITING_SCHEMA_VERSION}`,
    }
  }
  try {
    // `check()` is what surfaces an unknown mark: `nodeFromJSON` alone accepts
    // some shapes that the schema will later reject.
    schema().nodeFromJSON(input.doc).check()
    return { ok: true }
  } catch (error) {
    return classify(error)
  }
}

/**
 * The only sanctioned way to turn stored bytes into something the editor may
 * mount. On failure it returns the reason and **no document**, so a caller
 * cannot accidentally proceed with a blank one.
 */
export function parseCanonical(input: unknown): ParseResult {
  let candidate = input
  if (typeof input === 'string') {
    try {
      candidate = JSON.parse(input)
    } catch (error) {
      return {
        ok: false,
        code: 'invalid-structure',
        message: `stored content is not valid JSON: ${
          error instanceof Error ? error.message : String(error)
        }`,
      }
    }
  }
  const result = validateCanonical(candidate)
  if (!result.ok) return result
  return { ok: true, document: candidate as CanonicalDocument }
}
