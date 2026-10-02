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

- [ ] Z1 — Settings card + secret storage + key validation (`/keys/current`).
  Route: delegated writer (2+ non-trivial files).
- [ ] Z2 — Copy to Zotero completes an existing item's missing fields via the
  Web API. Route: delegated writer.
- [ ] Z3 — Copy to Zotero attaches the PDF to an existing item via the Web API
  upload flow. Route: delegated writer.
- [ ] Z4 — Owner's manual check in the app (Lite).

## Progress

- 2026-10-02: opened.
