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
| Processing tools' adapter over the batch services | `apps/desktop/src-tauri/src/automation/jobs.rs`, `apps/desktop/src-tauri/src/processing/commands.rs` |
| Owner approvals of paid work | `apps/desktop/src-tauri/src/automation/approvals.rs`, `apps/desktop/src/components/AgentApprovalDialog.svelte` |
| Clients, tokens, keyring target, token file fallback, scopes, revocation | `apps/desktop/src-tauri/src/automation/access.rs` |
| Audit trail | `apps/desktop/src-tauri/src/automation/audit.rs` |
| App surface (Tauri commands) | `apps/desktop/src-tauri/src/automation/commands.rs` |
| Connector CLI, data dir, credential read (keyring, then file) | `crates/entropia-agent-connector/src/{cli,discovery,keyring_source}.rs` |
| Connector relay and error mapping | `crates/entropia-agent-connector/src/{relay,error}.rs` |

## Status

| Piece | State |
|---|---|
| App endpoint (`automation/server.rs`) | Implemented. Loopback, stateless. Read tools, plus processing tools gated by owner-granted scopes and, for paid work, the owner's approval in the app. Same code in Pro and Lite. |
| Connector crate (`crates/entropia-agent-connector`) | Implemented. **Bundled with every installer** (Pro and Lite; NSIS/MSI, MSIX through the `entropia-agent-connector.exe` execution alias, `.deb`, macOS `.app`/`.dmg`) through the `tauri.connector.conf.json` / `tauri.connector.pro.conf.json` overlays. Code signing of the extra executable is not integrated for any binary. |
| Skill (`integrations/agents/skill/`) | **Planned** (stage 6). No code, no contract, no version declared yet. |
| Processing tools (prepare, follow, control, ask to start) | Implemented (stage 3, processing half). |
| Corpus write, research, library, writing tools | **Planned** (stages 3-5). See `INVENTORY.md`. |

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
(`src/relay.rs`). An MCP `2026-07-28` request names its own version in
`params._meta["io.modelcontextprotocol/protocolVersion"]`, before any `initialize`
(Claude Code opens with a `server/discover` probe); that value is sent as
`MCP-Protocol-Version` instead, because rmcp answers HTTP 400 when the two
disagree. Every request also carries the MCP routing headers (SEP-2243):
`Mcp-Method` with the body's method and, for `tools/call` / `prompts/get`
(`params.name`), `resources/read|subscribe|unsubscribe` (`params.uri`) and
`tasks/get|update|cancel` (`params.taskId`), `Mcp-Name`, wrapped as
`=?base64?…?=` when it is not visible ASCII. A leading UTF-8 byte order mark on a
stdin line (Windows PowerShell 5.1 adds one when `$OutputEncoding` is UTF-8) is
dropped. The app reports its own build identity as `appVersion` and
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
  of the keyring or the fallback file beforehand.
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
`automation/server.rs`), written owner-only (mode `0600` on Unix, a protected
user-and-SYSTEM access list given at creation on Windows) through a sibling temp file and a
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
- Before **every** request that carries `Authorization`, the connector sends
  `POST <mcpPath>` with `X-Entropia-Challenge: <32 random bytes, hex>` and
  **no** `Authorization`.
- The guard answers that request as row 4/5 of the table: `401 unauthorized`
  plus `X-Entropia-Proof: hex(HMAC-SHA256(instanceKey, challenge))`, where both
  the key and the challenge enter the HMAC as their ASCII text. A malformed
  challenge earns no proof.
- The connector computes the same HMAC from the key it read and compares in
  constant time. A missing or wrong proof — or a non-401 answer — is
  `app_absent` with a detail saying the instance could not be verified, and the
  token is **never** sent.
- Nothing is cached: no earlier verification of a `(port, instanceKey)` pair
  vouches for a later request, so a process that takes over the port after the
  app died never receives the token. The discovery file and the token (keyring
  or fallback file) are also read again for every message, so a restart on a new
  port is followed and revocation applies to the next request.
- Residual window: the challenge and the request it protects are two separate
  connections. The token can still reach another process only if the app dies
  and that process binds the same port in the short interval between the
  challenge and the request of the same message. A local attacker able to win
  that race on a loopback port is outside what this check promises to stop.
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
  client records `{id, name, scopes, token_sha256, token_store, created_at,
  last_used_at}` (snake_case on disk; the Tauri commands and the panel use the
  camelCase view of the same fields). Timestamps there are Unix **seconds**. The file is `0600`,
  written atomically, and an unreadable one is renamed to
  `agent-access.json.corrupt-<seconds>` and replaced by "disabled, no clients".
  `token_store` is `"keyring"` or `"file"` and is non-secret; a file written
  before the fallback existed has no `token_store` and loads as `"keyring"`.
