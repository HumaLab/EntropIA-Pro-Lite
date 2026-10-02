# Zotero Web API key in Settings

## Objective

Let the owner store a Zotero Web API key in Settings, next to the other API
keys, and use it where the key-less connector falls short: completing the
data of an item that already exists in Zotero, and attaching a captured PDF
to it. Requested by the owner on 2026-10-02 ("la api de zotero la tengo…
ponemos las otras APIs ahí").

## Problem

Copy to Zotero (phase 5 of `navegador.md`, merged at `5a0b7e55`) goes through
the local connector (`/connector/saveItems`, `saveAttachment`). The connector
can create items and link them, but it cannot update an existing item or add
an attachment to one. The Zotero Web API (api.zotero.org, v3) can, with a key
that has write access to the library.

## Scope

- Settings: a Zotero card in the same place and style as the other API keys.
  The key is a secret: stored through the existing secret settings path
  (`settings.rs`, OS keyring), never echoed back. Validating it reads
  `GET /keys/current` and shows the user id and which libraries it can write.
- Copy to Zotero: when the item already exists and a valid key is present,
  fill missing fields (never overwrite non-empty ones) with a versioned write,
  and upload the PDF as a child attachment through the Web API file-upload
  flow. Without a key, keep today's behaviour and its note.

## Constraints

- Web API writes reach the local Zotero only after Zotero syncs; the UI says so.
  File uploads count against the owner's Zotero storage quota.
- No telemetry of the key; no key in logs or errors.
- ACL: every new command goes into `build.rs` `APP_COMMANDS` and
  `capabilities/default.json`; the guard tests must pass.
- TDD strict (Vitest + cargo test). Lite and Pro share the code.

## Tasks

- [x] Z1 — Settings card + secret storage + key validation (`/keys/current`).
  Route: delegated writer (2+ non-trivial files). Commit `7151ebc6`.
  Evidence (RED observed first for Rust and Vitest, then GREEN): `pnpm lint`
  0 errors (pre-existing warnings only); `pnpm typecheck` 0 errors;
  `pnpm format:check` clean; `pnpm test` desktop 230 files passed;
  `cargo fmt --check` clean; `cargo clippy --all-targets -- -D warnings`
  clean; `cargo test` all green (2098 lib tests + ACL guards). One first
  run failed `zotero_copy::port::tests::nothing_listening_is_unreachable...`
  and passed on rerun: `test_server::dead_base` frees a port that a parallel
  test server can take. Z2/Z3 call `zotero_web::stored_credentials(conn)`
  (key + user id; `None` until the key is saved and verified).
- [x] Z2 — Copy to Zotero completes an existing item's missing fields via the
  Web API. Route: delegated writer. Commit `9e28b7e1` (follow-ups: group names
  `f6de31f3`, deterministic dead address `603bbf2d`).
  Behaviour: an existing item is read (`GET /users|groups/{id}/items/{key}`),
  only fields empty in Zotero and present in our source are patched (never a
  non-empty one, never creators, `websiteTitle` only on a webpage) with
  `If-Unmodified-Since-Version`; `412` re-reads once and retries once, then
  reports `conflict`. Needs a stored key re-verified at copy time with write
  access to that library; otherwise today's behaviour (`no_key`, `no_write`,
  `invalid_key` recorded in `detail.web.state`). A Web API failure never fails
  the copy. Result notes say what was completed and that it reaches the local
  Zotero after Zotero syncs.
  Evidence (RED observed first for Rust and Vitest): `pnpm lint` 0 errors;
  `pnpm typecheck` 0 errors; `pnpm format:check` clean; `pnpm test` all
  green (desktop 230 files); `cargo fmt --check` clean; `cargo clippy
  --all-targets -- -D warnings` clean; `cargo test` 2125 lib tests + ACL
  guards green. Live check (group "prueba" 6680944 only, throwaway item
  created and deleted): key can write to the group; first run completed
  `accessDate` and `websiteTitle`, read-back showed them filled and the title
  untouched; second run `NothingMissing`; a stale precondition returned 412;
  delete 204, then 404.
- [ ] Z3 — Copy to Zotero attaches the PDF to an existing item via the Web API
  upload flow. Route: delegated writer.
- [ ] Z4 — Owner's manual check in the app (Lite).

## Progress

- 2026-10-02: opened.
- 2026-10-02: Z1 done (`7151ebc6`).
- 2026-10-02: Z2 done (`9e28b7e1`), live-checked on group prueba.
