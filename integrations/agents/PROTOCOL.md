# Protocol `entropia-agent/1`

The contract between the installed EntropIA app, the `entropia-agent-connector`
binary and (later) the portable skill. This document describes what the code in
this repository does today; anything not implemented is marked **planned**.

Source of truth, by concern:

| Concern | Code |
|---|---|
| Endpoint, request guard, discovery file, protocol string | `apps/desktop/src-tauri/src/automation/server.rs` |
| Instance key, HMAC and constant-time compare (D12) | `apps/desktop/src-tauri/src/automation/proof.rs`, `crates/entropia-agent-connector/src/proof.rs` |
| Tools, arguments, results, tool error codes | `apps/desktop/src-tauri/src/automation/mcp.rs` |
| Read connection, pagination, read errors | `apps/desktop/src-tauri/src/automation/read.rs` |
| Clients, tokens, keyring target, scopes, revocation | `apps/desktop/src-tauri/src/automation/access.rs` |
| Audit trail | `apps/desktop/src-tauri/src/automation/audit.rs` |
| App surface (Tauri commands) | `apps/desktop/src-tauri/src/automation/commands.rs` |
| Connector CLI, data dir, keyring read | `crates/entropia-agent-connector/src/{cli,discovery,keyring_source}.rs` |
| Connector relay and error mapping | `crates/entropia-agent-connector/src/{relay,error}.rs` |

## Status

| Piece | State |
|---|---|
| App endpoint (`automation/server.rs`) | Implemented. Loopback, stateless, read-only tools. Same code in Pro and Lite. |
| Connector crate (`crates/entropia-agent-connector`) | Implemented as a standalone crate. **Not bundled in installers yet** (stage 6). |
| Skill (`integrations/agents/skill/`) | **Planned** (stage 6). No code, no contract, no version declared yet. |
| Write, processing, research, library, writing tools | **Planned** (stages 3-5). See `INVENTORY.md`. |
| File fallback for machines without a keyring | **Planned**. Both sides answer `credential_missing` today. |

## Compatibility policy

The protocol identifier is the literal string **`entropia-agent/1`**. The app
writes it into the discovery file (`PROTOCOL` in `automation/server.rs`) and the
connector compares it against its own constant (`PROTOCOL` in
`src/discovery.rs`) before it sends a single HTTP request.

**What bumps the version** — any change a connector built for the previous
version could not survive:

- renaming, removing or re-scoping a tool, or changing a tool's argument names,
  types or meaning, or the name/type of a field in a tool's result;
- adding a *required* argument to an existing tool;
- renaming or repurposing a discovery file field, or changing `/mcp` as the only
  path, the loopback-only binding, the guard order, or a guard status/error code;
- moving the credential (keyring service/account rule) or changing what the
  access file holds.

**What does not bump it** — additive changes a v1 connector ignores safely:
a new tool, a new optional argument, a new field in a result object, a new tool
error code that describes a new refusal, new audit fields.

**How a mismatch surfaces.** The connector never negotiates: it requires exact
equality. A discovery file whose `protocol` is anything else is answered with
JSON-RPC error `-32004` / `data.code: "protocol_incompatible"`, whose message
tells the person to update both the app and the connector, and whose
`data.detail` carries the value that was found
(`crates/entropia-agent-connector/src/error.rs`). The endpoint is never called,
so the app cannot observe this case.

**The instance proof (D12) is a required field inside `entropia-agent/1`.** The
discovery file gained `instanceKey` without a version bump, because a connector
that predates it simply ignores an unknown field: the string stays `entropia-agent/1`
and an older connector keeps working. The reverse is refused: a discovery file
with no usable `instanceKey` cannot be verified, so this connector answers
`-32004` / `protocol_incompatible` instead of trusting the port. The guard also
gained one step and two headers, which no conforming v1 caller notices because a
caller without a challenge header is authenticated exactly as before.

There is no other version channel between app and connector. The MCP
`initialize` handshake negotiates the *MCP* protocol version (handled by `rmcp`,
3.8), not `entropia-agent/1`; the connector remembers the negotiated MCP version
and sends it back as `MCP-Protocol-Version` on later requests
(`src/relay.rs`). The app reports its own build identity as `appVersion` and
`variant` in `entropia_status` and as `serverInfo.version` on `initialize`.