- Revoking removes the record first, so a stored token is useless from that
  moment whatever copies exist; the keyring entry and the file are deleted after.
- The token itself lives in the OS keyring, or in the owner-only file described
  below when the machine has none. No Tauri command and no MCP result ever
  carries a token or its hash (`commands.rs`, `mcp.rs`); the panel shows only the
  storage kind.

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

### Token file fallback (D14)

Only when the OS credential store is **unavailable** — keyring
`PlatformFailure` / `NoStorageAccess`, which the app reports as
`credential_store_unavailable` — the app writes the token to a file instead:

| Piece | Value |
|---|---|
| Path | `<data dir>/agent-tokens/<client id>.token` (so a dev profile keeps its own) |
| Permissions | directory `0700`, file `0600` on Unix; on Windows a protected access list naming only the user and SYSTEM (`automation/private_acl.rs`), so nothing inherited from `%APPDATA%` applies |
| Write | atomic, like `agent-access.json` |
| Record | `token_store: "file"` in `agent-access.json`; the panel shows **Archivo protegido** |

Any other keyring error still fails the pairing: no record, no file, no token.
The connector reads the keyring first and falls back to this file only on no
entry or an unavailable store; on Unix it refuses a file whose mode grants group
or other access (`credential_insecure`, `-32006`) and sends no request.

Neither the token directory nor `agent-access.json` nor `agent-endpoint.json`
(tokens, token hashes, instance key) ever enters a backup (`backup.rs` skips all
there), the asset sweep (which only walks `assets/`) or sync (which moves a fixed
table list and only blob paths under `assets/`).

### Pairing and revocation

- Pairing: Settings › Asistentes → "Conectar un asistente" → name (1-64
  characters, no control characters) → "Conectar". The app mints the token into
  the keyring (or the owner-only file when this machine has no credential store,
  see above) first and records the hash second; a persistence failure deletes
  the orphan token. The panel then shows the client id and
  `<connector> --client <id>` (the connector installed for that build, by
  absolute path or MSIX alias) plus one ready-to-copy line per client.
- Client ids look like `cl_` + 12 hex characters.
- Revocation removes and persists the record **first**, then deletes the stored
  token: the keyring entry for a keyring client, the file for a file client. A
  failed delete still leaves the client revoked and is reported as
  `revoked, but the token copy could not be removed: …`. Revoking an unknown
  client is a no-op.
- Disabling the switch also makes every token fail: `authenticate` refuses when
  `enabled` is false.
- `last_used_at` is written to disk at most once per 60 seconds per client.

## Scopes

Six scopes exist in the access file and in the tools' contract; the wire values
are the `snake_case` names (`automation/access.rs`).

| Scope | Meaning | Granted today |
|---|---|---|
| `read` | Read the archive through the read tools | **Yes**, every paired client |
| `local_write` | Change the local archive: prepare, pause, cancel processing jobs; create and rename collections; edit item titles and metadata | By the owner, per client: «Preparar procesamiento y editar el corpus» |
| `remote_spend` | Ask for paid provider work (start, resume, retry); each request still needs the owner's approval | By the owner, per client: «Pedir procesamiento pago» |
| `external_write` | Write outside EntropIA (Zotero, publication) | No |
| `admin` | Change access, keys, maintenance | No |
| `web` | Browse external sites with the Navegador (network egress) | No |

`create_client` always grants exactly `["read"]`. The owner adds or withdraws
`local_write` and `remote_spend` in Settings › Asistentes (`agent_client_set_grants`,
`AgentAccess::set_grants`): granting asks for a confirmation, withdrawing applies
at once, and `read` and every other scope stay as they are. The endpoint reads a
client's scopes on every request, so a withdrawn scope fails on the next call. No
tool, document or assistant can grant a scope. A tool a client may not use
answers the tool error `forbidden_scope` **before** any archive is touched, so a
refusal cannot leak whether a row exists.

## Tool catalogue

