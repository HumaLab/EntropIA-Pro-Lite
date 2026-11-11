/**
 * Read-time entity-alias resolver (T2 of `odd/tasks/entity-aliases.md`, S2/S3).
 *
 * The alias list is global to the archive and curated by the owner (S1); every
 * mention keeps its stored `value` and offsets, and the canonical name is
 * resolved here, at read time, so stored entities and future NER runs need no
 * rewrite. T3 adds editing on top and triggers a reload through
 * {@link refreshEntityAliases}/{@link invalidateEntityAliases}.
 *
 * A failure to load the list (store missing, query error) never breaks the
 * views: loading falls back to the identity resolver.
 */
import { buildAliasResolver } from '@entropia/store'
import { getStore } from './db'

/** Canonical name for a mention's surface form; identity when it has no alias. */
export type EntityAliasResolver = (value: string) => string

/** No aliases: every value resolves to itself. */
export const identityEntityAliasResolver: EntityAliasResolver = (value) => value

let cached: EntityAliasResolver = identityEntityAliasResolver
let loaded = false
let pending: Promise<EntityAliasResolver> | null = null

/** The resolver loaded so far, or the identity resolver before the first load. */
export function getEntityAliasResolver(): EntityAliasResolver {
  return cached
}

async function fetchResolver(): Promise<EntityAliasResolver> {
  const rows = await getStore().entityAliases.list()
  return buildAliasResolver(rows)
}

/**
 * Loads the alias list once through the store and shares the in-flight
 * request; later calls reuse the cached resolver. Never throws: on any
 * failure every value resolves to itself. A failure is not cached, so the
 * next load retries.
 */
export function loadEntityAliasResolver(): Promise<EntityAliasResolver> {
  if (loaded) return Promise.resolve(cached)
  if (!pending) {
    pending = fetchResolver().then(
      (resolver) => {
        cached = resolver
        loaded = true
        pending = null
        return resolver
      },
      () => {
        pending = null
        return identityEntityAliasResolver
      }
    )
  }
  return pending
}

/** Drops the cached resolver; the next load reads the list again. */
export function invalidateEntityAliases(): void {
  cached = identityEntityAliasResolver
  loaded = false
  pending = null
}

/** Reloads the list now, so alias editing (T3) can publish its changes. */
export function refreshEntityAliases(): Promise<EntityAliasResolver> {
  invalidateEntityAliases()
  return loadEntityAliasResolver()
}
