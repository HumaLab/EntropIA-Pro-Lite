# Agent processing tools — stage 3 of PLAN-AGENTES-ENTROPIA.md, processing first

Lets a paired assistant prepare, get approved, start, observe and control processing batches
(OCR, embeddings, NER, triples, schema extraction) through the loopback MCP endpoint. Builds on
the read-only tranche in `odd/tasks/agent-mcp-connector.md` (merged in v1.2.0).

## Specs

- S1 — Owner request (L1): "bien, entoces tenemos que avanzar con lo pendiente para que la conexion mcp tenga sentido, ahora solo es una funcionalidad poco util si so es lectura, avanzamos???" Context question from the owner just before: "una vez que conecto el mcp de entropia a mi agente que le peudo pedir a mi agente para que haga en entropia, puedo pedirle que envie tal coleccion o tal imagen para OCR???"
- S2 — Owner choice (L2): "Procesamiento primero (recomendado)" — "El agente prepara un lote (OCR, embeddings, etc.) sobre colecciones o ítems, ve costo y alcance, vos aprobás en la app, y sigue/pausa/reintenta el trabajo. Reusa la cola en Rust; no migra la UI." Previewed tools: "processing_prepare (alcance, operaciones, costo)", "aprobación en la app (un uso, 10 min)", "processing_start", "jobs_list / jobs_get", "processing_control (pausar, reanudar, cancelar, reintentar)", and "permisos por asistente en Configuración › Asistentes (lectura / modificación local / gasto)".
- S3 — Operating contract (plan, "Contrato operativo"): "Mutaciones reintentables con claves de idempotencia ligadas a parámetros y cliente. Desconexión no implica cancelación." · "Reutilizar la cola persistente existente donde corresponde; extenderla o integrar otros jobs sin crear un scheduler paralelo de procesamiento." · "Toda operación larga devuelve un ID consultable. Progreso MCP es una mejora, no la única forma de conocer el estado." · "Separar los costos del agente externo de los proveedores invocados por EntropIA. Usar estimaciones explícitas cuando no sea posible calcular importe exacto."
- S4 — Approval and scopes (plan, "Seguridad y confianza"): "Alcances por colección, archivos y acción: lectura, modificación local, consumo remoto, escritura externa y administración." · "Confirmación específica para eliminación masiva, sobrescritura/restauración, gasto relevante, publicación, compartir, cambios de cuenta y escritura Zotero. No preguntar por cada lectura o paso inocuo." · "Preparación devuelve impacto, revisión y destinos; aprobación ligada a ese plan, con caducidad y uso acotado. Un `approved: true` producido por el modelo no es consentimiento humano verificable." · "Documentos y páginas son datos no confiables. Nunca pueden autorizar nuevas herramientas, ampliar scopes o modificar la política; proteger especialmente publicación y envío de contenido."
- S5 — Owner decision on approvals (agent-mcp-connector L7, D13): "Diálogo en la app, un uso (recomendado)"; D13: "approvals by app dialog, bound to plan and client, single-use, 10-minute expiry".
- S6 — TypeScript/Rust boundary (plan): "Adaptadores MCP y UI hacia ese catálogo, sin una segunda implementación de reglas de negocio." · "Reutilizar servicios Rust existentes detrás de adaptadores tipados." · "Toda actualización externa debe refrescar la UI; no sobrescribir contenido de un editor con cambios locales sin guardar."
- S7 — Errors (plan): "Errores accionables: app ausente, conexión pendiente, permisos, proveedor no configurado, capacidad no disponible, presupuesto, conflicto de revisión."
- S8 — Stage 3 exit, processing half (plan §3): "procesar y recuperar tras interrupción sin duplicar documentos ni cargos por reenvío de la misma operación."
- S9 — Audit (plan): "Auditoría local de actor, acción, objetos, aprobación, estado y costo conocido, con redacción y retención; no volcar contenido sensible o tokens."
- S10 — Test isolation (plan, criterion 10): "Pruebas con perfil aislado y cuentas de prueba; nunca aplicar migraciones o escrituras Zotero a datos reales para validar la integración."

## Decisions

- D1 — One implementation: the batch operations in `processing/commands.rs` become `&Connection`-level
  services (the pattern `apply_bibliography_sync_request` already follows); the Tauri commands and the MCP
  tools both call them. No second copy of the rules (S6), same persistent queue (S3).
- D2 — Scopes per client, granted by the owner in Settings › Asistentes behind a confirmation:
  `local_write` (prepare, pause, cancel, priority) and `remote_spend` (ask to start, resume or retry paid
  work). Checked by the endpoint before any archive access (`forbidden_scope`). They live in the existing
  `scopes` array of `agent-access.json`: no new field, no protocol bump. A tool, document or agent can
  never grant a scope (S4).