Every reading tool is published with `readOnlyHint: true`; the three tools that
change the archive (`processing_prepare`, `processing_start`,
`processing_control`) declare `readOnlyHint: false` and `destructiveHint: false`.
Every tool declares `openWorldHint: false`: nothing reaches outside this machine
through the tool itself. Clients rely on this; Codex cancels a non-interactive
call to a tool that does not declare itself read-only, so it asks before calling
the three writing tools.

Twenty tools are served (`automation/mcp.rs`; `tools/list` order is alphabetical).
The seven archive tools:

| Tool | Arguments | Result |
|---|---|---|
| `entropia_status` | none — any field is refused | `{protocol, appVersion, variant, profile, client:{id,name,scopes}, archive:{collections,items}, tools:[name…]}` |
| `collections_list` | `cursor?`, `limit?` | `{records:[{id, name, description, itemCount, createdAt, updatedAt}], nextCursor}` ordered by name (case-insensitive), then id |
| `items_list` | `collectionId?`, `cursor?`, `limit?` | `{records:[{id, title, collectionId, updatedAt}], nextCursor}` ordered by title, then id |
| `items_get` | `itemId` | `{id, title, collectionId, metadata, createdAt, updatedAt, assets:[{id, type, path, sortIndex, size, pageNumber, createdAt}…]}` — metadata and file list, **never** extracted text |
| `search_items` | `query`, `collectionId?`, `limit?` | `{records:[{id, title, collectionId, updatedAt, rank}], matches}` — FTS5/BM25, any-term matching, best first |
| `documents_get_text` | `itemId`, `assetId?`, `cursor?`, `maxChars?` | `{item:{id,title}, pages:[{assetId, type, pageNumber, source, method, textHash, totalChars, startChar, endChar, text, segments}], nextCursor}` — page text in windows (see below) |
| `search_passages` | `query`, `collectionId?`, `itemId?`, `limit?` | `{records:[{itemId, title, collectionId, assetId, pageNumber, source, startChar, endChar, textHash, current, text, rank}], matches, coverage:{itemsInScope, itemsWithPassages}}` — passage FTS5/BM25, best first |

Contract details that hold for every tool:

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

### Editing the corpus

`local_write`, no dialog: these edits spend nothing and are revision-fenced. They go
through the same Rust services the app's own views use (`src-tauri/src/corpus`), so
the rules are the app's.

| Tool | Arguments | Result |
|---|---|---|
| `collections_create` | `requestId`, `name`, `description?` | `{id, name, description, createdAt, updatedAt, created}` |
| `collections_update` | `collectionId`, `revision`, `name?`, `description?` (`null` clears it) | `{id, name, description, createdAt, updatedAt}` |
| `items_update` | `itemId`, `revision`, `title?`, `metadata?` (object of text values) | `{id, title, collectionId, metadata, createdAt, updatedAt}` |
| `documents_text_update` | `itemId`, `assetId`, `textHash`, `text` | An approval (`action.kind: "replace_text"`) waiting for the owner |
| `documents_index` | `itemId` | `{itemId, indexed: true}` — refreshes the item's `search_items` entry |

- **Revisions.** `revision` is the `updatedAt` the assistant read (`collections_list`,
  `items_get`). A record that changed since then answers `revision_conflict` and
  nothing changes; a successful write always moves `updatedAt` forward.
- **Idempotent creation.** `collections_create` stores its `requestId` as
  `agent:<client id>:<requestId>`: resending it with the same arguments returns the
  same collection (`created: false`), with other arguments it is `invalid_argument`.
- **Item metadata.** `metadata` replaces every field of the item with text values.
  The app's own bookkeeping (keys starting with `__entropia_`, such as an imported
  file's provenance) is kept, never returned and cannot be written. Title and
  metadata are searchable with `search_items` as soon as the edit answers.
- **The app refreshes.** After each write the app emits `corpus:changed`
  (`{kind, id, collectionId}`); the sidebar and Colecciones re-read on it. An item
  already open in the app does not reload by itself yet (it may hold unsaved edits).
- **Replacing a page text asks the owner** (S5: overwriting needs a specific
  confirmation). `documents_text_update` checks everything first: the file belongs to
  the item, the page has recognized text (native or OCR; an audio transcription
  cannot be replaced this way), `textHash` is still the page's, the text is new and
  1-200000 characters. Then it creates an approval and nothing changes until the
  owner approves in the app, which shows the collection and the first 400 characters
  of the proposal. On approval the text is replaced only if the page's hash is still
  the approved one (otherwise the approval ends `failed`), the text it had before the
  first correction is kept in `ocr_correction_backups`, so «Restaurar original»
  undoes it, and the item is re-indexed. Embeddings are not recomputed: ask for them
  with `processing_prepare`, which has its own approval.
