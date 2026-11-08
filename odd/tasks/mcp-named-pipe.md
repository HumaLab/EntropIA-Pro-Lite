# MCP connector over a Windows named pipe

Codex sessions get no Entropia tools because an antivirus HTTP shield alters
the connector's loopback HTTP. Branch `feat/mcp-named-pipe`. Root cause in L2.

## Specs

- **S1 The connection must not fail.** "mejorar el funcionamiento intenro apr aque no falle la conexion" — and: "Corregí la causa demostrada, no suprimas el error ni agregues reintentos indiscriminados."
  - Decision (L3): on Windows the connector and the app talk over a local named pipe, which antivirus web shields do not inspect; loopback HTTP stays as the fallback (and the only transport on other systems).
- **S2 Same security guarantees.** The pipe keeps every guarantee of the HTTP endpoint: the instance proof before any token (HMAC challenge, D12), bearer authentication per request, per-client scopes and revocation, owner-only access (no other local user, no remote client), the 1 MiB request limit, and the same MCP behaviour (rmcp streamable HTTP, JSON responses). The pipe name is unguessable and advertised only in the discovery file.
- **S3 Compatibility.** "No registres tokens, credenciales ni contenido privado de las colecciones." / "Mantené stdout exclusivamente para JSON-RPC y diagnósticos en stderr." Connectors already installed must keep reading the discovery file and keep working over HTTP; a new connector with an old app falls back to HTTP.
- **S4 Regression test.** "Agregá una prueba de regresión para el fallo encontrado." A fake endpoint that answers `transfer-encoding: chunked` with a raw (un-chunked) body reproduces the shield; with the pipe advertised the connector must succeed through the pipe.
- **S5 Verification.** "Verificá initialize, notifications/initialized, tools/list y una llamada real de lectura." / "Cerrá la verificación usando Codex CLI: debe descubrir herramientas de Entropia y listar las colecciones mediante una llamada MCP real, no inventar una respuesta." / "No modifiques ni elimines colecciones para probar." / "Si algo sigue fallando, indicá exactamente qué y no lo declares resuelto." — with Avast's web shield active.

## Tasks

| ID | Specs | Route | Status | Commit |
|---|---|---|---|---|
| T1 | S1, S2, S3 | worker A — app side: named pipe listener (Windows) serving the same guarded service, owner-only DACL, remote clients rejected, advertised in discovery, same lifecycle as the TCP listener; server tests | done | (T1 commit) |
| T2 | S1, S2, S3, S4 | worker B — connector side: parse the optional pipe from discovery, transport over the pipe with the same proof + bearer flow, HTTP fallback, regression test for the shield pattern | done | (T2 commit) |
| T3 | S2 | independent verify (Risk item 2: security guard/transport) | pending | |
| T4 | S5 | parent — real Codex CLI with Avast active (dev profile + paired client) | pending | |

## Log

- **L1** (user, verbatim): "otra tarea es mejorar el funcionamiento de las conexiones MCP, por un lado mejorar el funcionmaiento intenro apr aque no falle la conexion y en segundo lugar intentar que todo se resuelva con un clic, o sea que no sea necesario que el usuario copie y pegue el comando correspondiente a los Agentes apra conectarlos, ¿me explico?" followed by the incident report (executable C:\Users\agusn\AppData\Local\Microsoft\WindowsApps\entropia-agent-connector.exe; Codex says it has no Entropia tools; `codex mcp get entropia` enabled stdio; direct PowerShell JSON-RPC works; Codex log "entropia-agent-connector: -32001 app_absent: se cortó la respuesta de http://127.0.0.1:62711/mcp: error decoding response body"; instructions quoted in S1-S5). The one-click setup is a separate task.
- **L2** Root cause (proven 2026-11-07): a connector traced under Codex gets `HTTP 200`, `transfer-encoding: chunked`, `content-type: application/json`, and the body fails at byte 0 with hyper "Invalid chunk size line: missing size digit". The same exact Codex initialize (`id` 0, `experimental.codex/auth-change`, `elicitation.form/url`), the same 22 environment variables, the same exe run by hand get `content-length=145` and succeed (22 tools). Fails without Codex's sandbox too. With Avast's Web Shield paused, `codex exec` discovered the tools and listed 28 collections through a real MCP call. Avast re-frames loopback HTTP responses of processes spawned by Codex (chunked header, raw body). Engram #7036.
- **L3** (user decision): "Canal que el antivirus no toca" — named pipe on Windows, HTTP as fallback.
- **L4** Evidence (explorer): connector `crates/entropia-agent-connector/src/relay.rs` (stdin loop :183-237, forward :334-434 re-reads discovery and credentials per message, unauthenticated HMAC challenge to `/mcp` then POST with bearer; read_reply :436-469 buffers `chunk()`), discovery `src/discovery.rs:11-55` (protocol `entropia-agent/1`, `http://127.0.0.1:<port><mcpPath>`), keyring `src/keyring_source.rs`; reqwest 0.12 rustls, `.no_proxy()`. App `apps/desktop/src-tauri/src/automation/server.rs`: binds `127.0.0.1:0` and advertises discovery (:255-285), exact `/mcp`, Host check including port, Origin refused, bearer per request, 401 with proof header (:429-490), rmcp streamable HTTP stateless with JSON responses and 1 MiB limit (:397-424), hyper HTTP/1 driver with cancellation (:358-375); handshake test `a_paired_client_completes_the_handshake_lists_tools_and_reports_itself` (:894). Connector tests `crates/entropia-agent-connector/tests/relay.rs` (fake endpoint).