- D3 — Approval: a tool that would spend never spends by itself. It creates a pending approval bound to
  (client id, batch id, batch revision, digest of the batch's members and operations), in memory, expiring
  10 minutes after creation (S5). The app shows a dialog (client, collections/items, operations, impact,
  estimate) and brings its window forward; "Aprobar" performs the start/resume/retry with the bound
  revision and consumes the approval (single use); "Rechazar", expiry, a changed batch or an app restart
  leave nothing approved. The agent polls `approvals_status`/`jobs_get`. No field the agent sends can mark
  anything approved (S4).
- D4 — Pause and cancel reduce spending, so they need `local_write` and no approval; they target one
  batch, never "all batches". Resume and retry re-start paid work, so they go through D3 like start.
- D5 — Idempotency (S3, S8): `processing_prepare`/retry take an agent `requestId`, stored as
  `agent:<clientId>:<requestId>` in `processing_requests`, so a replay returns the same batch and the same
  id with other parameters is refused. Asking to start the same batch at the same revision returns the
  same pending approval instead of a second dialog.
- D6 — Impact and cost (S3): prepare reports the files in scope and the operations; once planning finishes,
  `jobs_get` reports the units per kind (`tasksByKind`). Page counts are only known after planning, so no
  USD figure is computed at prepare time: the result says so explicitly instead of guessing (L4). In Lite,
  embeddings, entities, triples and schemas need the OpenRouter key exactly as the Procesamiento tab requires;
  without it prepare answers `provider_not_configured` (S7). Agent costs are not EntropIA's and are never
  reported.
- D7 — No schema migration in this feature (sync `schema_tag`, AGENTS.md): approvals are in memory,
  batches keep `origin='user'` so the Procesamiento tab shows them (it already refreshes on
  `processing:changed`, S6), and which client asked lives in the audit log (S9).

## Tasks

- [x] T1 — Extract `&Connection` services for prepare, start, control, priority, retry, list/get batches and tasks; Tauri commands delegate. S3, S6. Route: inline, TDD (RED: 24 compile errors, services missing → GREEN 11 service tests); start became a conditional update so two starts at one revision run once. Full Rust suite 3298 passed / 0 failed, clippy -D warnings clean. Risk: item 3 (shared code the Procesamiento tab uses). Commit: 1a4f225b
- [x] T2 — Owner grants `local_write`/`remote_spend` per client: `AgentAccess::set_grants` (only those two; `read` and other scopes untouched), `agent_client_set_grants` command (handler, APP_COMMANDS, capability), checkboxes in Asistentes: granting asks for confirmation, withdrawing applies at once. S2, S4. Route: inline, TDD (RED: `Grants`/`set_grants` missing; Vitest 3 failing → GREEN Rust 4 tests, Vitest 22/22 Pro and Lite). Full Rust suite 3298/0, clippy clean, lint, typecheck Pro and Lite clean, svelte-autofixer no issues. Risk: item 2 (permissions). Commit: see git log (feat(agents): let the owner grant processing scopes)
- [x] T3 — Read tools `jobs_list`, `jobs_get` (batch snapshot + paged tasks) through `automation/jobs.rs`, a `query_only` adapter over the T1 services: limits refused outside 1..200, opaque cursors, service errors mapped to stable codes (`not_found`, `invalid_argument`, `revision_conflict`, `invalid_state`, `unavailable`) without leaking paths. S3, S7. Route: inline, TDD (RED: 8 endpoint tests failing with the implementation stashed → GREEN automation 89; found and fixed: "unknown batch state" was mapped to not_found). Full Rust suite 3305/0, clippy clean. Commit: see git log (feat(agents): add jobs_list and jobs_get)
- [x] T4 — `processing_prepare` (local_write): collections and/or items (`prepare_scope_membership`, each file once; collection-only requests keep their old hash so the tab's replays still match), operations, `requestId` stored as `agent:<client>:<id>`, files/operations/revision plus an explicit cost note (D6). Lite refuses text operations without the OpenRouter key (`provider_not_configured`), reading only the stored reference, never the secret. S2, S3, S7, S8. Route: inline, TDD (RED: 17 compile errors → GREEN 110 automation+processing tests incl. the Lite key test). Full Rust suite 3315/0, clippy clean. Risk: item 3 (prepare service shared with the tab). Commit: see git log (feat(agents): add processing_prepare)
- [ ] T5 — Approval core: store, plan digest, expiry, single use, events, window focus, `agent_approvals_*` commands. S4, S5. Route: inline, TDD. Risk: items 2, 4. Commit: —
- [ ] T6 — `processing_start`, `processing_control` (pause, resume, cancel, retry), `approvals_status`; resume/retry through T5. S2, S4, S7, S8. Route: inline, TDD. Risk: item 2. Commit: —
- [ ] T7 — Approval dialog in the app (Svelte, Pro and Lite). S5, S6. Route: inline, TDD (Vitest). Commit: —
- [ ] T8 — Docs: PROTOCOL (tools, scopes, approvals, codes), README, INVENTORY statuses. S7, S9. Route: inline. Commit: —
- [ ] T9 — End-to-end proof on an isolated dev profile with a real client. S10. Route: inline + independent verify. Commit: —

- Slices: commits exceed ~400 authored lines where code moves with its tests (T1 is ~1k lines, mostly the commands' bodies moved into services). Each task is its own commit; the branch goes to `main` as one PR after T9, or split per task if the owner prefers.

## Log

- L1 (2026-11-02, owner): "bien, entoces tenemos que avanzar con lo pendiente para que la conexion mcp tenga sentido, ahora solo es una funcionalidad poco util si so es lectura, avanzamos???"
- L2 (2026-11-02, owner answer): "Procesamiento primero (recomendado)" over "Corpus primero" and "Etapa 3 completa".
- L3 (2026-11-02, exploration): batch commands take only `State<AppDbState>` (no AppHandle); prepare scopes collections only and reports counts, no cost; start fences by `expected_revision` and has no request id; prepare/retry are idempotent by `request_id` + payload hash; control with no batch id acts on every live batch; the UI refreshes on `processing:changed` and polls every 3 s; no Rust-triggered approval exists; clients are always created with `read` only; the only USD estimator is GLM-OCR's in `bibliography/reprocess.rs`.
- L4 (2026-11-02, design): a batch's OCR pages are counted by background planning, not at prepare time, and the only known rate is GLM-OCR's; D6 now reports files, operations and later the planned units instead of a USD estimate it cannot compute. T4 also adds item scope to `prepare_batch` (hash unchanged for collection-only requests, so the tab's replays keep matching).