- Names are 1-200 characters, descriptions up to 2000, titles 1-500, at most 100
  metadata fields; control characters are refused.

### Importing files

The owner enables import folders in Settings › Asistentes («Carpetas de entrada»,
stored in `agent-access.json` as `import_folders`, canonical paths). An assistant can
only read files **directly** inside them (no subfolders) and never names a folder of
its own. Both tools need `local_write`.

| Tool | Arguments | Result |
|---|---|---|
| `import_sources_list` | none | `{folders:[{path, files:[{name, kind, sizeBytes, modifiedAt}], truncated}]}` — supported files (PDF, images, audio) by name, up to 500 per folder |
| `documents_import` | `collectionId`, `folder`, `files` (names, 1-200) | An approval (`action.kind: "import_files"`) waiting for the owner |

- **Checked before any dialog.** The folder must be an enabled one; each name must be a
  plain file name; each file is checked by its real path (links and `..` resolved) to be
  a regular, supported file of at most 1 GiB directly inside an enabled folder and
  outside EntropIA's own data. One bad file refuses the whole request
  (`invalid_argument`), so the dialog only lists real files.
- **The owner sees the list.** The dialog shows the collection and every file's name and
  size; the approval carries the same list as `importFiles` (`name`, `kind`,
  `sizeBytes`), never the paths. The same files in any order are the same request.
