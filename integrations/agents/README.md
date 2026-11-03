# Agents

An external assistant — Claude Code, Codex, OpenCode, Pi — can read this
installed EntropIA archive through a loopback MCP endpoint, once you pair it in
**Configuración › Asistentes**. If you allow it there, it can also prepare and
control processing jobs (OCR, embeddings, entities, triples, schemas) and ask to
start paid work, which you approve in the app. The endpoint is **local-only and
off by default**, a new assistant can only read, and it is the same code in Pro
and Lite.

Status: the endpoint and the connector exist and were exercised end to end
against a real Lite debug build (isolated dev profile, one seeded client):
OpenCode, Claude Code and Pi connected and listed the five tools, and a
non-interactive Codex session (`codex exec`) called `entropia_status` through the
connector. The connector now ships **with every installer** — Pro and
Lite, Windows NSIS/MSI and MSIX, Linux `.deb`, macOS `.app`/`.dmg` — and Settings ›
Asistentes names its absolute path. See the **Verified** column under “Pair a
client” for what exactly was and was not proven, and “Where the connector is
installed” for the per-platform location.

## Pieces

| Piece | What it does | State |
|---|---|---|
| App endpoint (`apps/desktop/src-tauri/src/automation/`) | Serves MCP Streamable HTTP on `127.0.0.1` at `/mcp`, authenticates each client by bearer token, offers the read tools and writes the local audit trail | Implemented |
| Connector (`crates/entropia-agent-connector`) | Small stdio↔loopback relay. Reads the app's discovery file, reads the client token from the OS keyring (or the owner-only file when the machine has no keyring), forwards JSON-RPC. Stdout carries protocol messages only | Implemented, bundled with the installers |
| Skill (`integrations/agents/skill/`) | Teaches an agent how to investigate with the tools | Planned (stage 6) |

The app owns the archive and is the only writer. Several clients can connect to
the same running app; the app never starts a second instance for an agent, and it
may be minimized while they work. If the endpoint is off, nothing listens.

## Where the connector is installed

Every installer bundles the connector beside the app executable, so the panel can
name an absolute path instead of hoping the binary is on `PATH`. Settings ›
Asistentes shows that path (or says it is missing) above the lines it copies, and
every recipe uses it, quoted for the platform.

| Build | Installed location | What the lines name |
|---|---|---|
| Windows NSIS/MSI (Pro, Lite) | `entropia-agent-connector.exe` (Lite) or `entropia-pro-agent-connector.exe` (Pro) in the install directory, beside the app exe | that absolute path, in PowerShell single quotes (a literal string; a `'` inside is doubled), with the call operator `&` in front on the standalone command line |
| Microsoft Store MSIX (Lite) | packaged beside the app exe, reached through the execution alias `entropia-agent-connector.exe` | the alias name |
| Linux `.deb` (Pro, Lite) | `/usr/bin/entropia-agent-connector` (Lite) or `/usr/bin/entropia-pro-agent-connector` (Pro) | the absolute path, single-quoted when it contains spaces |
| macOS `.app`/`.dmg` (Pro, Lite) | `EntropIA ….app/Contents/MacOS/entropia-agent-connector` (Pro: `entropia-pro-agent-connector`) | the absolute path, single-quoted when it contains spaces |

Pro has its own file name because the Lite and Pro `.deb` packages both install
into `/usr/bin` and can be installed together: one shared name would make `dpkg`
refuse the second package. Both names are the same binary; either one talks to
whichever EntropIA is running.

The MSIX alias is not a convenience. The packaged copy lives under the versioned
`WindowsApps` directory, which Windows refuses to launch from an ordinary command
line, so the repacked manifest declares `uap3:AppExecutionAlias`
`entropia-agent-connector.exe`. Launching through the alias also starts the
connector in package context, which is what applies the Store package's
data-directory redirection.

A build that carried no connector (`pnpm exec tauri dev`, or an installer built
without the overlay below) reports “no está instalado en esta compilación” and
falls back to the bare `entropia-agent-connector`, which only works if it is on
`PATH`.

## Pair a client

1. Settings → **Asistentes** → turn on **"Permitir asistentes externos"**. The
   panel shows where the endpoint is listening.
2. Under **"Conectar un asistente"**, type a name and press **"Conectar"**.
3. Copy one of the lines the panel shows for that client. Each line names the
   client id, never a token — the credential stays in the OS keyring or, on a
   machine with no keyring, in a file only your user can read. The client list
   says which of the two it is (**Llavero del sistema** or **Archivo
   protegido**).

