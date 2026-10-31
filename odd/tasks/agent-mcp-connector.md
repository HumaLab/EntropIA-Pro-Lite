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
  "appVersion":"x.y.z","variant":"pro"|"lite","profile":null|"name"}`. Written when
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
- [x] T6 — Contract and inventory docs under `integrations/agents/`: protocol version, compatibility policy, scopes, error codes, domain inventory. S10. Route: delegated worker (passive docs: structural checks, every tool and error code grepped in source). Commit: (T6 commit)
- [ ] T7 — End-to-end proof: connector → app endpoint against an isolated archive; record what remains unproven (four real clients). S1, S12. Route: inline + verify. Commit: —

## Delivery

- Slices exceed ~400 lines per commit (T5 alone is ~2.1k with tests). Each task is one
  commit so the branch can be split into chained PRs per commit (T1, T2, T5, ...).

## Log

- L1 (2026-10-31, owner): "está bien no es necesario el commit arrancá con la implementación del plan"
- L2 (2026-10-31, owner): chose "SDK oficial rmcp (recomendado)" over a hand-rolled JSON-RPC subset.
- L3 (2026-10-31): rmcp 3.8.0 declares rust-version 1.88; repo toolchain is 1.90.0. hyper 1.9, hyper-util, tower 0.5 and schemars 1.x are already in `Cargo.lock`.
