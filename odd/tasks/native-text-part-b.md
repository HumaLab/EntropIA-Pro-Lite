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
- [ ] B1 — No automatic re-demand of stored text (plan 2.1): `extraction_is_settled` = matches source only;
  admission skips a failed/cancelled OCR attempt for the same file (fingerprint `mtime` prefix, incl.
  `source_changed`); `bibliography_extract` checkpoints survive terminal failure; empty GLM answer
  checkpointed as `Ok("")`.
- [ ] B2 — `is_garbled_bibliography_text` + `ocr_candidate_pages` + threshold measurement on a read-only copy
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