**Skill.** When the skill exists it must declare the protocol version it targets
and refuse to guide someone through an app that answers a different one. Planned,
not implemented.

## Transport

MCP Streamable HTTP served by `rmcp` 3.8 over hyper/tower, inside the app
(`automation/server.rs`).

| Property | Value |
|---|---|
| Bind address | `127.0.0.1:0` — loopback only, ephemeral port, never `0.0.0.0` |
| Path | The path component must be exactly `/mcp`. The guard compares `request.uri().path()`, so a query string is ignored by that comparison: `/mcp?x=1` reaches `rmcp`, `/mcp/` and `/mcp/anything` are `404 not_found` |
| Session mode | Stateless (`legacy_session_mode: false`). No `Mcp-Session-Id` is trusted; nothing caches identity between requests |
| Response encoding | JSON (`json_response: true`); SSE keep-alive disabled |
| Request body cap | 1 MiB (`MAX_REQUEST_BODY_BYTES`) |
| Identity per request | Bearer token → `Principal` placed in the request extensions; `rmcp` copies `http::request::Parts` into each tool's `RequestContext` |
| Server name | `entropia` |
| Lifetime | Started when the Asistentes switch is on, stopped by the switch or by app exit; a start failure is logged, never fatal |

### Request guard

The guard runs before `rmcp` sees the request, in this fixed order. Each refusal
is a JSON body `{"error": "<code>", "message": "<one Spanish sentence>"}` with
`Content-Type: application/json`.

| # | Condition | Status | `error` |
|---|---|---|---|
| 1 | Path component is not exactly `/mcp` (`/mcp/`, `/elsewhere`) | 404 | `not_found` |
| 2 | `Host` is not exactly `127.0.0.1:<port>` or `localhost:<port>` (case-insensitive, port included) | 403 | `forbidden_host` |
| 3 | Any `Origin` header is present, whatever its value | 403 | `forbidden_origin` |
| 4 | `X-Entropia-Challenge` is present, no `Authorization`, and the value is 64 hex characters | 401 + `WWW-Authenticate: Bearer` + `X-Entropia-Proof` | `unauthorized` |
| 5 | `X-Entropia-Challenge` is present, no `Authorization`, and the value is malformed | 401 + `WWW-Authenticate: Bearer`, **no** proof | `unauthorized` |
| 6 | No `Authorization: Bearer` token, or it authenticates to no live client | 401 + `WWW-Authenticate: Bearer` | `unauthorized` |
| 7 | Passed | — | `rmcp` handles the MCP message |

A challenge request that also carries an `Authorization` header is not treated as
a challenge: it takes the ordinary bearer path (row 6).

Notes that are part of the contract:

- Authenticating happens on **every** request, so revoking a client or turning
  the switch off takes effect on the next request, even for a token copied out
  of the keyring beforehand.
- No browser page is a legitimate caller; that is why any `Origin` is refused
  rather than filtered by value. The `Origin` refusal runs **before** the
  challenge, so a browser page cannot even obtain a proof.
- Session ids are never treated as credentials.
- `GET` and `DELETE` on `/mcp` are answered by `rmcp` with `405 Method Not
  Allowed` and `Allow: POST` (a GET with an `Accept` that omits
  `text/event-stream` is `406 Not Acceptable` before that); neither returns MCP
  data. Only `POST` carries messages.

### Discovery file

`agent-endpoint.json` in the app's data directory (`DISCOVERY_FILE` in
`automation/server.rs`), written mode `0600` through a sibling temp file and a
rename. Written when the listener is already bound (so it never names a dead
port) and removed when the endpoint stops or the app exits.

