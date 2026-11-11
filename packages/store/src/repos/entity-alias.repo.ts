import { asc, eq } from 'drizzle-orm'
import type { DrizzleClient } from '../types'
import { entityAliases } from '../schema'

export type EntityAlias = typeof entityAliases.$inferSelect

/**
 * Normalizes an entity surface form into the alias key, exactly how the app
 * already compares entity values: trim, collapse every whitespace run to one
 * space, lowercase. Mirrors Rust `normalize_entity_value`
 * (`apps/desktop/src-tauri/src/nlp/ner/mod.rs`) and the relation-graph key
 * (`apps/desktop/src/lib/relation-graph.ts`).
 */
export function normalizeEntityAliasKey(value: string): string {
  return value.trim().replace(/\s+/g, ' ').toLowerCase()
}

/** The only columns the read-time resolver needs. */
export type AliasResolverRow = Pick<EntityAlias, 'aliasKey' | 'canonical'>

/**
 * Builds a pure read-time resolver from alias rows: the canonical name when
 * the normalized value has an alias, otherwise the value unchanged.
 * Single-hop on purpose (S6): a canonical is never resolved again, so a bad
 * entry cannot cascade into a silent wrong merge.
 */
export function buildAliasResolver(rows: AliasResolverRow[]): (value: string) => string {
  const byKey = new Map(rows.map((row) => [row.aliasKey, row.canonical]))
  return (value: string) => byKey.get(normalizeEntityAliasKey(value)) ?? value
}

export class EntityAliasRepo {
  constructor(private db: DrizzleClient) {}

  /** Every alias ordered by normalized key. */
  async list(): Promise<EntityAlias[]> {
    return this.db.select().from(entityAliases).orderBy(asc(entityAliases.aliasKey))
  }

  /**
   * Inserts `{ alias, canonical }`, or — when the normalized alias key
   * already exists — updates that row's canonical and `updatedAt`.
   *
   * The owner decides every entry (S6), so anything that would silently merge
   * is rejected with a clear Error and leaves the table unchanged:
   * - empty alias or canonical;
   * - alias equal to canonical after normalization;
   * - a chain: a canonical that is itself an alias key, or an alias whose
   *   key is already used as a canonical by another row (single-hop only).
   */
  async upsert(input: { alias: string; canonical: string }): Promise<EntityAlias> {
    const key = normalizeEntityAliasKey(input.alias)
    if (!key) {
      throw new Error('Cannot save entity alias: the alias must not be empty.')
    }
    const canonical = input.canonical.trim()
    if (!canonical) {
      throw new Error('Cannot save entity alias: the canonical name must not be empty.')
    }
    if (key === normalizeEntityAliasKey(canonical)) {
      throw new Error(
        `Cannot alias "${input.alias}" to itself: alias and canonical normalize to the same value.`
      )
    }

    const rows = await this.list()
    const existing = rows.find((row) => row.aliasKey === key)
    const others = existing ? rows.filter((row) => row.id !== existing.id) : rows

    if (others.some((row) => row.aliasKey === normalizeEntityAliasKey(canonical))) {
      throw new Error(
        `Cannot point "${input.alias}" at "${canonical}": "${canonical}" is itself an alias — resolution is single-hop, so the owner picks the canonical directly.`
      )
    }
    if (others.some((row) => normalizeEntityAliasKey(row.canonical) === key)) {
      throw new Error(
        `Cannot alias "${input.alias}": it is already used as a canonical name by another entry — the owner decides merges one by one.`
      )
    }

    if (existing) {
      const updated = await this.db
        .update(entityAliases)
        .set({ canonical, updatedAt: Date.now() })
        .where(eq(entityAliases.id, existing.id))
        .returning()
      return updated[0]!
    }

    const now = Date.now()
    const created = await this.db
      .insert(entityAliases)
      .values({
        id: crypto.randomUUID(),
        aliasKey: key,
        alias: input.alias,
        canonical,
        createdAt: now,
        updatedAt: now,
      })
      .returning()
    return created[0]!
  }

  /** Deletes the alias with this id; a no-op when it does not exist. */
  async delete(id: string): Promise<void> {
    await this.db.delete(entityAliases).where(eq(entityAliases.id, id))
  }
}
