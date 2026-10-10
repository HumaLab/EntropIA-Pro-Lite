# Audit remediation (plan.md)

Source: `plan.md` (remediation plan for `auditoria.md`). One branch/PR per task or small group of
related tasks; security never mixed with refactors. Work starts at Phase 0, then Phase 1.

## Phase 0 — Linux CI baseline (branch `ci/audit-phase-0`)

- [x] P0.1 (Q-05) — Lite `cargo test` passes on Linux. The `navegador/download.rs` half was already
  fixed in `5905a2dd`. Baseline (Lite, Linux): only
  `attachment_page_keeps_pdfs_..._decodes_the_enclosure` failed (RED: `C:/Libros/externo.pdf` is not
  absolute on Linux, so `native_path` was `None`). Production is right; the fixture now uses a
  platform-native absolute path, plus a non-Windows test that a drive-letter path is not local.
  GREEN: `bibliography_processing` 168/168; every other binary was already green (lib 2444 passed);
  clippy `-D warnings` and fmt clean.
- [x] P0.2 (C-01) — `rust-lite-linux` CI job on `ubuntu-22.04`: fmt, clippy `-D warnings`, test,
  no features, gated by `detect-rust-changes`. Commit `cd357702`. Pending: observe the job green on
  the PR (needs a push).
- [ ] P0.3 (E-02 verification) — Pro `.deb` on a clean Ubuntu 22.04 VM without the runtime: does
  Pdfium load? Manual, needs a VM; cannot run from this session.

## Phase 1 — Critical security (small changes)

- [ ] P1.1 (S-03) — renderer cannot overwrite the runtime bootstrap trust source.
- [ ] P1.3 (S-04) — `prosemirror-view` >= 1.42.3.
- [ ] P1.2 (S-01) — narrow the `fs` plugin scope (inventory first).

## Later phases

- [ ] Phase 2 — vulnerable dependencies (D-01, D-02, D-08, D-03, D-04)
- [ ] Phase 3 — SQL IPC and asset protocol hardening (S-02, S-05, A-04, A-05, S-06)
- [ ] Phase 4 — migrations unified in Rust (A-01, A-03, A-02)
- [ ] Phase 5 — performance and observability (P-01..P-04)
- [ ] Phase 6 — packaging and release (E-02 fix, E-01, E-03, C-02, C-04, D-07, E-04)
- [ ] Phase 7 — CI supply chain (C-03, D-05, D-06, C-05)
- [ ] Phase 8 — quality, hygiene, docs (Q-01..Q-04, R-01..R-05, A-06, S-07, C-06)

## Notes

- The hlab task board API answered 401 ("Clave incorrecta o ausente") on 2026-10-09; no cards moved.
