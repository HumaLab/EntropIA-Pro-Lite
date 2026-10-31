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
  OS keyring (service `com.entropia.agent-connector`, account = client id, prefixed
  by the dev profile when one is active).
- D3 — Discovery: while enabled the app writes `agent-endpoint.json` (protocol
  version, port, pid, app version, variant) next to the archive; the connector reads
  it. Port is ephemeral on 127.0.0.1.
- D4 — Reads go through a dedicated `query_only` connection, never the UI
  connection's write path.
- D5 — Bundling the connector into installers is stage 6 (delivery); this tranche
  builds it as a standalone crate.

## Tasks

- [x] T1 — Access store: client records, token mint/verify/revoke, keyring seam. S4, S8. Route: inline, TDD. Risk: item 2. Commit: (this commit)
- [ ] T2 — MCP endpoint: rmcp Streamable HTTP on loopback, Host/Origin/bearer guard, discovery file, start/stop. S2, S3, S11. Route: inline, TDD. Risk: items 2, 5. Commit: —
- [ ] T3 — Read-only tools: `entropia_status`, `collections_list`, `items_list`, `items_get`, `search_items`; pagination, scope check, audit log. S6, S7, S9. Route: inline, TDD. Commit: —
- [ ] T4 — App surface: Tauri commands + Settings "Asistentes" panel (enable, add client, revoke, status). S1, S4. Route: inline, TDD. Commit: —
- [ ] T5 — Connector crate `crates/entropia-agent-connector`: discovery, keyring token, stdio relay, actionable errors. S1, S5, S7. Route: inline, TDD. Commit: —
- [ ] T6 — Contract and inventory docs under `integrations/agents/`: protocol version, compatibility policy, scopes, error codes, domain inventory. S10. Route: inline. Commit: —
- [ ] T7 — End-to-end proof: connector → app endpoint against an isolated archive; record what remains unproven (four real clients). S1, S12. Route: inline + verify. Commit: —

## Log

- L1 (2026-10-31, owner): "está bien no es necesario el commit arrancá con la implementación del plan"
- L2 (2026-10-31, owner): chose "SDK oficial rmcp (recomendado)" over a hand-rolled JSON-RPC subset.
- L3 (2026-10-31): rmcp 3.8.0 declares rust-version 1.88; repo toolchain is 1.90.0. hyper 1.9, hyper-util, tower 0.5 and schemars 1.x are already in `Cargo.lock`.
