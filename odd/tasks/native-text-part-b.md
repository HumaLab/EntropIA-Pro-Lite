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
- [ ] B3 — `plan_reprocess`, `bibliography_reprocess_candidates`, `bibliography_reprocess_preview` (progress,
  cancel) and the USD estimate (plan 2.3, 2.4).
- [ ] B4 — Reprocess contract, `bibliography_reprocess_confirm` (own user batch, own admission, busy on
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