The lines the app generates (`apps/desktop/src/lib/agent-access.ts`) name the
connector installed for **that build**, never a name you must have on `PATH`:
the path from `agent_connector_location` (quoted for the platform), or the MSIX
alias. Below, `<connector>` stands for that program — `entropia-agent-connector`
in Lite, `entropia-pro-agent-connector` in Pro — and the client id is
abbreviated. A Pro-only machine has no `entropia-agent-connector` anywhere, so
copy the panel's line instead of typing the Lite name. The **Verified** column
describes the Lite debug run, where `<connector>` was
`entropia-agent-connector`.

| Client | Line | Verified |
|---|---|---|
| Claude Code | `claude mcp add entropia -- <connector> --client cl_…` | Linux and Windows — `claude mcp list` reported **Connected**; Claude Code 2.1 speaks MCP `2026-07-28` (`server/discover`, then `tools/list`) |
| Codex | `codex mcp add entropia -- <connector> --client cl_…` | Linux — `codex exec` called `entropia_status`. Windows — handshake and `tools/list` completed (the model call itself failed on that machine's account). Codex starts MCP servers with a reduced environment; the connector copes on both |
| OpenCode | `{"mcp":{"entropia":{"type":"local","command":["<connector>","--client","cl_…"]}}}` | Linux and Windows — `opencode mcp list` reported **connected** |
| Pi | `{"mcpServers":{"entropia":{"command":"<connector>","args":["--client","cl_…"]}}}` | Linux and Windows — `pi mcp list` reported **connected** with 5 tools |

"No token in the config" held for all four: each config names only the client id.

The same panel lists every paired client with its creation date and last use, and
**"Revocar"** (behind a confirmation) makes that credential useless on the next
request, even for a copy someone saved earlier.

## What an assistant can read

Besides listing collections and items and searching by item, a paired assistant
reads what the documents say: `documents_get_text` returns a document's text page
by page in windows (native text, OCR or the transcription of an audio), and
`search_passages` finds passages across the archive. Both give each passage its
page and character offsets plus a hash of the page text, so the assistant can cite
precisely and notice when a text changed. Passages exist only for documents
processed with embeddings; the answer says how many are.

## Let an assistant process documents

A paired assistant can only read. Under each assistant in Settings › Asistentes,
two boxes add more; granting one asks for a confirmation, unticking it applies at
once and the next request is refused.

| Box | Scope | What the assistant can do |
|---|---|---|
| **Preparar procesamiento y editar el corpus** | `local_write` | Prepare a paused job over collections or items (`processing_prepare`), pause or cancel it (`processing_control`); create and rename collections (`collections_create`, `collections_update`); edit item titles and metadata (`items_update`); propose a correction of a page's recognized text (`documents_text_update`, which you approve, and «Restaurar original» undoes); propose importing files from your «Carpetas de entrada» (`import_sources_list`, `documents_import`, which you approve seeing the list of files); propose deleting documents (`documents_delete`) or a whole collection (`collections_delete`), which you approve seeing how many pages, notes and annotations are lost. Nothing is spent. |
| **Pedir procesamiento pago** | `remote_spend` | Ask to start, resume or retry a job (`processing_start`, `processing_control`). Each request waits for you. |

When an assistant asks to start paid work, EntropIA comes to the front with a
dialog: who asks, which operations, over which collections and how many files.
Only **Aprobar** runs it; **Rechazar** is focused first, Escape rejects, and a
request nobody answers expires after ten minutes. If the job changed between the
request and your answer (another priority, a pause), nothing runs and the
assistant has to ask again. Approvals are not kept across a restart.

A request to delete looks the same, with **Borrar** instead of **Aprobar**: it
names the collection and the documents, and says what is lost («Se pierden: 3
documentos, 14 páginas, 2 notas, 5 anotaciones y su texto reconocido») and that it
cannot be undone and deletes on every device. If a document changed, or a
collection gained or lost one, between the request and your answer, nothing is
deleted.

The flow an assistant follows: `processing_prepare` → poll `jobs_get` until the
job is `ready` (planning counts the pages and units) → `processing_start` with
that `revision` → poll `approvals_status` until `approved` → follow progress with
`jobs_get`. Jobs it prepares show up in Procesamiento like your own. See
`PROTOCOL.md` for every argument and error code.

## Try it against an isolated dev profile

Never point a development run at the real archive. A debug build with
`ENTROPIA_DEV_PROFILE` uses its own data directory, turns sync off and gives the
connector its own keyring entries, so the installed app's token is neither read
nor replaced.

```bash
(cd apps/desktop && ENTROPIA_DEV_PROFILE=agents pnpm exec tauri dev)
# check the startup line: [setup] profile=dev:agents data_dir=… sync=disabled
```

Pair a client inside that window, then, from the repository root, run the
connector against the same profile:

```bash
cargo run --manifest-path crates/entropia-agent-connector/Cargo.toml -- \
  --client cl_… --profile agents
```

The connector reads `<data>/com.entropia.shared/dev-profiles/agents/agent-endpoint.json`
and speaks newline-delimited JSON-RPC on stdin/stdout, so you can drive it by hand:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"hand","version":"0"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"entropia_status","arguments":{}}}
```

`ENTROPIA_DATA_DIR=<path>` replaces only the shared directory
(`<data>/com.entropia.shared`); it does not stand in for the profile. With
`--profile agents` the connector still reads
`<path>/dev-profiles/agents/agent-endpoint.json`, and without `--profile` it reads
`<path>/agent-endpoint.json` (useful in tests). Diagnostics go to stderr; stdout carries only protocol
lines.

### Build the bundled sidecar locally

The installers carry the connector through an explicit config overlay
(`apps/desktop/src-tauri/tauri.connector.conf.json` for Lite,
`tauri.connector.pro.conf.json` for Pro; `bundle.externalBin` only),
never through `tauri.conf.json`: `tauri-build` copies every `externalBin` during
any `cargo build`, `cargo test` or `cargo clippy` of the app crate and fails when
the file is missing, so a plain developer build must not depend on a compiled
connector. Build the file the overlay names, then pass the overlay to the
bundler:

```bash
pnpm connector:build                      # host triple from `rustc -vV`
pnpm connector:build -- --target aarch64-apple-darwin   # cross build
pnpm connector:build -- --target universal-apple-darwin # both Apple arches + lipo
pnpm connector:build -- --variant pro     # Pro sidecar name; pair with tauri.connector.pro.conf.json

