# Entity aliases: unify variants of the same person

**Status:** pending, not started. Requested by the owner 2026-11-05.

## Problem

NER records each mention as a loose row in `entities` (`value` text, no
canonical id). The same person shows up under several forms, e.g.
"Carmela Cuevas" and "Mdme. Cuevas". The only unification today is
case/whitespace (`normalize_entity_value`, `nlp/ner/mod.rs:450`). Fixing
variants means editing every mention by hand in `ItemView`, document by
document.

What already works and must be kept: a manual edit sets `source = 'manual'`,
so a NER rerun does not overwrite it (`nlp/ner/mod.rs:286`), and changing
`value` resets the geocode (`entity.repo.ts:105`).

## Agreed direction

1. **Owner-curated alias list** (the source of truth): "Mdme. Cuevas =
   Carmela Cuevas". Applied to entities already stored and to every future
   NER run. Never wrong, because the owner decides.
2. **Suggestions, not automatic merges:** the app proposes groups that look
   like the same person (shared surname, initials, honorifics such as
   Mdme./Sra./Dr., optionally an LLM pass). The owner approves or rejects each
   group with one click; an approval adds rows to the alias list.

Rejected: silent automatic merging. "Cuevas" alone can be a different
relative; a wrong merge corrupts research data without anyone noticing.

## Open questions

- Scope of the alias list: per collection or global.
- Keep the original surface form (needed for offsets and citations) and add a
  canonical name, instead of overwriting `value`.
- New migration means a `schema_tag` bump for sync (see `plan.md` rule 5).
- hlab-platform parity.
