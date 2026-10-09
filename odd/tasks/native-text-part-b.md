# Native text — part B: repair what is already stored

Plan: `odd/plans/plan-texto-nativo.md` sections 3.2–3.4, refined in `odd/plans/plan-texto-nativo-parte-b.md`.
Part A (`odd/tasks/native-text-part-a.md`) is merged on main (49a5f42).

Branch `feat/native-text-reprocess`, worktree
`G:/EntropIA-Stack/EntropIA-Pro-Lite-worktrees/native-text-reprocess`.

## Scope rules

- No automatic content-based re-demand, ever. Every paid OCR is approved by the owner with the cost visible.
- No change to `attachment_extraction_fingerprint` (JD5-A-002).
- No SQL migration without asking the owner.
- Strict visual homogeneity with existing Biblioteca components.

## Tasks

- [x] B0 — Write the part B plan and run Judgment Day on it before implementing.
- [x] B1 — No automatic re-demand of stored text (plan 2.1): `extraction_is_settled` = matches source only;
  admission skips a failed/cancelled OCR attempt for the same file (fingerprint `mtime` prefix, incl.
  `source_changed`); `bibliography_extract` checkpoints survive terminal failure; empty GLM answer
  checkpointed as `Ok("")`.
- [x] B2 — `is_garbled_bibliography_text` + `ocr_candidate_pages` + threshold measurement on a read-only copy
  of `prueba-sync` (plan 2.2).
- [x] B3 — `plan_reprocess`, `bibliography_reprocess_candidates`, `bibliography_reprocess_preview` (progress,
  cancel) and the USD estimate (plan 2.3, 2.4).
- [x] B4 — Reprocess contract, `bibliography_reprocess_confirm` (own user batch, own admission, busy on
  conflict) and executor reprocess mode (plan-hash gate, converted reuse, page-only OCR) (plan 2.3).
- [ ] B5 — UI: Biblioteca toolbar button, work detail button, preview/confirm dialog (plan 2.5).
- [ ] B6 — Local verification, CI green incl. Pro, Judgment Day on the code, merge.

## Progress

- 2026-10-08: tasks opened; worktree created from main 49a5f42.
- 2026-10-08: B0 done — plan `e69242b` judged (JD7: 2 confirmed HIGH, 4 single-judge HIGH, info), revised
  in `1bf1be0`, scoped re-judgment approved with two warnings folded in. JUDGMENT: APPROVED.
- 2026-10-08: B1 done — settled = source identity only; admission skips failed (OCR codes +
  source_changed) or cancelled attempts on the same `mtime` prefix; extract checkpoints survive terminal
  failure; empty GLM answer checkpointed. RED observed on 9 tests; lib bibliography 187 passed,
  bibliography_processing 165 passed, clippy and fmt clean. Residuals for the code judgment:
  `source_changed` never lands on a `failed` row today (`record_source_change` requeues the same task
  with a re-pinned fingerprint, so its old-fingerprint checkpoints are not reused); a cancelled task
  still wipes its checkpoints (`cancel_running_task`).
- 2026-10-08: B2 done — `is_garbled_bibliography_text` (v1) in `ocr/pdf.rs`, wired into
  `ocr_candidate_pages`; RED on 4 new positives, lib 2415 passed, bibliography_processing 165 passed,
  clippy and fmt clean. Measurement on a backup copy of `prueba-sync`
  (`odd/reports/native-text-detector-measurement.md`): PDFium text rule 1 = 0 pages, rule 2 = 421
  (418 in the scanned Abulafia 1950 book); stored text flagged = 1,610 rows in 82 attachments; FP upper
  bound 3/18,576 (0.016 %) < 0.5 %. Thresholds unchanged. Known flake outside scope:
  `navegador::zotero_copy::run::tests::only_one_drain_runs_at_a_time_and_the_guard_lets_go` under load.
- 2026-10-09: B3 done — `bibliography/reprocess.rs` (pure `plan_reprocess`, canonical `plan_hash` without the
  contract, USD estimate), shared pre-OCR basis `read_native_extraction_basis` in `processing.rs`, commands
  `bibliography_reprocess_candidates` / `_preview` / `_preview_cancel` (+ APP_COMMANDS and capability).
  RED on 7 unit + 7 integration stubs; lib bibliography 197, bibliography_processing 165,
  bibliography_reprocess 7, acl_manifest_guard 6 passed; clippy and fmt clean. Measured on the copy (debug):
  103 candidates (84 garbled stored pages, 19 empty without OCR) in 25-35 s; preview ~3 s per attachment.
  Follow-ups: B4 removes the duplicated B1 attempt rule in `reprocess.rs` by exposing the
  `processing/repository.rs` helpers; B5 shows candidates loading as a running state; release timing
  unmeasured.
- 2026-10-09: B4 done — `parse_extract_contract` at the three contract sites; `bibliography_reprocess_confirm`
  (own `origin='user'` batch, `operations=["ocr"]`, priority 2, `BEGIN IMMEDIATE`, plain INSERT, UNIQUE
  collision = busy, never attaches); executor reprocess mode (plan recomputed from the re-read bytes,
  `reprocess_authorization_stale` before any provider call, converted OCR reuse, page-only OCR); receipts
  carry `sourceMtime`/`sourceBytes` (+ `sourceSha256` and `reprocess` block); B1 rule duplicates removed.
  RED on 12 integration tests + the contract parse test; lib 2425, bibliography_processing 165,
  bibliography_reprocess 19, acl_manifest_guard 6 passed; clippy and fmt clean. The reprocess plan reads
  PDFium through the path app setup resolves (`lib.rs:729`). Batch tab renders the batch by id with
  "OCR ✓ / Embeddings —" and task-count progress (static check; B5 checks it live).
