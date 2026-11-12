/**
 * Rule-based entity-alias suggestions (T5 of `odd/tasks/entity-aliases.md`,
 * S5/S6). No LLM: groups of person-type values that look like the same person
 * (shared surname, initials, honorifics such as Mdme./Sra./Dr.) are proposed
 * and the owner approves or rejects each group one by one. Nothing is merged
 * automatically and mentions are never edited (S2): an approval only adds
 * rows to the alias list.
 *
 * Conservative on purpose (S6): a bare surname alone ("Cuevas") is only
 * suggested when exactly one fuller name has that surname, and when two
 * different fuller names share a surname ("Carmela Cuevas", "Juan Cuevas")
 * the short forms are attached to neither.
 */

export interface AliasSuggestionValue {
  /** A distinct archive value, e.g. "Mdme. Cuevas". */
  value: string
  /** How many mentions carry this value across the archive. */
  mentions: number
}

export interface AliasSuggestionAlias {
  /** Normalized alias key (`normalizeEntityAliasKey`). */
  aliasKey: string
  canonical: string
}

export interface AliasSuggestionInput {
  values: AliasSuggestionValue[]
  aliases: AliasSuggestionAlias[]
  /** Normalized canonical keys the owner dismissed in this session. */
  dismissedKeys?: ReadonlySet<string>
}

export interface EntityAliasSuggestion {
  /** The fullest form (ties broken by most mentions). */
  canonical: string
  mentions: number
  variants: string[]
  totalMentions: number
}

/**
 * Honorifics stripped from the front of a name, matched case-insensitively
 * with or without the trailing dot (diacritics ignored, so Doña and Dona
 * both match).
 */
const HONORIFICS = new Set([
  'mdme',
  'mme',
  'madame',
  'sra',
  'sr',
  'srta',
  'dr',
  'dra',
  'don',
  'dona',
  'd',
  'dna',
  'mr',
  'mrs',
  'ms',
])

function fold(value: string): string {
  return value
    .normalize('NFD')
    .replace(/[\u0300-\u036f]/g, '')
    .toLowerCase()
}

function normalizeKey(value: string): string {
  return value.trim().replace(/\s+/g, ' ').toLowerCase()
}

function tokensOf(value: string): string[] {
  const collapsed = value.trim().replace(/\s+/g, ' ')
  return collapsed ? collapsed.split(' ') : []
}

function isHonorificToken(token: string): boolean {
  return HONORIFICS.has(fold(token.replace(/\.+$/, '')))
}

/**
 * Removes leading honorifics ("Mdme.", "Sra.", "Dr.", "Don", "D.", "Mrs."
 * …) and returns the bare name. Names without an honorific are returned
 * unchanged (whitespace collapsed).
 */
export function stripHonorific(value: string): string {
  const tokens = tokensOf(value)
  let index = 0
  // Always keep at least one token, so a lone "Sra." does not vanish.
  while (index < tokens.length - 1 && isHonorificToken(tokens[index]!)) {
    index += 1
  }
  return tokens.slice(index).join(' ')
}

function isInitial(token: string): boolean {
  return /^[a-záéíóúñü]\.?$/i.test(token)
}

interface ParsedName {
  key: string
  display: string
  mentions: number
  hadHonorific: boolean
  stripped: string[]
  surname: string
  given: string
  isFull: boolean
  isShort: boolean
}

function parseName(display: string, mentions: number, key: string): ParsedName | null {
  const original = tokensOf(display)
  const stripped = tokensOf(stripHonorific(display))
  if (stripped.length === 0) return null
  if (stripped.length === 1 && isInitial(stripped[0]!)) return null
  const surname = stripped[stripped.length - 1]!
  const given = stripped[0]!
  const isFull = stripped.length >= 2 && !isInitial(given)
  // Short forms: a bare surname (with or without a honorific) or a given
  // name reduced to an initial ("C. Cuevas").
  const isShort = stripped.length === 1 || (stripped.length >= 2 && isInitial(given) && !isFull)
  return {
    key,
    display,
    mentions,
    hadHonorific: original.length > stripped.length,
    stripped,
    surname,
    given,
    isFull,
    isShort,
  }
}

function surnameKey(surname: string): string {
  return fold(surname)
}

function givenKey(given: string): string {
  return fold(given.replace(/\.+$/, ''))
}

