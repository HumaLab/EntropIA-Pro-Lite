# `bibliography-derived-v1` — wire contract (client side)

This file is the client half of the server contract for the derived cache of paid Bibliography results
(`odd/plans/plan-sync-bibliografia-derivada.md` §2.5 option (b), §2.7, §2.9). The code that implements it
lives in `sync/derived.rs` (tables, account semantics, capability, cursor) and `sync/derived_transport.rs`
(push/pull/cycle). The server of EntropIA Cloud (`entropia-sync-server`) must implement the other half; its
source is not in this repository (task T10), so this document is the specification to implement it from.

The canonical wire document lives outside this repository (`EntropIA-Cloud/docs/PROTOCOL.md`); this
file is the section to merge there, written from the client that already implements it.

It is a **dedicated channel**, in the same family as `writing-envelope-v1` and `research-envelope-v1`: its own
opt-in header, its own outbox flag, its own cursor and its own since-zero catch-up. It never rides the generic
oplog, never touches `SYNCED_TABLES` and never moves the corpus cursor `last_pull_seq`.

## 1. Capability

- Token: the exact, case-sensitive string `bibliography-derived-v1`.
- **Advertisement (server → client).** Every successful ordinary `POST /v1/sync/push` and
  `GET /v1/sync/pull` response carries the additive `capabilities` array. The server advertises the token there
  when it implements this channel. The client records `{epoch, advertised}` for the response's `server_epoch`
  only, and only from an ordinary response (never from an opted-in one). A later ordinary response that omits
  the token overwrites the record (a rolled-back server must stop receiving derived rows).
- **Opt-in (client → server).** The channel's own requests always send
  `X-Sync-Capabilities: bibliography-derived-v1` — exactly that token, nothing else. The client only issues them
  after an ordinary response advertised the token for the current `server_epoch`. A server without the
  capability therefore never receives a request it does not understand; the corpus keeps syncing normally.
- **Three client states** (`sync::derived::capability_state`):
  `Unknown` (no `server_epoch` yet, or no record for the current one), `Supported` (recorded advertised for the
  current epoch) and `Unsupported` (recorded `advertised: false` for the current epoch — a legacy server).
  Only `Supported` sends or applies anything from this channel.
- An interrupted or rotated epoch (`server_epoch` changed) invalidates the record: the new epoch has to
  advertise the token again, and its catch-up runs again from zero.

## 2. Endpoints

The channel reuses the existing sync endpoints; the opt-in header selects it. This keeps one transport, one
authentication model and one `schema_tag` rule.

| Method | Path | Request | Response |
| --- | --- | --- | --- |
| `push_with_bibliography_derived_v1` | `POST /v1/sync/push` | `{ "changes": [Change] }` | `{ results: [Result], max_server_seq, server_epoch, server_now_ms, capabilities }` |
| `pull_with_bibliography_derived_v1` | `GET /v1/sync/pull?since=&limit=` | — | `{ rows: [Row], next_since, has_more, schema_tag, server_epoch, server_now_ms, capabilities }` |

Both requests carry `Authorization: Bearer <device token>` and `X-Schema-Tag`. A derived request answers only
derived rows (see §3); an ordinary request must never answer them, because an ordinary client cannot apply a
row of a table it does not know.

`since` / `next_since` / `has_more` / `server_epoch` / `capabilities` have exactly the meaning they already have
for the corpus, over the channel's own sequence. The channel's sequence is independent from the corpus
sequence: the client's `last_pull_seq` never moves because of a derived request, and a derived `since` is never
compared with a corpus `server_seq`.

**Push response contract.** The client does **not** interpret per-row `results` on this channel (it accepts an
empty `results` array). A `200` whose `server_epoch` equals the frozen epoch and whose `capabilities` include the
token is taken as "every change in this request is durably stored": the client marks exactly the sent keys
`pushed = 1` in one local transaction. The server must therefore (a) apply each change idempotently and (b) answer
`200` only after the rows are durably inserted. It must answer a `4xx` for a request it did not store at all
(the client then leaves those rows pending and retries them in a later cycle); `413 payload_too_large` on a
single-row request is understood as "this row is larger than `max_push_bytes`" and the client skips that row for
the rest of the cycle, leaving it `pushed = 0`.