| Field | Type | Meaning |
|---|---|---|
| `protocol` | string | `entropia-agent/1` |
| `mcpPath` | string | `/mcp` |
| `port` | number | The ephemeral loopback port |
| `pid` | number | The app process id |
| `appVersion` | string | `CARGO_PKG_VERSION` of the app |
| `variant` | string | `pro` with the `local-ml` feature, `lite` without |
| `profile` | string or `null` | Active dev profile name; `null` in a release build |
| `instanceKey` | string | 64 hex characters (32 bytes) minted per start from the OS generator; the D12 proof key, never sent over HTTP |

The connector re-reads this file for **every** message, so an app restart on a
new port is followed without restarting the agent. A file that cannot be parsed,
that names port 0 or a path that is not a plain absolute path, or that has no
usable `instanceKey`, is reported as `app_absent` (the first three) or
`protocol_incompatible` (the missing key) (`src/discovery.rs`).

### Instance proof (D12)

After a crash the app cannot remove the file, so a stale `agent-endpoint.json`
can name a port another process — possibly another OS user — later holds. Without
this step that process would receive a valid token on the connector's first
request.

- At each start the app mints a new `instanceKey` (`automation/proof.rs`) and
  writes it into the `0600` discovery file. It is never sent over HTTP.
- Before the first token goes to a `(port, instanceKey)` pair it has not verified
  in this process, the connector sends `POST <mcpPath>` with
  `X-Entropia-Challenge: <32 random bytes, hex>` and **no** `Authorization`.
- The guard answers that request as row 4/5 of the table: `401 unauthorized`
  plus `X-Entropia-Proof: hex(HMAC-SHA256(instanceKey, challenge))`, where both
  the key and the challenge enter the HMAC as their ASCII text. A malformed
  challenge earns no proof.
- The connector computes the same HMAC from the key it read and compares in
  constant time. A missing or wrong proof — or a non-401 answer — is
  `app_absent` with a detail saying the instance could not be verified, and the
  token is **never** sent.
- The verified pair is cached for the life of the connector process. A new port
  or a new key is verified again. The token is read from the keyring for every
  message, as before, so revocation still applies to the next request.
- HMAC is built on the `sha2` crate already in both trees (no HMAC crate), and
  RFC 4231 test case 2 pins the same vector on both sides, so the two
  implementations cannot drift apart.

## Credentials

### Token

- Format: `eat_` + 64 lowercase hex characters — 244 random bits from two UUID
  v4 values (`mint_token` in `automation/access.rs`). The prefix makes a leaked
  token recognisable to a secret scanner.
- `agent-access.json` (next to the archive, **not** in SQLite, never synced)
  stores only the SHA-256 hex of each token, plus `version`, `enabled` and the
  client records `{id, name, scopes, tokenSha256, createdAt, lastUsedAt}`.
  Timestamps there are Unix **seconds**. The file is `0600`, written atomically,
  and an unreadable one is renamed to `agent-access.json.corrupt-<seconds>` and
  replaced by "disabled, no clients".
- The token itself lives in the OS keyring. No Tauri command and no MCP result
  ever carries a token or its hash (`commands.rs`, `mcp.rs`).

### Keyring target

Computed identically on both sides
(`automation::access::keyring_target` and
`crates/entropia-agent-connector/src/keyring_source.rs`); a test pins the strings.

| App | Service | Account |
|---|---|---|
| Release | `com.entropia.agent-connector` | `<client id>` |
| Dev profile `<name>` | `com.entropia.agent-connector (dev profile)` | `<client id>@dev-profile:<name>` |

Both the service **and** the account change in a profile, so a profile can
neither read nor replace a real client's token.

### Pairing and revocation

- Pairing: Settings › Asistentes → "Conectar un asistente" → name (1-64
  characters, no control characters) → "Conectar". The app mints the token into
  the keyring first and records the hash second; a persistence failure deletes
  the orphan token. The panel then shows the client id and
  `entropia-agent-connector --client <id>` plus one ready-to-copy line per
  client.
- Client ids look like `cl_` + 12 hex characters.
- Revocation removes and persists the record **first**, then deletes the keyring
  entry. A failed keyring delete still leaves the client revoked and is reported
  as `revoked, but the token copy could not be removed: …`. Revoking an unknown
  client is a no-op.
- Disabling the switch also makes every token fail: `authenticate` refuses when
  `enabled` is false.
