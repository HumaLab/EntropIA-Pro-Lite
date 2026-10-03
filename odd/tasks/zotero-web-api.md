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
- [x] Z2b — Account guard for the personal library. Commit `4e76872c`. Before
  any Web API write to the personal library the open Zotero's account is read
  from the local API (`/api/users/0/items/top?limit=1`, the `library.id` of the
  first item) and compared with the key's verified user id. A different account
  records `other_account`, an unreadable one (empty library, Zotero closed)
  records `account_unknown`, and nothing is written; groups are addressed by
  group id and need no check. Each outcome has its own note.
- [x] Z3 — Copy to Zotero attaches the PDF to an existing item via the Web API
  upload flow. Route: delegated writer. Commit `35d6eafd`.
  Behaviour: behind the same guard, children of the item are listed first and
  the PDF is skipped when one has the same md5 (or, with no md5 yet, the same
  filename, which carries the file's hash). Otherwise an `imported_file`
  attachment child is created (write token), upload is authorized
  (`md5/filename/filesize/mtime`, `If-None-Match: *`), `{"exists":1}` needs no
  upload, the bytes go to the storage address with prefix/suffix and without
  the API key, and the upload is registered (`upload=<key>`). An attachment
  that could not get its file is deleted again. `413` is the `quota` outcome;
  anything else is `failed`; neither fails the copy. Notes say the PDF reaches
  the local Zotero after Zotero syncs, files included. New dependency: `md-5`
  (already in `Cargo.lock` through another crate; git pin line intact).
  Evidence (RED observed first for the Rust parts; the Vitest additions were
  written together with their code): `pnpm lint` 0 errors; `pnpm typecheck` 0
  errors; `pnpm format:check` clean; desktop `pnpm test` 230 files green;
  `cargo fmt --check` and `cargo clippy --all-targets -- -D warnings` clean;
  `cargo test` 2154 lib tests + ACL guards green. Live check (group "prueba"
  only; throwaway parent, tiny PDF generated in the temp dir, all deleted):
  first attach `Attached` (child `imported_file`, `application/pdf`, md5
  matches, the stored file downloads byte-identical); second run
  `AlreadyThere`, still one child; attachment and parent deleted, parent then
  404.
- [x] Z4 — Owner's manual check in the app (Lite). Passed on the owner's
  machine after the fix below. Result: after re-saving the key, the
  source-level copy of articulos/230 offered "Completar en Zotero" and attached
  the PDF (server: JC4DPPR4 has child ZP42WAPB, `imported_file`,
  `web-capture-0a41e561.pdf`; the local Zotero shows it too); a second copy
  says "El PDF ya estaba adjunto" and offers only "Abrir en Zotero". Follow-up
  UI fixes from that run, commit given below. Also from the run: a test deleted
  the owner's real keyring entry through `persist_setting(ZOTERO_API_KEY, "")`;
  fixed in `d83f3809` (tests must never touch the real credential store; use
  `persist_setting_with` with stubs).
  UI fixes after the owner's pass, commit `d1531d1f`:
  - Only the SOURCE-level "Copiar a Zotero" button remains; the one inside each
    PDF capture is gone, with the dialog's `capture` prop, the `introPdf`
    strings and the tests that only served it. The backend keeps `capture_id`
    (the rows and the picked PDF still use it); the dialog always asks with
    `captureId: null` and the backend picks the latest PDF.
  - The result line for an existing item no longer contradicts itself: it says
    "no se duplicó y se agregaron los datos que faltaban" / "...y se adjuntó el
    PDF" / both, only for what the Web API really did (`added.fields`,
    `added.pdf`); with nothing changed, or a failed step, it keeps "no se
    duplicó ni se modificó el elemento existente".
  - "Se adjuntará el PDF guardado el ..." is shown only when an attach will
    really happen: a new item, or an existing one that can be completed and
    lacks the PDF. Hidden when the status says it is already attached, when it
    cannot be attached without a key, and for an existing item with no PDF.
    Spanish and English.
  Evidence for these: `pnpm lint` 0 errors, `pnpm typecheck` 0 errors, `pnpm
  format:check` clean, desktop `pnpm test` 230 files green (RED observed first
  for the dialog and describeCopy tests), `cargo fmt --check`, `cargo clippy
  --all-targets -- -D warnings` and `cargo test` (2170 lib tests + ACL guards)
  clean and green with the coordinator's `d83f3809` included.
  First run (dev profile `navegador`): key card OK, completing fields OK, PDF
  NOT attached. Findings from the coordinator's read of the dev DB:
  1. The copy was started from the SOURCE's "Copiar a Zotero" button
     (`capture_id` empty), and a source-level copy carried no PDF at all; the
     sources drawer has two buttons and the source one was used.
  2. `web.state = failed` hid the cause, so a 404 on an item that exists only
     in the local Zotero (created through the connector minutes earlier, not
     synced yet) read as a generic failure.
  3. Not in the report, found while fixing: a finished (`copied`/`linked`) row is
     never requeued by `request()`, so "copy again after sync" did nothing, and
     the dialog only offered "Abrir en Zotero" for an item that was present.
  Fix, commit `a511baa0`:
  - Diagnostics: failures record `phase:cause` (`key_info`, `read_item`,
    `patch`, `children`, `create_attachment`, `authorize`, `upload`,
    `register`; cause is the HTTP status, `network` or `invalid`) in
    `detail.web.reason` / `pdfReason`, never the key, a URL or a body. Shown as
    "Detalle técnico: ..." in the dialog result and the sources list.
  - A 404 on `read_item` is the distinct state `not_synced_yet` (no PDF
    attempt), with a note to copy again after Zotero syncs. No retry machinery.
  - A source-level copy takes its latest saved PDF capture (readable, hash
    verified, within the size limit; an unusable newer one falls back to the
    next, and a page copy never fails for it). Both the connector create path
    and the existing-item Web API path attach it. The status says which PDF
    goes along (the day it was saved) or that none does, and the dialog shows
    it.
  - "Copy again" works: `request()` requeues a finished row whose Web step can
    still change (`not_synced_yet`, `failed`, `conflict`, PDF `failed` or
    `quota`); a settled one (completed, no key, another account...) is left
    alone. The dialog offers "Completar en Zotero" for a present item when a
    key is stored and the item lacks fields or the PDF (`canComplete`, computed
    in Rust), otherwise still "Abrir en Zotero".
  Evidence (RED observed first; the Vitest additions were written with their
  code in some places): `pnpm lint` 0 errors; `pnpm typecheck` 0 errors; `pnpm
  format:check` clean; desktop `pnpm test` 230 files green; `cargo fmt --check`
  and `cargo clippy --all-targets -- -D warnings` clean; `cargo test` 2170 lib
  tests + ACL guards green. Live check (group "prueba" only; throwaway parent
  with title and url, all deleted): a source-level copy in a test archive with
  two PDF captures ran the existing-item path against the real Web API:
  `web.state completed` (accessDate, websiteTitle), `web.pdf attached`, one
  `imported_file` child with the newer capture's md5, title untouched; deleted,
  then 404.

## Progress

- 2026-10-02: opened.
- 2026-10-02: Z1 done (`7151ebc6`).
- 2026-10-02: Z2 done (`9e28b7e1`), live-checked on group prueba.
- 2026-10-02: account guard (`4e76872c`) and Z3 (`35d6eafd`) done; Z3 live-checked on group prueba.
- 2026-10-02: owner's Z4 run found the PDF not attached; fixed in `a511baa0` (diagnostics, not_synced_yet, source-level PDF, copy again). Z4 stays open for the recheck.
- 2026-10-02: Z4 passed on the owner's machine; UI fixes (single copy button, honest result and PDF lines) in `d1531d1f`.
