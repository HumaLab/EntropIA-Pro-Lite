# Research: readable message when a batch's model output is unusable

**Status:** pending, not started. Requested by the owner 2026-11-05, after a
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
