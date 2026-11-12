# Research: readable message when a batch's model output is unusable

**Status:** done (2026-11-10): the engine fixed the root cause on 2026-11-05/06
and the desktop now words the remaining warnings plainly; see «Resolution».
Requested by the owner 2026-11-05, after a
real run on another PC ("Variantes en el nombre de Carmela", model
`anthropic/claude-haiku-5.5`, collection of 323 indexed items).

## What the researcher saw

In the report's "Limitaciones" block:

- `salida de archivo no parseable; lote registrado sin claims`
- `Degradación del pipeline: Salida inválida de asistente_archivo: la salida no contiene JSON recuperable (asistente_archivo).`

Both lines describe the SAME event, in engine jargon (`parseable`, `claims`,
`pipeline`, `JSON`, internal role name `asistente_archivo`). The researcher
cannot tell what happened, how much it matters, or what to do.

## What actually happened

The `asistente_archivo` role got a batch of evidence and had to answer with
the contract `{summary, claims, limitations}`. Its answer had no readable
JSON (`decodificar_salida`), the single repair call failed too, so the role
fell back to `T::default()` and the batch was recorded with no findings.

Consequence that the message hides: **the documents in that batch were never
read into the report.** Negative conclusions in the same report ("ninguno de
los fragmentos menciona 'Carmela Mackenna de Cuevas'") may be false, because
part of the evidence is missing.

## Where the text is produced

- Engine (repo `HumaLab/EntropIA-Agent`, pinned via git in
  `apps/desktop/src-tauri/Cargo.toml`):
  - `src/investigacion.rs` ~2002: the limitation text `salida de archivo no
    parseable; lote registrado sin claims`.
  - `src/investigacion.rs` ~1109: `role_warning` event
    `Salida inválida de {role}: {e}`.
  - `src/investigacion.rs` ~2567: `la salida no contiene JSON recuperable`.
  - `src/informe_render.rs` ~550: Markdown render of the degradation line.
- Desktop (this repo): `apps/desktop/src/views/InvestigationView.svelte`
  ~625-638 (`reportLimitations`) turns `role_warnings` into
  `Degradación del pipeline: …` and merges them with `archive_limitations`.

## Proposed direction

1. **One message per event, in plain Spanish**, saying the consequence and
   the action. Draft:
   > «No se pudieron leer N fragmentos de documentos: el modelo devolvió una
   > respuesta con un formato que el programa no entiende. Lo que dice este
   > informe no incluye esos documentos, así que una ausencia («no se
   > menciona…») puede no ser cierta. Probá volver a correr la investigación
   > con un modelo más capaz.»
2. **Say how much was lost**: number of fragments/documents in the failed
   batch, and ideally which documents, so the researcher can check them by
   hand. Needs the engine to put the batch's item ids or count in the
   limitation/warning payload.
3. **Map internal role names to plain labels** in the desktop UI
   (`asistente_archivo` → "lectura de documentos", etc.) through
   `lib/i18n.ts` (es + en). Never show the raw role id.
4. **Deduplicate**: when a role warning and the "no parseable" limitation come
   from the same batch, show one line, not two.
5. **Keep the technical detail reachable but out of the way**: the raw error
   (`la salida no contiene JSON recuperable`) goes into a collapsed "detalle
   técnico" or the diagnostic logs, not in the main list.
6. Optional: tell apart "answer cut off for length" from "answer was not in
   the expected format", since the fix differs (smaller batches vs. a stronger
   model). The raw output is already stored as artifact
   `output_asistente_archivo` on the job.

## Open questions

- How much of this changes in the engine (payload, wording) vs. only in the
  desktop UI (wording, mapping, dedup). The UI-only part (3, 4, 5) can ship
  first without touching the engine.
- Same treatment for the other roles that can fall back to default
  (`prospeccion`, verifier, etc.).
- hlab-platform parity if it shows the same report.

## Resolution (2026-11-10)

Checked against EntropIA-Agent `441bc269` (the revision the desktop pins, same as
its `main`) and the desktop `main`.

Already done in the engine, shipped in desktop 1.5.0 (pin `29929f5d`, 2026-11-06):

- `ce6268b7`: a failed archive batch is retried with 48, 24 and 12 KB inputs (one
  `archive_retry` event per failed attempt). If no size closes it, the job pauses
  with a `research_error` naming the batch and its documents, and resuming retries
  that same batch. A batch is no longer recorded without claims and skipped, so
  the false-negative risk is gone, and the string `salida de archivo no parseable;
  lote registrado sin claims` no longer exists. Covers point 2.
- `aa3bd98b`: output cap of 16384 tokens and `finish_reason` exposed, so a cut-off
  answer is told apart from a malformed one. Covers point 6.
- `20a03793`: each failed archive attempt leaves one `role_warning` (`el lote de
  archivo necesitó reintentos con lotes más chicos`); the engine aggregates role
  warnings by role and error with a `times` count.
- The desktop already showed the pause (`research_error`) and the retries
  (`archive_retry`) in `InvestigationView.svelte`.

Done in the desktop on branch `fix/research-readable-degradations` (points 1, 3,
4 and 5, for every role, as the owner decided on 2026-11-10):

- `lib/research-degradations.ts` turns each role warning into one line: the
  step's plain name instead of its id (es and en; an unknown role gets a generic
  name, and a role id quoted inside the engine text is replaced too). An
  unreadable or cut-off answer says what it means for the report and what to
  try; a retried archive batch says the documents were still read.
- A cut-off answer points to the fix the person controls: raise the «Tope de
  salida» (`max_tokens_salida`, 1024 to 131072) in Configuración › Agente,
  quoting the cap the engine reports, or use a model that allows longer answers.
- The raw engine error of an unreadable or cut-off answer goes into a folded
  «Detalle técnico»; the «Degradación del pipeline:» prefix is gone.
- Lines are deduplicated by their final message.

Still open, not part of this task: hlab-platform parity.