**Capability evidence.** Only an ORDINARY `POST /v1/sync/push` / `GET /v1/sync/pull` response records a verdict,
and only for its own `server_epoch`; opted-in responses never record or change one. An opted-in response that
does not advertise the token back is treated as "this server did not implement the channel": the client installs
nothing and marks nothing (no row, no cursor, no catch-up, no `pushed`) and retries in a later cycle only if an
ordinary response advertises the token again.

## 3. Rows

Three wire tables: `derived_vectors`, `derived_ocr_pages` and `derived_contracts`. `row_id` is the composite key
of the row, joined as described below; `payload` is the row's columns as a JSON object; `op` is always `upsert`
(this cache is append-only: no tombstones, no `lww`, no per-row versions). `base_seq` is `0` and the push DTO's
`changed_at` is advisory only — the server ignores it for this channel: vector and OCR rows carry their local
`created_at`, contract rows carry the client clock at the moment the row was read for that request. Row identity
is the composite key, never a timestamp.

### 3.1 `derived_vectors`

Composite key `(account_id, contract_hash, input_hash)`; `row_id = "<contract_hash>:<input_hash>"` (both are
lowercase hex, so the join is unambiguous).

```json
{
  "contract_hash": "5f0c…",
  "input_hash": "9a73…",
  "dimensions": 1024,
  "embedding": "AAAAgD8AAAB…",
  "created_at": 1761600000000
}
```

`embedding` is **base64 of the raw little-endian `f32` values**, exactly `dimensions * 4` bytes. This mirrors
`vec_assets`, so the server stores it as an opaque blob and never converts it. A client accepts a received row
only when `dimensions > 0`, `len(blob) == dimensions * 4` and every value is finite; anything else is skipped
(never repaired, never published).

### 3.2 `derived_ocr_pages`

Composite key `(account_id, source_hash, page_number, ocr_contract)`;
`row_id = "<source_hash>:<page_number>:<ocr_contract>"` (`source_hash` is hex and `page_number` is numeric, so
the join is unambiguous even though `ocr_contract` contains `:`).

```json
{
  "source_hash": "3b1e…",
  "page_number": 7,
  "ocr_contract": "ocr-v1:glm:glm-ocr:p1",
  "outcome": "text",
  "text_content": "## Página 7\n…",
  "text_hash": "c0d4…",
  "created_at": 1761600000000
}
```

`outcome` is what the provider answered, not the row the Bibliography ends up publishing:
`"text"` (with `text_content` as the RAW Markdown answer) or `"empty"` (with `text_content = ""`). A definitive
provider failure is never cached: it is not a result and it may not repeat. `text_hash` is the lowercase hex
SHA-256 of `text_content` in UTF-8; a client accepts a received row only when `text_hash` matches its own text,
`outcome` is one of the two values, and an `"empty"` row carries no text.

### 3.3 `derived_contracts`

Composite key `(contract_hash)`; `row_id = "<contract_hash>"`.

```json
{
  "contract_hash": "5f0c…",
  "provider": "openrouter",
  "model": "text-embedding-3-large",
  "dimensions": 1024,
  "chunking_contract": "chunks-v1:800:100"
}
```