- `lastUsedAt` is written to disk at most once per 60 seconds per client.

## Scopes

Six scopes exist in the access file and in the tools' contract; the wire values
are the `snake_case` names (`automation/access.rs`).

| Scope | Meaning | Granted today |
|---|---|---|
| `read` | Read the archive through the read tools | **Yes**, every paired client |
| `local_write` | Change the local archive | No |
| `remote_spend` | Call a paid provider on the person's behalf | No |
| `external_write` | Write outside EntropIA (Zotero, publication) | No |
| `admin` | Change access, keys, maintenance | No |
| `web` | Browse external sites with the Navegador (network egress) | No |

`create_client` always grants exactly `["read"]`. A tool a client may not use
answers the tool error `forbidden_scope` **before** any archive is touched, so a
refusal cannot leak whether a row exists. Changing a client's scopes is not
exposed in the app in this tranche.

## Tool catalogue

Five tools are served (`automation/mcp.rs`; `tools/list` order is alphabetical).

| Tool | Arguments | Result |
|---|---|---|
| `entropia_status` | none — any field is refused | `{protocol, appVersion, variant, profile, client:{id,name,scopes}, archive:{collections,items}, tools:[name…]}` |
| `collections_list` | `cursor?`, `limit?` | `{records:[{id, name, description, itemCount, createdAt, updatedAt}], nextCursor}` ordered by name (case-insensitive), then id |
| `items_list` | `collectionId?`, `cursor?`, `limit?` | `{records:[{id, title, collectionId, updatedAt}], nextCursor}` ordered by title, then id |
| `items_get` | `itemId` | `{id, title, collectionId, metadata, createdAt, updatedAt, assets:[{id, type, path, sortIndex, size, pageNumber, createdAt}…]}` — metadata and file list, **never** extracted text |
| `search_items` | `query`, `collectionId?`, `limit?` | `{records:[{id, title, collectionId, updatedAt, rank}], matches}` — FTS5/BM25, any-term matching, best first |

Contract details that hold for all five:

- **Strict arguments.** Every argument struct denies unknown fields and the
  published JSON Schema says so. A wrong type, an unknown field or an
  unparsable value is `invalid_argument`.
- **Pagination.** `limit` defaults to 50, accepts 1-200, and is **refused**
  outside that range rather than clamped. Lists page by keyset over
  `(sort key, id)`, so a page never repeats or skips a record; `cursor` is an
  opaque base64 token that can only be handed back, and a made-up one is
  `invalid_argument`. `nextCursor` is `null` on the last page. `search_items`
  has no cursor: it returns the best `limit` hits and `matches` counts every hit
  before truncation.
- **Revisions and ids.** `id` is stable and `updatedAt` is the revision to
  compare. Timestamps in tool payloads come from the archive and are Unix
  **milliseconds** (`packages/store` writes `Date.now()`).
- **No generic escape hatch.** There is no SQL, shell, `eval` or "invoke any
  command" tool, and the module is written so one must never be added.
- **Reads are isolated.** Each call opens its own connection with
  `open_archive_connection` + `PRAGMA query_only = ON`; the UI connection is
  never used, and blocking work runs on a blocking task.

### Tool result and tool error shape

A success is a normal MCP tool result with `structuredContent` and
`isError: false`. A refusal is an MCP **tool error** (not a JSON-RPC error):
`isError: true` and
`structuredContent: {"error": "<code>", "message": "<one Spanish sentence>"}`.

| `error` | When | Source |
|---|---|---|
| `forbidden_scope` | The client lacks `read` | `mcp.rs` (`guard`) |
| `invalid_argument` | Unknown field, wrong type, bad cursor, `limit` outside 1-200, empty search text | `mcp.rs`, `read.rs` |
| `not_found` | `items_get` names an id that does not exist in this archive | `read.rs` |
| `unavailable` | The archive could not be opened or read right now | `read.rs`, `mcp.rs` |
| `internal_error` | A request reached a tool without an authenticated principal — a server bug, never something the caller caused | `mcp.rs` (`caller`) |

## Connector errors

