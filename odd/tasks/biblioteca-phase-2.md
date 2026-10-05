# Biblioteca phase 2

## Objective

Finish the Zotero bibliography layer after phase 1 (`biblioteca-scope.md`) was
verified end to end by the owner on 2026-10-05. Four goals, each a separate
feature slice.

## Where phase 1 left things (main at `ca29890`, CI green incl. Pro)

- Library sync: works, PDF and HTML attachments, OCR of scanned PDFs (GLM
  whole-PDF up to 100 pages), embeddings batched (3 in flight, 429 backoff),
  index generation activation, interrupted system work resumes, honest sync
  button with status polling.
- Search: chat scope Corpus/Biblioteca/Ambos with passage citations; Obras
  passages (lexical + fuzzy + vector, persistent search); Zotero tab
  accent-insensitive + fuzzy + "Por contenido"; TopBar "Biblioteca" group;
  "Abrir original" opens the PDF in-app (one-file runtime asset scope grant).
- Speed: cached int8 vector index (`bibliography/vector_index.rs`,
  `crates/vecscan` at opt-level 3 even in dev), warmed at startup.
- Storage: checkpoints deleted at task end; one-time compaction at close
  (`auto_vacuum = INCREMENTAL` afterwards).
- Windows: vendored `tao` 0.35.3 with the upstream tao#1215 backport
  (`apps/desktop/src-tauri/vendor/tao/PATCHED.md`).

## Owner rules that apply to every task

- Strict visual homogeneity: reuse existing components and tokens; icons via
  `ActionIcon`; search inputs only via `SearchBar`; no native `<select>`.
- Lite and Pro share the code; CI verifies Pro (`local-ml`).
- Work in a git worktree under `G:/EntropIA-Stack/EntropIA-Pro-Lite-worktrees/`:
  the owner runs `tauri dev` from the primary folder and it rebuilds on every
  Rust edit there.
- Test with the dev profile `navegador` (`G:\EntropIA-Stack\agent-scratch\devrun-navegador.ps1`).
  Avast must be off: it blocks the network of freshly rebuilt unsigned builds.
- No SQL migration without asking the owner first.
- TDD: RED first, then GREEN. Conventional Commits, no AI attribution.

## Tasks

- [ ] P1 — Investigación can use the Biblioteca. The research agent lives in
  the external crate `entropia-agent` (`G:\EntropIA-Stack\EntropIA-Agent`,
  pinned in `Cargo.lock` by git rev; CI's Pester guard checks the pin; the
  "Engine pin bump" workflow exists). Scope choice like the chat (Corpus /
  Biblioteca / Ambos), frozen in the job snapshot; bibliography passages as a
  source with citations "p. N" / "párr. a–b"; reuse `rag/scope.rs` passage
  search. Pushing EntropIA-Agent needs the owner's authorization.
- [ ] P2 — Top-level "Biblioteca" section in the navigation, built like
  Colecciones: each Zotero work is an item/document; the item view shows the
  page viewer (PDF pages, in-app via the per-file asset scope grant) and the
  extracted-text tab, metadata, attachments, the same actions where they make
  sense. Explore the Colecciones/ItemView code first and propose how to reuse
  it (bibliography tables are local-only and are not `items`/`assets`); ask
  the owner before any migration.
- [ ] P3 — First sync of a large library: process every work's profile first
  (works search by meaning usable early), then passages, recent or opened works
  first; show progress and an estimated time in the Zotero tab (and status
  bar) instead of a generic message. The sync status command
  `processing_bibliography_sync_status` already exists.
- [ ] P4 — Upgrade to Tauri 2.12 (needs Rust 1.90 in `rust-toolchain.toml`)
  and drop the vendored `tao` patch once the upstream fix is in the resolved
  `tao` (tao#1215, merged 2026-06-10). Check every Tauri plugin version, the
  app ACL manifest guards, both variants' builds and the Windows release build
  (`build.rs` runtime-bootstrap guard, VC runtime staging).
  Branch `build/tauri-2.12`, worktree `EntropIA-Pro-Lite-worktrees/tauri-2.12`.
  - [x] P4.1 Rust 1.88 -> 1.90: `rust-toolchain.toml`, `rust-version` in
    `apps/desktop/src-tauri/Cargo.toml` and `crates/vecscan/Cargo.toml`,
    workflow toolchains (`lite-preview.yml`, `release.yml`), READMEs; clippy
    `-D warnings` clean on 1.90.
  - [ ] P4.2 Tauri 2.12.x: `tauri`/`tauri-build`/plugins (dialog, fs) in
    Cargo, `@tauri-apps/api`/`cli`/plugins in `package.json`; lockfiles;
    `entropia-agent` pin unchanged; `app_acl` rejection strings re-checked
    against 2.12 sources.
  - [ ] P4.3 Drop `vendor/tao` and the `[patch.crates-io]` entry; the
    resolved `tao` is >= 0.36 and contains tao#1215.
  - [ ] P4.4 Verify: cargo check lean + `local-ml`, `cargo test
    -- --test-threads=1`, frontend lint/typecheck/test, CI green incl. Pro,
    owner's typing check in the dev app (no keyboard freeze).

## Acceptance

Each task: tests green (frontend + `cargo test -- --test-threads=1`), CI green
including Pro, and the owner's visual check in the dev app.

## Progress

- 2026-10-05: plan opened.
