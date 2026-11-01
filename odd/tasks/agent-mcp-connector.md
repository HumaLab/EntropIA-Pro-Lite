# Agent MCP connector — stages 1 and 2 of PLAN-AGENTES-ENTROPIA.md

Source plan: `PLAN-AGENTES-ENTROPIA.md` (repo root, untracked by the owner's choice).
This tranche covers stage 1 (contracts, inventory) and stage 2 (connection and
permissions, read-only tools). Stages 3-6 are later tranches.

## Specs

- S1 — Stage 2 exit (plan, "Plan de ejecución", §2): "Endpoint local, autenticación, emparejamiento, conector stdio, descubrimiento de instancia y errores humanos. Empezar con diagnóstico y lectura. Salida: los cuatro clientes consultan el mismo archivo; un cliente revocado y una web ajena no acceden, tampoco con una copia de la credencial obtenida antes de revocarlo; ninguna configuración de cliente contiene el token en texto plano."
- S2 — Endpoint (plan, "Arquitectura propuesta" 3): "Endpoint MCP Streamable HTTP en loopback, habilitado explícitamente. Autenticación por cliente, emparejamiento y revocación desde la aplicación."
- S3 — Loopback hardening (plan, "Seguridad y confianza"): "Loopback exclusivo; validar Origin/Host, autenticar incluso peticiones sin Origin y no tratar IDs de sesión como credenciales. Sin exposición LAN por defecto."
- S4 — Credentials (plan, "Seguridad y confianza"): "Emparejamiento visible, credenciales por cliente, mínimo privilegio y revocación; estado de conexión visible en la app." and "El conector guarda la credencial del cliente en el llavero del sistema o, si no hay llavero disponible, en un archivo legible solo por el usuario; nunca como token en texto plano dentro de la configuración del agente (`mcp.json`, `config.toml`, `.mcp.json`). La revocación invalida la credencial en EntropIA, de modo que una copia exfiltrada deja de servir."
- S5 — Connector (plan, "Arquitectura propuesta" 4): "Un conector nativo pequeño, distribuido con EntropIA, que ofrece stdio al agente y conecta al endpoint local. Resuelve ubicación, versión y estado de la app. No abre SQLite ni ejecuta modelos por su cuenta. Su salida estándar contiene solo mensajes del protocolo."
- S6 — Tool contract (plan, "Contrato operativo"): "Herramientas pequeñas por intención de negocio, con schemas estrictos; no `execute_sql`, `invoke_any`, shell ni `eval` público." and "Resultados con IDs estables, revisiones, paginación, procedencia y advertencias. Lecturas extensas por páginas/pasajes, nunca todo el archivo por defecto."
- S7 — Errors (plan, "Contrato operativo"): "Errores accionables: app ausente, conexión pendiente, permisos, proveedor no configurado, capacidad no disponible, presupuesto, conflicto de revisión."
- S8 — Scopes (plan, "Seguridad y confianza"): "Alcances por colección, archivos y acción: lectura, modificación local, consumo remoto, escritura externa y administración."
- S9 — Audit (plan, "Seguridad y confianza"): "Auditoría local de actor, acción, objetos, aprobación, estado y costo conocido, con redacción y retención; no volcar contenido sensible o tokens."
- S10 — Stage 1 exit (plan, §1): "Salida: ninguna función de producto sin mapeo y ninguna promesa de capacidad inexistente." and "Definir la versión del protocolo y la política de compatibilidad entre app, conector y skill; no crear un repositorio separado."
- S11 — Single owner (plan, "Arquitectura propuesta"): "La app será el propietario del archivo. Varios clientes pueden conectarse a ella; no se levanta otra instancia escritora."
- S12 — Test isolation (plan, criterion 10): "Pruebas con perfil aislado y cuentas de prueba; nunca aplicar migraciones o escrituras Zotero a datos reales para validar la integración."

## Decisions

- D1 — MCP stack: official Rust SDK `rmcp` (owner choice, L2), Streamable HTTP server
  over tower/hyper inside the app. The connector is a protocol-agnostic stdio↔HTTP
  relay, so it needs no SDK.
- D2 — No migration: access state (enabled flag, client records with token SHA-256)
  lives in a local, unsynced `agent-access.json` in the data dir. Tokens live in the
  OS keyring (service `com.entropia.agent-connector`, account = client id; with a dev
  profile the service becomes `com.entropia.agent-connector (dev profile)` and the
  account `<id>@dev-profile:<name>`).
- D3 — Discovery: while enabled the app writes `agent-endpoint.json` (protocol
  version, port, pid, app version, variant) next to the archive; the connector reads
  it. Port is ephemeral on 127.0.0.1.
- D4 — Reads go through a dedicated `query_only` connection, never the UI
  connection's write path.
- D5 — Bundling the connector into installers is stage 6 (delivery); this tranche
  builds it as a standalone crate.
- D6 — Endpoint request guard, in this order, before rmcp sees the request: path must
  be `/mcp` (404); `Host` must be `127.0.0.1:<port>` or `localhost:<port>` (403
  `forbidden_host`); any `Origin` header is refused (403 `forbidden_origin`: only
  browsers send it, and no browser page is a legitimate caller); `Authorization:
  Bearer <token>` must authenticate through `AgentAccess::authenticate` on every
  request (401 `unauthorized`, with `WWW-Authenticate: Bearer`). No session caches
  identity, so revocation applies to the next request (S1, S3). The authenticated
  `Principal` goes into the request extensions; rmcp copies `http::request::Parts`
  into the tool `RequestContext`. rmcp runs stateless (`legacy_session_mode: false`),
  `json_response: true`, loopback `allowed_hosts`, `enforce_origin_validation()`,
  1 MiB body cap. Bind `127.0.0.1:0` only.
- D7 — Discovery file `agent-endpoint.json` (0600, atomic write) in the data dir:
  `{"protocol":"entropia-agent/1","mcpPath":"/mcp","port":N,"pid":N,
  "appVersion":"x.y.z","variant":"pro"|"lite","profile":null|"name","instanceKey":"<64 hex>"}`
  (`instanceKey` added by D12). Written when
  the endpoint starts, removed when it stops. Variant is `cfg!(feature = "local-ml")`.
- D8 — Lifecycle: the app loads `AgentAccess` at setup (data dir, `KeyringSecretStore`
  with `dev_profile::active()`), manages it, and starts the endpoint when enabled.
  A start failure is logged, never fatal to startup. Toggling enabled starts/stops it.
  Same code in Pro and Lite.
- D10 — Read-only tools (T3). Reads use a dedicated connection opened with
  `db::open::open_archive_connection(db_path)` plus `PRAGMA query_only = ON`, never
  the UI connection. Every tool requires `Scope::Read` (else tool error
  `forbidden_scope`). Lists page with an opaque `cursor` and `limit` (default 50,
  max 200) and return `nextCursor` (null at the end); records carry stable `id` and
  `updatedAt` as revision. Tools: `entropia_status` (adds archive counts and the tool
  list), `collections_list`, `items_list {collectionId?}`, `items_get {itemId}`
  (metadata + assets, no full text), `search_items {query, collectionId?, limit?}`
  through `nlp::fts`. Unknown ids → tool error `not_found`. Audit: one JSON line per
  call in `<cache>/logs/agent-audit.jsonl` with time, client id and name, tool,
  status, error code and object ids; never queries or other free text (query length
  only), never tokens; rotated past 5 MiB keeping one previous file.
- D11 — App surface (T4). Tauri commands `agent_access_status` → `{enabled, running,
  port, clients}`, `agent_access_set_enabled {enabled}` → status (persists, then starts
  or stops the endpoint), `agent_client_create {name}` → `{client, status}`,
  `agent_client_revoke {clientId}` → status. Each registered in `generate_handler!`,
  `build.rs` `APP_COMMANDS` and `capabilities/default.json` (`acl_manifest_guard`).
  Settings gets an "Asistentes" section, Spanish copy, same in Pro and Lite: a toggle to
  allow external assistants (read-only in this version, this computer only), the
  endpoint state, the paired clients (name, created, last used) each with "Revocar"
  behind a confirmation, and "Conectar un asistente" (name → create) which then shows
  the client id and the connector command `entropia-agent-connector --client <id>`
  with one ready-to-copy line per client (Claude Code, Codex, OpenCode, Pi). No token
  is ever shown or copied. Bundling the connector binary is stage 6, so the panel
  says the connector must be installed.
- D12 — Instance proof before any token leaves the connector (fixes verify W1: a
  stale `agent-endpoint.json` after a crash could point the connector at another
  process, possibly another OS user, holding the port). At each start the app mints a
  random 32-byte `instanceKey` and writes it into the discovery file (0600; never
  sent over HTTP). Before sending a token to a port it has not verified in this
  process, the connector sends `POST /mcp` with header
  `X-Entropia-Challenge: <32 random bytes hex>` and no `Authorization`; the guard
  answers that request (after the path and Host checks, before the Origin and bearer
  checks are relevant) with 401 plus `X-Entropia-Proof: hex(HMAC-SHA256(instanceKey,
  challenge))`. The connector compares in constant time; mismatch or absence →
  `app_absent` (detail: instance not verified) and the token is never sent. The
  verified `(port, instanceKey)` pair is cached until either changes. HMAC is built
  on `sha2` (no new HMAC crate) and pinned by an RFC 4231 test vector on both sides.
- D13 — Owner decisions on the stage 1 open items (L7): approvals by app dialog,
  bound to plan and client, single-use, 10-minute expiry; provider keys never via
  agents; backup restore human-only in the app; local models/runtime by plan →
  approval → job; Navegador gets its own `web` scope (added to `Scope`, not granted);
  no generic `llm_run`; Writing agent actions only as reviewable proposals.
- D14 — Token file fallback (T9, S4 second clause). Only when the OS credential store
  is unavailable (`credential_store_unavailable`, i.e. keyring `PlatformFailure` /
  `NoStorageAccess`), the app writes the token to
  `<data dir>/agent-tokens/<client id>.token` (directory 0700, file 0600 on Unix,
  atomic write; on Windows the per-user profile ACL applies). Any other keyring error
  still fails pairing. The client record gains a non-secret `tokenStore:
  "keyring"|"file"` shown in Settings › Asistentes. Revocation removes the record first,
  then the keyring entry and the file. The connector reads the keyring first and, on
  no entry or an unavailable store, the file in the same (profile-aware) data dir; on
  Unix it refuses a file readable by group or others (`credential_insecure`, -32006).
  The token directory must never enter backups, sync, or the asset sweep.
- D15 — Bundling (T11). `tauri-build` copies every `bundle.externalBin` during any
  `cargo build/test/clippy` and fails when the file is missing
  (tauri-build 2.7.4 `src/lib.rs:735`), so the connector lives only in an explicit
  overlay `src-tauri/tauri.connector.conf.json` (`bundle.externalBin:
  ["binaries/entropia-agent-connector"]`), never in `tauri.conf.json` or an
  auto-merged `tauri.<platform>.conf.json`. A script builds it first
  (`cargo build --release --locked`, per target triple; universal macOS = both
  arches + `lipo`) into the gitignored `src-tauri/binaries/` as
  `entropia-agent-connector-<triple>[.exe]`. Every installer build adds the overlay:
  release.yml Pro matrix and build-lite, lite-preview.yml macOS and Linux,
  windows-pro-validation.yml. MSIX (repacked, not a Tauri bundle) adds the exe and a
  `uap3:AppExecutionAlias` `entropia-agent-connector.exe`, so agents launch it inside
  the package context (which also applies MSIX data-dir redirection); asserted in the
  script. Settings › Asistentes shows the installed absolute path (next to the main
  executable; the alias name under MSIX), quoted/escaped correctly for spaces and
  non-ASCII in every recipe; without an installed connector it keeps the bare
  command and says it is not installed. smoke-linux checks the installed connector.
- D16 — Pro installs its connector as `entropia-pro-agent-connector`; Lite keeps
  `entropia-agent-connector` (and the MSIX alias). Found by inspecting the locally built
  Lite `.deb` (L8): both `.deb` packages install into `/usr/bin`, Lite and Pro may be
  installed together (shared archive, no `Conflicts`), so one shared name would make
  dpkg refuse the second. Pro builds pass `--variant pro` and
  `tauri.connector.pro.conf.json`; the app picks the stem by `cfg!(feature = "local-ml")`.
- D9 — Connector (`crates/entropia-agent-connector`, standalone crate with its own
  committed `Cargo.lock`, binary `entropia-agent-connector`):
  - CLI: `entropia-agent-connector --client <id> [--profile <name>]`, `--version`.
    Agent configs name only the client id, never a token (S4).
  - Data dir: `dirs::data_dir()/com.entropia.shared`, plus `dev-profiles/<name>` with
    `--profile` (same name rule as `dev_profile.rs`); `ENTROPIA_DATA_DIR` overrides it
    (tests; MSIX redirection is a stage 6 item).
  - Token: keyring `(service, account)` computed exactly like
    `automation::access::keyring_target` (test pins the strings).
  - Relay: newline-delimited JSON-RPC on stdin → one POST per message to
    `http://127.0.0.1:<port>/mcp` (port re-read from `agent-endpoint.json` per
    message, so an app restart is picked up), with `Authorization: Bearer`,
    `Accept: application/json, text/event-stream`, and `MCP-Protocol-Version` once
    `initialize` negotiated one. 202 → nothing; `application/json` → one stdout line;
    `text/event-stream` → one line per `data:` event. Stdout carries only protocol
    lines, serialized through one writer; diagnostics go to stderr (S5).
  - Errors (S7) become JSON-RPC error responses for requests with an id (code
    -32000 family, message in Spanish, `data.code` stable): `app_absent` (no
    discovery file, or connection refused), `unauthorized` (401: not paired or
    revoked), `forbidden` (403), `protocol_incompatible` (discovery `protocol` is not
    `entropia-agent/1`), `credential_missing` (no keyring entry for the client id),
    parse error -32700 for a malformed line. Notifications that fail are logged to
    stderr only.
  - Not in this tranche: file fallback when no keyring exists (S4 second clause,
    both sides); bundling (D5); CI job for the crate (delivery, owner decision).

## Tasks

- [x] T1 — Access store: client records, token mint/verify/revoke, keyring seam. S4, S8. Route: inline, TDD. Risk: item 2. Commit: cf3836cc
- [x] T2 — MCP endpoint: rmcp Streamable HTTP on loopback, Host/Origin/bearer guard, discovery file, start/stop. S2, S3, S11. Route: delegated worker, TDD (RED 10 failing → GREEN 23). Risk: items 2, 5. Commit: c949ce52
- [x] T3 — Read-only tools: `entropia_status`, `collections_list`, `items_list`, `items_get`, `search_items`; pagination, scope check, audit log. S6, S7, S9. Route: delegated worker, TDD (RED 13 failing → GREEN 43). Deviations: limit > 200 refused (`invalid_argument`); extra codes `unavailable`, `internal_error`; `search_items` returns `matches` instead of a cursor; timestamps are unix millis. Commit: 89c2e36b
- [x] T4 — App surface: Tauri commands + Settings "Asistentes" panel (enable, add client, revoke, status). S1, S4. Route: delegated worker, TDD (RED 6 Rust + missing component → GREEN 6 Rust, 7 Vitest Pro and Lite). Commit: 92c92f9c
- [x] T5 — Connector crate `crates/entropia-agent-connector`: discovery, keyring token, stdio relay, actionable errors. S1, S5, S7. Route: delegated worker, TDD (RED 0/32 → GREEN 32). Commit: 1432f913
- [x] T6 — Contract and inventory docs under `integrations/agents/`: protocol version, compatibility policy, scopes, error codes, domain inventory. S10. Route: delegated worker (passive docs: structural checks, every tool and error code grepped in source). Commit: d3cf1e08
- [x] T8 — Verify follow-ups: D12 instance proof (W1); JSON-RPC error for an id'd request answered with empty 2xx or non-JSON (W2, S7); tests pinning GET/DELETE → 405 and `/mcp/` → 404; archive-unavailable messages without absolute paths; doc fixes (README client verification status per L4, PROTOCOL `unauthorized` vs disabled endpoint, `/mcp` query wording, D12). S3, S4, S7. Route: delegated worker, TDD (RED compile errors + failing preflight/notification tests → GREEN app 59, connector 45). Commit: e133b5e1
- [x] T7 — End-to-end proof: connector → app endpoint against an isolated archive; record what remains unproven (four real clients). S1, S12. Route: inline + independent verify (gentle-ai-verify: no CRITICAL; W1/W2 fixed in T8). Evidence: L4, L5. Commit: (doc only)
- [x] T9 — Token file fallback when no OS keyring exists (S4 second clause), app and connector. S4. Route: delegated worker, TDD (RED compile + failing backup/UI tests → GREEN app 62 + backup 6, connector 51, Vitest 8 Pro/Lite). Found and fixed: `backup.rs` copied `agent-access.json`/`agent-endpoint.json` into backups. Commit: 5e3ec833
- [x] T10 — CI job that builds, tests and lints `crates/entropia-agent-connector` on Linux, Windows and macOS. Route: inline (YAML parsed; runs on first push). Risk: item 5. Commit: 84843876
- [x] T11 — Bundle the connector with the app installers (Pro and Lite, every platform) and show its installed absolute path in Settings › Asistentes. Route: delegated worker + inline D16 fix, TDD (RED compile/Vitest/node:test → GREEN Rust 70, Vitest 14 Pro/Lite, node:test 10). Evidence: L8. Unverified locally: MSIX repack/alias, macOS universal, Windows, Pro naming under `local-ml` (CI). Risk: item 5. Commit: 01000535
- [x] T12 — Owner decisions on INVENTORY.md "Unmapped / needs decision" (7 items), recorded in INVENTORY.md and here. Route: ask (two questionnaires). Commit: bc0200e1
- [ ] T13 — Publish: push the branch, open chained PRs (one per work unit), native review per slice. Route: inline. Commit: —

## Delivery

- Slices exceed ~400 lines per commit (T5 alone is ~2.1k with tests). Each task is one
  commit so the branch can be split into chained PRs per commit (T1, T2, T5, ...).

## Log

- L1 (2026-10-31, owner): "está bien no es necesario el commit arrancá con la implementación del plan"
- L2 (2026-10-31, owner): chose "SDK oficial rmcp (recomendado)" over a hand-rolled JSON-RPC subset.
- L3 (2026-10-31): rmcp 3.8.0 declares rust-version 1.88; repo toolchain is 1.90.0. hyper 1.9, hyper-util, tower 0.5 and schemars 1.x are already in `Cargo.lock`.
- L4 (2026-10-31, T7 evidence): real Lite debug app via `ENTROPIA_DEV_PROFILE=agents-e2e VITE_LOCAL_ML=0 pnpm exec tauri dev` (startup line `profile=dev:agents-e2e ... sync=disabled`), one client seeded only under the dev-profile keyring service. Discovery file written 0600 with `variant: lite`, `profile: agents-e2e`; listener `127.0.0.1:38661` only. Real connector over stdio: initialize, tools/list (5 tools), `entropia_status` → archive 3 collections / 13 items, client `cl_e2e0000001` scope read; `collections_list` records; unknown item → `not_found`; extra `sql` argument → `invalid_argument`. Raw HTTP: no token 401, fake token 401, `Origin` 403 `forbidden_origin`, rebinding `Host` 403 `forbidden_host`. Unknown client → -32005 `credential_missing`. Real clients with temporary configs (user configs untouched): OpenCode `opencode mcp list` → connected; Claude Code `claude mcp list` → ✔ Connected; Pi `pi mcp list` → connected, 5 tools; Codex `codex mcp add` recipe registers the server (connection not exercised: needs a model session). No client config contains a token. Audit lines carried ids only. After killing the app: stale discovery file remains (SIGKILL skips `RunEvent::Exit`), connector answers -32001 `app_absent`. Keyring test entry removed afterwards.
- L5 (2026-10-31, T7 rerun after D12 at e133b5e1): same isolated profile, fresh seeded client. Discovery file 0600 now carries `instanceKey` (64 hex). Connector stdio run identical to L4 (initialize, 5 tools, status 3/13, not_found, invalid_argument). Real clients connected again: OpenCode (connected), Claude Code (✔ Connected), Pi (connected, 5 tools). Impostor check: discovery `instanceKey` replaced with zeros while the app ran → connector -32001 `app_absent` ("no pude verificar ... (HTTP 401)"), no tool call reached the app (audit unchanged). Codex connection still not exercised (needs a model session). Pro `cargo clippy --features local-ml` could not run here: ocr-rs/llama-cpp build scripts fail because `~/.local/bin/cmake` is a broken Python wrapper (environment, not this change); left to CI. Keyring test entry removed; profile dir `dev-profiles/agents-e2e` and `/tmp/e2e-agents` remain (rm -rf blocked by the safety policy; owner decides).

- L6 (2026-11-01, owner): "continuar con todo lo pendiente punto 2 3 y 4" — read as items 2 (chained PRs for review), 3 (delete the e2e leftovers; done: `dev-profiles/agents-e2e`, `/tmp/e2e-agents`, `/tmp/rmcp-src` removed) and 4 (token file fallback, connector CI job, bundling, the seven INVENTORY decisions) of the previous close report.
- L7 (2026-11-01, owner answers): approvals "Diálogo en la app, un uso (recomendado)"; keys "Nunca (recomendado)"; backups "Solo la persona, en la app (recomendado)"; models/runtime "Plan + aprobación + job (recomendado)"; Navegador "Alcance propio «web» (recomendado)"; llm_run "No exponer (recomendado)"; Writing actions "Solo como propuestas (recomendado)".
- L8 (2026-11-01, T11 evidence): local Lite `.deb` built with `VITE_LOCAL_ML=0 pnpm exec tauri build --config src-tauri/tauri.lite.conf.json --config src-tauri/tauri.lite.linux.conf.json --config src-tauri/tauri.connector.conf.json --bundles deb` (exit 0, 8m52s, 28.52 MiB). `dpkg -c`: `usr/bin/entropia-agent-connector` (6.5 MB) beside `usr/bin/entropia-lite`; extracted copy `--version` → `entropia-agent-connector 0.1.0`, exit 0. Plain `cargo test/clippy` without the overlay unaffected. Inspection revealed the Lite/Pro `/usr/bin` collision → D16. Owner chose `feature-branch-chain` for delivery.