The connector turns every failure into a JSON-RPC error response for requests
that carry an `id`, with a `-32000`-family code, a Spanish `message` for the
person and a stable `data.code` for the agent
(`crates/entropia-agent-connector/src/error.rs`).

| JSON-RPC code | `data.code` | Meaning | What the person should do |
|---|---|---|---|
| `-32001` | `app_absent` | No discovery file, an unreadable one, nothing listening on its port, or an endpoint that could not prove the discovery file's `instanceKey` | Open EntropIA and turn on Settings › Asistentes; if it persists, the file may be stale after a crash — the running app rewrites it on the next start |
| `-32002` | `unauthorized` | HTTP 401 for a token: not paired, or revoked. A disabled endpoint is not this error — turning the switch off stops the listener and removes the discovery file, so the connector reports `app_absent` | Pair the assistant again in Settings › Asistentes |
| `-32003` | `forbidden` | HTTP 403 from the loopback guard (`forbidden_host`, `forbidden_origin`) | Use the connector; do not call the endpoint from a browser or another host name |
| `-32004` | `protocol_incompatible` | Discovery `protocol` is not `entropia-agent/1`, or it has no usable `instanceKey` | Update the app and the connector |
| `-32005` | `credential_missing` | No keyring entry for the client id, or the keyring cannot be read | Pair the assistant; if it persists, check the OS credential store |
| `-32000` | `endpoint_error` | Any HTTP status no rule covers (404, 500, …), or a `2xx` answer that carried no JSON where a result was expected | Treat as an app-side fault; report it with the app's logs |
| `-32700` | `parse_error` | The stdin line was not JSON-RPC | Fix the client's framing |

`data.detail` carries technical context (the discovery path, the HTTP status, the
keyring error) and never a secret.

Rules around the relay (`src/relay.rs`):

- A **notification** (no `id`) that fails never produces a response; the failure
  is logged to stderr. That includes a `2xx` whose body is empty or is not JSON.
- A **request** (with an `id`) answered with an empty `2xx` or a non-JSON body
  becomes a JSON-RPC error on stdout — `-32000` / `data.code: "endpoint_error"` —
  because an agent waiting for that result would otherwise wait forever. Stdout
  carries protocol messages only.
- Responses may overlap, but every stdout line is written whole by one writer.

## Audit trail

One JSON object per line in `<cache>/logs/agent-audit.jsonl`, where `<cache>` is
the app's local cache directory (`automation/audit.rs`, wired in `lib.rs`).

| Field | Meaning |
|---|---|
| `ts` | Unix milliseconds |
| `clientId`, `clientName` | The authenticated caller |
| `tool` | Tool name |
| `status` | `ok` or `error` |
| `errorCode` | Present only on `error`: the stable code above |
| `objectIds` | Ids the call returned or touched |
| `queryChars` | Present only for `search_items`: the query **length** |

Never recorded: the query text, titles, paths, any other free text, tokens.
Rotation: before appending, if the file is non-empty and the new line would pass
5 MiB (`5 * 1024 * 1024`), the current file is renamed to
`agent-audit.jsonl.1`, replacing whatever was there — one previous file is the
whole retention policy. Writing is best effort: a failure logs a warning and the
tool call still answers. Refusals that happen **inside a tool** are audited too,
including `forbidden_scope` and `invalid_argument`.

Gaps worth knowing: refusals at the HTTP guard (401, 403, 404) and
`internal_error` are **not** audited, because no authenticated principal exists
to attribute them to. The audit trail therefore answers "what did a paired
assistant do", not "who knocked on the port".

## Planned, not implemented

- A file fallback for machines with no OS keyring (both sides) — today
  `credential_missing`.
- Bundling the connector into installers, and therefore any installer-driven
  pairing (stage 6).
- The skill artifact and its declared protocol version (stage 6).
- Write tools, jobs/approvals tools, progress notifications and any session
  state (stages 3-5). `approvals_status`, `collections_create`,
  `documents_import_prepare`, `processing_start`, `jobs_get`,
  `search_passages`, `writing_apply_patch` and `writing_export` are names the
  plan proposes; none of them exists in this code.