function initialOf(given: string): string {
  return givenKey(given).charAt(0)
}

function shortMatchesFuller(short: ParsedName, fuller: ParsedName): boolean {
  if (short.stripped.length === 1) return true
  return initialOf(short.given) === initialOf(fuller.given)
}

/**
 * Groups person-type values into alias suggestions by rules only. Skips
 * variants that already have an alias, groups the owner dismissed this
 * session, and anything ambiguous. Deterministic: most mentions first
 * (group total), then alphabetical by canonical name.
 */
export function suggestEntityAliases(input: AliasSuggestionInput): EntityAliasSuggestion[] {
  const aliasKeys = new Set(input.aliases.map((row) => row.aliasKey))
  const canonicalKeys = new Set(input.aliases.map((row) => normalizeKey(row.canonical)))
  const dismissed = input.dismissedKeys ?? new Set<string>()

  // Merge raw rows that normalize to the same key; the display form is the
  // one with most mentions (ties: longest, then alphabetical).
  const merged = new Map<string, { display: string; mentions: number }>()
  const rawCounts = new Map<string, Map<string, number>>()
  for (const { value, mentions } of input.values) {
    const key = normalizeKey(value)
    if (!key) continue
    let perRaw = rawCounts.get(key)
    if (!perRaw) {
      perRaw = new Map()
      rawCounts.set(key, perRaw)
    }
    perRaw.set(value, (perRaw.get(value) ?? 0) + mentions)
  }
  for (const [key, perRaw] of rawCounts) {
    let display = ''
    let displayMentions = -1
    let total = 0
    for (const [raw, count] of perRaw) {
      total += count
      if (
        count > displayMentions ||
        (count === displayMentions &&
          (raw.length > display.length || (raw.length === display.length && raw < display)))
      ) {
        display = raw
        displayMentions = count
      }
    }
    merged.set(key, { display, mentions: total })
  }

  const parsed: ParsedName[] = []
  for (const [key, { display, mentions }] of merged) {
    if (aliasKeys.has(key)) continue
    const name = parseName(display, mentions, key)
    if (name) parsed.push(name)
  }

  const bySurname = new Map<string, ParsedName[]>()
  for (const name of parsed) {
    const key = surnameKey(name.surname)
    const group = bySurname.get(key)
    if (group) group.push(name)
    else bySurname.set(key, [name])
  }

  const suggestions: EntityAliasSuggestion[] = []
  for (const group of bySurname.values()) {
    if (group.length < 2) continue
    const fullers = group.filter((name) => name.isFull)
    if (fullers.length === 0) continue
    // S6: two different fuller names sharing a surname means the short
    // forms could belong to either — attach them to neither.
    if (new Set(fullers.map((name) => givenKey(name.given))).size > 1) continue

    const orderedFullers = fullers.slice().sort((a, b) => {
      if (b.stripped.length !== a.stripped.length) return b.stripped.length - a.stripped.length
      if (b.mentions !== a.mentions) return b.mentions - a.mentions
      return a.key < b.key ? -1 : a.key > b.key ? 1 : 0
    })
    const canonical = orderedFullers[0]!
    if (aliasKeys.has(canonical.key) || dismissed.has(canonical.key)) continue

    const variants: string[] = []
    for (const name of group) {
      if (name.key === canonical.key) continue
      if (aliasKeys.has(name.key) || canonicalKeys.has(name.key)) continue
      if (name.isFull) {
        variants.push(name.display)
        continue
      }
      if (name.isShort && shortMatchesFuller(name, canonical)) {
        variants.push(name.display)
      }
    }
    if (variants.length === 0) continue
    variants.sort((a, b) => {
      const ka = normalizeKey(a)
      const kb = normalizeKey(b)
      return ka < kb ? -1 : ka > kb ? 1 : 0
    })

    suggestions.push({
      canonical: canonical.display,
      mentions: canonical.mentions,
      variants,
      totalMentions: group.reduce((sum, name) => sum + name.mentions, 0),
    })
  }

  suggestions.sort((a, b) => {
    if (b.totalMentions !== a.totalMentions) return b.totalMentions - a.totalMentions
    const ka = normalizeKey(a.canonical)
    const kb = normalizeKey(b.canonical)
    return ka < kb ? -1 : ka > kb ? 1 : 0
  })
  return suggestions
}
