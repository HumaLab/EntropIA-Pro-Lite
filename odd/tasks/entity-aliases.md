# Entity aliases: unify variants of the same person

**Status:** done (2026-11-11), branch `feat/entity-aliases`, PR #279; server deployed. Requested by
the owner 2026-11-05; scope decided 2026-11-11 (see Log L2). Mirror in Engram:
`odd/entity-aliases/tasks`.

## Specs

- **S1 — Global alias list, curated by the owner.** «Globales»: one alias
  list for the whole archive, valid in every collection. An alias maps a
  surface form to a canonical name, e.g. "Mdme. Cuevas = Carmela Cuevas"
  (example from the original request). The owner decides every entry.
- **S2 — Never rewrite mentions.** Each mention keeps its original `value`
  and offsets (needed for highlights and citations); the canonical name is
  resolved at read time from the alias list. Stored entities and every future
  NER run get the alias without rewriting rows, and changing an alias is one
  operation.
- **S3 — Apply where names are grouped:** the item's entity list shows the
  canonical name (with the original form still visible), and **Relaciones 3D**
  unifies its nodes: «¿Los alias también unifican los nodos de Relaciones 3D
  (los extremos de las relaciones extraídas)? — Sí». "Mdme. Cuevas" and
  "Carmela Cuevas" become one node.
- **S4 — Sync including the server.** «Incluir el servidor»: the alias table
  syncs between the owner's devices; EntropIA-Cloud is changed to accept it,
  and the desktop does not sync the table until that server is deployed.
- **S5 — Suggestions by rules only, approved one by one (stage 2).** «Solo
  reglas»: the app proposes groups that look like the same person (shared
  surname, initials, honorifics such as Mdme./Sra./Dr.); the owner approves or
  rejects each group with one click; an approval adds rows to the alias list.
  No LLM pass.
- **S6 — No silent automatic merging.** "Cuevas" alone can be a different
  relative; a wrong merge corrupts research data without anyone noticing.
- **S8 — Where aliases are managed.** «¿Dónde creás y administrás los alias? —
  Ambos (Recomendado)»: from an item's entity list («Es la misma persona que…»
  on a mention) and a full list in Configuración to review, edit and delete
  every alias.
- **S9 — Dialog clarity (owner app check, 2026-11-11).** When the owner picks
  the mention's own name as canonical, the refusal explains that this name is
  already the right one and that an alias is created from the other form,
  choosing this one as canonical. The dialog's suggestions list only mentions
  of the same entity type («Solo mismo tipo»): a person is never offered a
  place or an organization.
- **S7 — Safe development.** The new migration is irreversible and raises the
  account `schema_tag` (all devices must update). Develop and check only with
  `ENTROPIA_DEV_PROFILE`, never on the real archive.

## Tasks

| ID | Specs | What | Route | Commit |
|---|---|---|---|---|
| T1 ✅ | S1, S2, S7 | Migration `0065_entity_aliases` (alias key normalized like `normalize_entity_value`, alias, canonical, timestamps), Drizzle schema, Rust registry entry, `EntityAliasRepo` (list, upsert, delete, resolve), schema fixture | worker | `87cf3973` |
| T2 ✅ | S2, S3 | Read-time resolution: item entity list shows the canonical name plus the original form; Relaciones 3D groups triple endpoints and NER rows through the alias list | worker | `845249c6` |
| T3 ✅ | S1, S6, S8 | UI: «Es la misma persona que…» from the item entity list, plus an alias list in Configuración to review, edit and delete | worker | `2ff52172` |
| T4 ✅ | S4 | Sync: `entity_aliases` in the desktop synced tables after the server accepts it; EntropIA-Cloud change and deploy | worker (both repos) | `722b9af7` (desktop), EntropIA-Cloud `7b018d7` |
| T6 ✅ | S6, S8, S9 | Dialog fixes from the owner's app check: a clear message when the chosen canonical equals the mention, and suggestions limited to the mention's entity type | worker | `ef18a3cf` |
| T5 ✅ | S5, S6 | Stage 2: rule-based group suggestions, approve/reject each | worker | `ad674828` |

## Background

### Problem

NER records each mention as a loose row in `entities` (`value` text, no
canonical id). The same person shows up under several forms, e.g.
"Carmela Cuevas" and "Mdme. Cuevas". The only unification today is
case/whitespace (`normalize_entity_value`, `nlp/ner/mod.rs:484`). Fixing
variants means editing every mention by hand in `ItemView`, document by
document.

What already works and must be kept: a manual edit sets `source = 'manual'`,
so a NER rerun does not overwrite it (`nlp/ner/mod.rs:318-335`), and changing
`value` resets the geocode (`entity.repo.ts:96-120`).

### Map of the current code (2026-11-11)

- `entities` (`packages/store/src/schema.ts:182-200`): no canonical name and no
  collection column; latest migration `0064_derived_source_path`
  (`src-tauri/src/db/migrations.rs:208`); Rust applies the shared SQL files.
- Almost nothing groups by name: the item entity list, the map and the export
  show one row per mention. Relaciones 3D groups **triple endpoints**
  (`lib/relation-graph.ts:53,80-117`, trim + collapse + lowercase) and looks up
  NER rows by normalized value (`RelationGraph3D.svelte:109-124`,
  `lib/relation-occurrences.ts:80-112`).
