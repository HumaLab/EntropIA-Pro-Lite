# Patched tao

Upstream: `tao 0.35.3` (crates.io), the version Tauri 2.11.x resolves to.
Only change: backport of tauri-apps/tao#1215 ("avoid reentrant input lock
deadlocks", merged on `dev` 2026-06-10, shipped in tao 0.36+/0.37).

## Why

On Windows, `KeyEventBuilder::process_message` called `PeekMessageW` while the
global `KEY_EVENT_BUILDERS` mutex (and `LAYOUT_CACHE` / the window state mutex
in the IME path) was held. `PeekMessageW` dispatches pending cross-thread
`SendMessage` calls (WebView2 does this) into the same window procedure, which
tried to lock the same non-reentrant `parking_lot` mutex on the same thread.
Result: 0% CPU, window "Not Responding" while typing.

## What changed

The next queued key message is peeked in `event_loop.rs` before any lock is
taken and passed into `KeyEventBuilder::process_message` / `MinimalIme`.
`LAYOUT_CACHE` is held only for short scopes and the keyboard state is read
without it. Verbatim #1215, plus: the "does this message peek?" decision is
extracted into `needs_next_key_message` with a unit test, and the `[[example]]`
entries were dropped from `Cargo.toml` (examples are not vendored).

## When to drop

Remove this directory and the `[patch.crates-io]` entry in
`../../Cargo.toml` once Tauri is on a release that resolves `tao >= 0.36`
(Tauri 2.12 uses tao 0.37.1, but needs Rust 1.90; this repo pins 1.88).