An immutable, content-addressed row: it names the embedding space a vector belongs to. The client mirrors it
locally when it records a paid vector and sends it **with the first vector of that contract in each push run**:
a contract row is read from the local mirror on demand and prepended to the batch that carries that vector, at
most once per contract per run (a run is one sync cycle's push phase, bounded to 64 requests). A contract with
no pending vector is never sent. If the server rejects that batch, the next vector of the same contract carries
the contract row again, so a rejected request never loses it. It carries no per-account secrets, but it travels
inside the account's channel like everything else.

## 4. Idempotency and account scoping

- **Insert if absent.** The server stores rows under `(account_id from the bearer token, table, composite key)`
  with `ON CONFLICT DO NOTHING`. The first writer wins; the same key always names the same content, so a
  repeated push, a retried batch or two devices racing are all no-ops. The server never has to compare
  payloads.
- **Account scope.** The account comes from the token, never from the payload. A derived pull answers only the
  calling account's rows. The client stamps and reads only its own session account: it never pushes a row of
  another account (and never `''`), and a response that arrives after a logout/account change is discarded
  before it can install a row, move a cursor, touch a capability record or set `pushed` (§2.9).
- **Quota.** §2.7 leaves quota per account to the server. A rejected push (`4xx` other than the client's own
  gates) leaves the rows pending locally; the client retries them in a later cycle and never marks them pushed.
- **Streamed outbox.** The client never builds the whole outbox in memory: it reads pending rows page by page
  (keyset order over each table's primary key, 256 rows per page) and emits one byte-bounded request at a time,
  sending and settling each before building the next. A backfill of ~100k vectors therefore drains over several
  cycles without a memory spike, and the server sees a sequence of independent, idempotent requests.
- Received rows are stored with `origin = 'remote'` and `pushed = 1`: they came from the server, so echoing them
  back would only cost bytes. A logout resets every `pushed` flag (rows are content, not session state), so a
  re-login of the same account re-pushes its local rows and the server ignores the duplicates.

## 5. Cursor and catch-up

- **Own cursor.** `sync_meta['derived_cursor']` is a JSON `{"scope": {account_id, server_url, device_id,
  server_epoch, session_incarnation}, "since": n}`. It is written only inside the transaction that installs the
  page it belongs to. A stored cursor whose scope differs from the current session is ignored (the pull restarts
  from `0`); it is never reused to skip rows.
- **Since-zero catch-up per epoch.** Once `bibliography-derived-v1` is advertised for an epoch and the epoch's
  catch-up has not run (`sync_meta['derived_catchup_epoch']`), the client pulls from `0` and keeps paging until
  `has_more` is `false`; then it records the catch-up for that epoch and, in the same transaction, consumes any
  pending pull request: a completed since-zero walk is strictly fresher than the incremental page the gate asked
  for. An interrupted catch-up does not consume it. A device whose corpus cursor already passed rows is never
  left without them, because this cursor never depended on the corpus one. A new epoch (`server_epoch` changed)
  drops the record and the catch-up runs again.
- **Incremental pulls.** After the catch-up, every page's `next_since` becomes the new cursor. The start time of
  the last successful incremental pull is stored in `sync_meta['derived_last_pull_started_at']` (milliseconds);
  the pull-before-pay gate of §2.6 reads it as its freshness signal.
- **On-demand pull.** `sync::derived::request_incremental_pull` sets `sync_meta['derived_pull_requested'] = '1'`.
  The derived phase performs at most one incremental pull per cycle, and it always performs it when the flag is
  set (even if a pull happened less than `MIN_PULL_SPACING_MS` ago); a successful incremental install consumes
  the flag, and a completed catch-up consumes it too. Many requests coalesce into that one pull. The engine never
  polls the channel faster than the spacing unless asked to.

## 6. Byte limits

- One request is bounded by the corpus limits: at most `MAX_PUSH_CHANGES` = 500 changes and at most the server's
  `max_push_bytes` (`GET /v1/health`, default `DEFAULT_MAX_PUSH_BYTES` = 8 MiB). The client sizes each batch from
  its rows' serialized JSON plus a per-row envelope overhead — an over-estimate of the request body — so a request
  never exceeds the budget. A single row larger than the limit still goes alone rather than being dropped, and if the
  server answers `413` to that one-row request the client skips the row for the rest of the cycle (it stays
  `pushed = 0` and is retried once per cycle) so the rows behind it keep flowing. The client logs one summary
  line per cycle for those skips.
- One pull page asks `limit` rows (the channel uses 500, inside the protocol's 1..=1000 range). A page's rows are
  applied in one client transaction; the size of a page is the server's ceiling on one response.
- Vectors dominate the volume (a 1024-dimension vector is 4 KiB raw, ~5.5 KiB in base64); a 400 MiB catch-up —
  the measured size of an account like the owner's (§1.5) — is therefore expected to be several thousand pages,
  which is why catch-up and pull loop budgets are separate and bounded per cycle.
