# Agents

An external assistant — Claude Code, Codex, OpenCode, Pi — can read this
installed EntropIA archive through a loopback MCP endpoint, once you pair it in
**Configuración › Asistentes**. In this version the endpoint is **read-only,
local-only and off by default**, and it is the same code in Pro and Lite.

Status: the endpoint and the connector exist and were exercised end to end
against a real Lite debug build (isolated dev profile, one seeded client):
OpenCode, Claude Code and Pi connected and listed the five tools; the Codex
recipe registers the server, but its connection was not exercised because that
needs a model session. The connector now ships **with every installer** — Pro and
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
| Windows NSIS/MSI (Pro, Lite) | `entropia-agent-connector.exe` (Lite) or `entropia-pro-agent-connector.exe` (Pro) in the install directory, beside the app exe | that absolute path, in double quotes |
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

The lines the app generates (`apps/desktop/src/lib/agent-access.ts`), with the
client id abbreviated:

| Client | Line | Verified |
|---|---|---|
| Claude Code | `claude mcp add entropia -- entropia-agent-connector --client cl_…` | Yes — `claude mcp list` reported **✔ Connected** |
| Codex | `codex mcp add entropia -- entropia-agent-connector --client cl_…` | Registering only — `codex mcp add` accepted the recipe; its connection was not exercised (it needs a model session) |
| OpenCode | `{"mcp":{"entropia":{"type":"local","command":["entropia-agent-connector","--client","cl_…"]}}}` | Yes — `opencode mcp list` reported **connected** |
| Pi | `{"mcpServers":{"entropia":{"command":"entropia-agent-connector","args":["--client","cl_…"]}}}` | Yes — `pi mcp list` reported **connected** with 5 tools |

"No token in the config" held for all four: each config names only the client id.

The same panel lists every paired client with its creation date and last use, and
**"Revocar"** (behind a confirmation) makes that credential useless on the next
request, even for a copy someone saved earlier.

## Try it against an isolated dev profile

Never point a development run at the real archive. A debug build with
`ENTROPIA_DEV_PROFILE` uses its own data directory, turns sync off and gives the
connector its own keyring entries, so the installed app's token is neither read
nor replaced.

```bash
cd apps/desktop
ENTROPIA_DEV_PROFILE=agents pnpm exec tauri dev
# check the startup line: [setup] profile=dev:agents data_dir=… sync=disabled
```

Pair a client inside that window, then run the connector against the same
profile:

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

`ENTROPIA_DATA_DIR=<path>` replaces the whole data directory instead of using the
profile (useful in tests). Diagnostics go to stderr; stdout carries only protocol
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
(`chmod 600`) or pair the assistant again.

## Read next

- [`PROTOCOL.md`](PROTOCOL.md) — protocol version and compatibility policy,
  transport and guard order, discovery file, credentials, scopes, the five tools,
  every error code, and the audit format.
- [`INVENTORY.md`](INVENTORY.md) — what each product operation maps to today,
  which tool would expose it, and what is deliberately not exposed.