- **Checked again at the answer.** On approval each file is checked against the folders
  enabled *at that moment* and against the size and modification time it was listed
  with. A file that changed, left the folders, or is already in the collection (same
  path ignoring case, size and modification time, the app's own rule) is skipped.
  `approvals_status` then reports `importResult`: `{collectionId, imported:[{name,
  itemId, title}], skipped:[{name, reason}]}`, where `reason` starts with a stable word
  (`changed`, `duplicate`, `not_in_folder`, `missing`, …).
- **The same result as the app's import.** One item per file titled by its name without
  the extension, the file copied into the archive with its provenance, each PDF kept as
  a parent with one page asset per page, the title searchable at once. Importing spends
  nothing; recognizing text or embedding is a processing job with its own approval.

### Reading text and citing

A **page** is one file of an item: a split PDF has one file per page (with
`pageNumber`), an image or an audio file is one page. Its text is what the app's
own citations read (`lib/page-text.ts`): the latest extraction (native text or OCR)
when it has text, otherwise the latest transcription, whose timed `segments` come
along.

- **Windows, never the whole file.** `documents_get_text` returns at most
  `maxChars` characters per call (1-20000, default 4000), across pages in file
  order, and an opaque `nextCursor` for the rest; `assetId` reads one file. A
  cursor names the file it stopped in, so a document whose files changed is
  `invalid_argument` and is read again from the start. Files without text are left
  out (`items_get` lists every file).
- **Locations.** `startChar` / `endChar` count **Unicode characters** of the whole
  page text, the unit the passage index uses, and `textHash` is sha256 of that
  text. A citation is `itemId` + `assetId` + `pageNumber` + offsets + `textHash`;
  row ids of extractions and passages are not stable and are never exposed.
- **Passages.** `search_passages` searches the chunk index the app's chat uses
  (`rag_chunks_fts`), so only documents processed with embeddings have passages;
  `coverage` says how many items of the scope do. `current: false` means the page
  text changed after the passage was indexed: read the page again before citing
  it. Only the query's length is audited.

### Processing tools

They drive the same persistent batch queue as the Procesamiento tab, through the
same services (`processing/commands.rs`): a job an assistant prepares appears in
the tab, and the tab's rules (validation, revision fencing, idempotency) apply
unchanged. A "job" is a batch.

| Tool | Scope | Arguments | Result |
|---|---|---|---|
| `jobs_list` | `read` | `states?`, `cursor?`, `limit?` | `{records:[{id, state, desiredState, operations, revision, priority, createdAt, updatedAt, activeUnits, failedUnits, succeededUnits}], nextCursor}`, newest first; system containers (library sync, repairs) are not listed |
| `jobs_get` | `read` | `jobId`, `taskState?`, `taskKind?`, `tasksCursor?`, `tasksLimit?` | `{job:{id, state, desiredState, operations, revision, planningDone, membersTotal, progressDone, progressTotal, tasksByState, tasksByKind, collections, …}, tasks:[…], nextTasksCursor}` |
| `processing_prepare` | `local_write` | `requestId`, `collectionIds?`, `itemIds?`, `operations` | `{jobId, created, files, operations, state, revision, costNote, nextStep}` — a **paused** draft; nothing is spent |
| `processing_start` | `remote_spend` | `jobId`, `revision` | An approval (below) with `status: "pending"`; nothing starts until the owner approves |
| `processing_control` | see below | `jobId`, `action`, `revision?`, `taskId?`, `failedOnly?` | `{jobId, action, outcome, job, approval}` |
| `approvals_status` | `read` | `approvalId` | The approval, for the client that asked for it only |

- **Prepare.** `operations` are `ocr`, `embeddings`, `ner`, `triples` and
  `schema:<id>`; any other value is `invalid_argument` (never silently dropped).
  The scope is collections, items or both; each file enters once.
  `requestId` (1-128 visible characters) is stored as `agent:<client id>:<requestId>`:
  sending it again with the same arguments returns the same job (`created: false`),
  with other arguments it is `invalid_argument`, and two assistants never collide.
  Pages and units are counted by background planning, so no cost is estimated at
  this point (`costNote` says so); `jobs_get` reports `tasksByKind` once
  `planningDone` is true. In Lite, embeddings, entities, triples and schemas need
  the OpenRouter key, as in the tab: without it the answer is
  `provider_not_configured`. Only the stored reference is checked; the secret is
  never read.
- **Control.** `pause` and `cancel` need `local_write` and apply at once to that
  one job (never "all jobs"); they only withdraw demand — running units finish
  their current step and confirmed results are kept. `resume` and `retry` spend
  again, so they need `remote_spend` and `revision` and become an approval. Only a
  job that has started can be resumed: a draft (`preparing`, `ready`) is started
  with `processing_start`, and `resume` on it is `invalid_state`
  (`outcome: "approval_pending"`); `retry` reopens one task (`taskId`) or every
  failed one (`failedOnly: true`).

### Approvals

An assistant can never start, resume or retry paid work by itself. `processing_start`
and the paying `processing_control` actions create an approval
(`automation/approvals.rs`) bound to the client, the action, the job and the job
`revision` the assistant named. The app is told (`agent-approvals:changed`), its
window is unminimized and brought to the front, and a dialog shows who asks, the
operations, the collections and the number of files. A job with OCR and no
embeddings also says that every recognized page is then embedded with the configured
provider, which may cost too: the app admits that follow-up for every committed OCR.
Only the owner's «Aprobar»
runs the action, through the same services, fenced by that revision: if the job
changed in the meantime nothing runs and the approval ends `failed`. The dialog's
initial focus is «Rechazar», Escape rejects and a click outside does nothing.

```json
{"id": "apr_…", "clientId": "cl_…", "clientName": "Claude Code",
 "action": {"kind": "start"}, "jobId": "batch-…", "revision": 3,
 "operations": ["ocr"], "collections": ["Alfa"], "files": 12,
 "status": "pending", "createdAt": 1767225600000, "expiresAt": 1767226200000,
 "decidedAt": null, "failure": null}
```

`action` is `{"kind": "start"}`, `{"kind": "resume"}`,
`{"kind": "retry", "task_id": …, "failed_only": …}`,
`{"kind": "replace_text", "item_id": …, "asset_id": …, "text_hash": …, "new_text_hash": …}` or
`{"kind": "import_files", "collection_id": …, "files": …, "plan_hash": …}`.
For `import_files`, `jobId` is empty, `revision` is 0, `importFiles` lists the files and
`importResult` is filled once it ran. For `replace_text`, `jobId` is empty, `revision` is 0 and `preview` holds the first
400 characters of the proposed text; the whole text is never returned. `status` is `pending`,
`approved` (approved and run), `failed` (approved, but the job had changed; nothing
ran, `failure` says why), `rejected` or `expired`.

- **Ten minutes, once.** An approval expires ten minutes after it was asked for and
  can be answered once; two answers racing cannot both run.
- **No duplicates.** The same client asking the same action on the same job at the
  same revision while it is pending gets that approval back, not a second dialog.
- **Memory only.** Approvals are not persisted: an app restart leaves nothing
  approved, and the assistant has to ask again.
- **Nothing the assistant sends approves.** The endpoint can only create approvals
  and read the caller's own; the answer comes from the app (`agent_approval_decide`).

### Tool result and tool error shape

A success is a normal MCP tool result with `structuredContent` and
`isError: false`. A refusal is an MCP **tool error** (not a JSON-RPC error):
`isError: true` and
`structuredContent: {"error": "<code>", "message": "<one Spanish sentence>"}`.

| `error` | When | Source |
|---|---|---|
| `forbidden_scope` | The client lacks the scope the tool or action needs | `mcp.rs` (`guard`, `guard_scope`) |
| `invalid_argument` | Unknown field, wrong type, bad cursor, `limit` outside 1-200, empty search text, unknown state, kind, operation or action, a `requestId` reused with other arguments, a missing `revision` for `resume`/`retry` | `mcp.rs`, `read.rs`, `jobs.rs` |
| `not_found` | An item, job or approval id that does not exist here (or, for an approval, belongs to another client) | `read.rs`, `jobs.rs`, `mcp.rs` |
| `revision_conflict` | The job is no longer at the `revision` the assistant named: read it again with `jobs_get` and retry | `jobs.rs` |
| `invalid_state` | The job's state does not admit the action (start before planning finished, resume a job that is not paused, act on a finished one) | `jobs.rs` |
| `provider_not_configured` | Lite, an operation that needs the OpenRouter key, and no key configured | `jobs.rs` |
| `unavailable` | The archive could not be opened or read right now; a processing error the contract has no code for also lands here, with its detail kept in the app log | `read.rs`, `jobs.rs`, `mcp.rs` |
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
| `-32005` | `credential_missing` | No token anywhere for the client id: no keyring entry (and no fallback file), or a credential-store error that is not "unavailable" | Pair the assistant; if it persists, check the OS credential store |
| `-32006` | `credential_insecure` | The fallback token file is readable by group or others (Unix), so it is refused and no request is sent | Fix the file's permissions (`chmod 600 <data dir>/agent-tokens/<client id>.token`) or pair the assistant again |
| `-32007` | `credential_store_unavailable` | The OS credential store cannot be reached from the connector's process (no Secret Service or session bus) and there is no fallback file, so whether the client is paired is unknown. On Linux the connector first points `DBUS_SESSION_BUS_ADDRESS` at the user's own bus socket when the agent started it without one | Run the agent inside the desktop session with the keyring unlocked, or pass it `DBUS_SESSION_BUS_ADDRESS` |
| `-32000` | `endpoint_error` | Any HTTP status no rule covers (404, 500, …), or a `2xx` answer that carried no JSON where a result was expected (empty body, non-JSON body, or an event stream without a JSON event) | Treat as an app-side fault; report it with the app's logs |
| `-32700` | `parse_error` | The stdin line was not JSON-RPC | Fix the client's framing |

`data.detail` carries technical context (the discovery path, the HTTP status, the
keyring error) and never a secret.

Rules around the relay (`src/relay.rs`):

- A **notification** (no `id`) that fails never produces a response; the failure
  is logged to stderr. That includes a `2xx` whose body is empty or is not JSON.
- A **request** (with an `id`) answered with an empty `2xx`, a non-JSON body,
  or a `text/event-stream` body with no JSON `data:` event (none at all, or only
  non-JSON ones) becomes a JSON-RPC error on stdout — `-32000` / `data.code: "endpoint_error"` —
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
| `objectIds` | Ids the call returned or touched (for a paid request, the job id and the approval id) |
| `queryChars` | Present only for `search_items`: the query **length** |
| `decision` | Present only on `owner_decision` lines: `approved`, `failed` (approved, but the job had changed and nothing ran) or `rejected` |

When the owner answers a request in the app, the trail gets one line with
`tool: "owner_decision"`, attributed to the assistant that asked, with
`objectIds: [approval id, job id]` and the `decision`. A request that expires
unanswered leaves no `owner_decision` line.

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

- Installer-driven pairing (stage 6). The connector itself already ships with
  every installer (see Status); code signing of that extra executable is not
  integrated for any binary.
- The skill artifact and its declared protocol version (stage 6).
- Corpus write tools, research, library and writing tools, progress
  notifications and any session state (stages 3-5). `collections_create`,
  `documents_import_prepare`, `search_passages`, `writing_apply_patch` and
  `writing_export` are names the plan proposes; none of them exists in this code.
- A USD estimate before a job starts: pages are only counted by planning (see
  Processing tools).
- Approvals that survive an app restart; today they are memory only on purpose.