- `entities` is row-synced (`src-tauri/src/sync/capture.rs:16-55`); a new
  synced table needs the client allowlist and FK order plus the server's
  acceptance; apply journals unknown columns as `schema_drift`.

### Rejected

Silent automatic merging (S6). A canonical column written on every entity row:
changing an alias would rewrite many rows, and it would not cover triple
endpoints.

## Open questions

- hlab-platform parity (out of scope for now).

## Log

- L1 (2026-11-11, owner, verbatim): «me interesan estas 3, me las explicas y
  desarrollas??  - Tres tareas sin empezar: entity-aliases, in-app-help-assistant
  y research-unreadable-batch-message.» Then, choosing the next task:
  «Alias de entidades».
- L2 (2026-11-11, owner answers, verbatim): «¿Los alias valen para todo el
  archivo o por colección? — Globales (Recomendado)»; «La tabla de alias tiene
  que sincronizarse … ¿Cómo lo encaramos? — Incluir el servidor
  (Recomendado)»; «Para la etapa 2, ¿cómo detecta la app los grupos
  candidatos? — Solo reglas (Recomendado)»; «¿Los alias también unifican los
  nodos de Relaciones 3D (los extremos de las relaciones extraídas)? — Sí
  (Recomendado)».
- L3 (2026-11-11): T1 done in `87cf3973`: store 23 files / 494 tests (16 new, RED first: module missing), Rust `db::` 123 tests incl. 0065 on top of 0064, typecheck, prettier and cargo fmt clean. Not synced (T4).
- L4 (2026-11-11, owner answer, verbatim): «¿Dónde creás y administrás los alias? — Ambos (Recomendado)».
- L5 (2026-11-11): T2 done in `845249c6`: optional `resolveAlias` prop on the ui EntityViewer (a first desktop fork of the component was replaced by the prop to avoid duplicating it), alias resolution in relation-graph and relation-occurrences (identity by default). ui 959 tests, desktop 297 files / 4140 tests, both typechecks, eslint and format:check clean.
- L6 (2026-11-11): T3 done in `2ff52172`: optional `onaliasentity` action on the ui EntityViewer, EntityAliasDialog (suggestions from the item, owner confirms), Configuración › Alias tab (edit canonical, delete with confirmation, empty state), resolver refresh on every change. ui 961 tests, desktop 299 files / 4156 tests, both typechecks, eslint and format:check clean. Relaciones 3D picks up a change on its next render.
- L7 (2026-11-11): T5 done in `ad674828`: pure rule-based suggester (honorifics, surname + compatible given names or initials, bare surname only with a single fuller name, already-aliased variants skipped), EntityRepo.listDistinctValues, «Sugerencias» in Configuración › Alias with accept/discard. store 498 tests, desktop 300 files / 4190 tests, both typechecks, eslint and format:check clean.
- L8 (2026-11-11, owner, verbatim): «Pusheá la rama y abrí el PR, aunque sea como borrador hasta que esté la T4. Te autorizo a trabajar en EntropIA-Cloud para la sincronización».
- L9 (2026-11-11): T4 code done. Server (EntropIA-Cloud `7b018d7`, branch `feat/entity-aliases-v1`): `entity_aliases` in the allowlist (21) behind capability `entity-aliases-v1`, advertised last; rows stay opaque, no server schema change; full cargo test green. Desktop (`722b9af7`): table synced (19), alias ops held in the oplog until an ordinary pull advertises `entity-aliases-v1` for the epoch, one since-zero catch-up, logout clears the verdict; ids are now deterministic (`alias:` + normalized key) so the same alias on two devices converges instead of colliding on the UNIQUE key. sync 511 / db 123 / store 500 tests green. Pending: deploy the server (owner, RUNBOOK §1) and a live two-device check.
- L10 (2026-11-11): EntropIA-Cloud PR #2 merged (`b8c6d31f`) after review and a local full `cargo test` (19 suites) + fmt. Coolify auto-deployed it: production container runs image `idhit0wuzbld1u4ee83akvr6:b8c6d31f…`, 0 errors in the logs, /v1/health ok, epoch `1cb97727` unchanged. Still to do: owner check in the running app with `ENTROPIA_DEV_PROFILE`, and a live two-device sync check once a desktop build with migration 0065 is installed.
- L11 (2026-11-11, owner in the dev app, verbatim): «no me deja guardar cuando opto pro el msimo nombre proqiue es el correcto!!!!» — dialog on «Fernando Solá», suggestions «Buenos Aires, Anselmo Marini, Confederación General del Trabajo, Norberto Oscar Centeno, Movimiento de Acción Sindical, Marino Contino, Instituto Tecnológico Pesquero, Puerto», message «El alias y el nombre canónico son el mismo: elegí un nombre canónico distinto.» Decision: «Mensaje claro (Recomendado), Solo mismo tipo (Recomendado)»; the reverse-direction dialog was not chosen.
- L12 (2026-11-11): T6 done in `ef18a3cf`: same-name refusal explains what to do (es/en), dialog suggestions limited to the mention's entity type and never its own value. desktop 300 files / 4192 tests, typecheck, eslint and format:check clean.
- L13 (2026-11-11, owner, verbatim): «ya hice todo, funciona» — checked in the dev app (profile `alias`, sync off): the same-name message, same-type suggestions, saving an alias (canonical shown with the original form), the unified node in Relaciones 3D, and editing and deleting in Configuración › Alias. Not checkable in a dev profile: sync between two devices (needs an installed build with migration 0065 on both).