cd apps/desktop
pnpm exec tauri build --config src-tauri/tauri.connector.conf.json \
  --config src-tauri/tauri.lite.conf.json   # plus the variant/platform overlays you need
```

`pnpm connector:build` writes
`apps/desktop/src-tauri/binaries/entropia-agent-connector-<triple>[.exe]`
(`entropia-pro-agent-connector-…` with `--variant pro`; gitignored);
the installer workflows run it before every Tauri build, `smoke-linux.sh` asserts
the `.deb` really installed it, and the MSIX repack asserts the packaged bytes and
the alias.

A failure you will recognize: `app_absent` means the app is not running, the
switch is off (a disabled endpoint is stopped and its discovery file removed), or
the process holding the advertised port could not prove the discovery file's
instance key; `unauthorized` means the client was revoked or was never paired;
`protocol_incompatible` means the discovery file names another protocol version or
has no instance key (update app and connector); `credential_missing` means there is
no token for that client id (not paired, or revoked); `credential_insecure` means
the fallback token file is readable by other users, so fix its permissions
(`chmod 600`) or pair the assistant again; `credential_store_unavailable` means the
connector could not reach the OS keyring from the process the agent started (log
in to the desktop session with the keyring unlocked, or pass the agent
`DBUS_SESSION_BUS_ADDRESS`).

### Known issues on Windows

- **Antivirus sandboxing delays every start.** Avast's Autosandbox (and similar
  "unknown program" sandboxes) holds an unsigned program that keeps running until
  its emulation times out: on a test machine the connector took about **26 s** to
  start, on every launch, and back to ~0 s with the antivirus paused. The
  installers do not sign the connector yet (see `CODE_SIGNING.md`). Clients with a
  short MCP startup limit give up first: Codex defaults to 10 s, so set
  `startup_timeout_sec = 60` under `[mcp_servers.entropia]` in `config.toml`, or
  exclude the EntropIA install folder in the antivirus. The Store (MSIX) build is
  signed by the Store on publication; whether that avoids the sandbox was not
  measured.
- **SSH sessions have no keyring.** Windows denies the Credential Manager to a
  key-authenticated SSH logon (`ERROR_NO_SUCH_LOGON_SESSION`); run the agent in the
  desktop session, or the connector answers `credential_store_unavailable` unless
  the fallback token file exists.

## Read next

- [`PROTOCOL.md`](PROTOCOL.md) — protocol version and compatibility policy,
  transport and guard order, discovery file, credentials, scopes, the twenty-two
  tools and the approvals, every error code, and the audit format.
- [`INVENTORY.md`](INVENTORY.md) — what each product operation maps to today,
  which tool would expose it, and what is deliberately not exposed.
