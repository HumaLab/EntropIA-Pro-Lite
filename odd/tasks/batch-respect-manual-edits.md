# Batches never overwrite text the person edited by hand

Branch `fix/batch-respect-manual-edits`.

## Specs

- **S1 OCR correction batch.** A page whose text the person edited by hand after the last automatic write (OCR, OCR correction, restore) is not eligible for the batch OCR correction: today `correction_decision` (`apps/desktop/src-tauri/src/processing/eligibility.rs:458-507`) treats any text that differs from the stored correction as eligible, so the batch sends the edited text to the model and replaces it.
- **S2 OCR batch with «Incluir texto nativo».** The same pages are not replaced by an OCR batch run with `ocr_include_native`: today `plan_member` (`processing/repository.rs:~6301`) and the claim check (`validate_corpus_claim_input`, `~:4964`) override the done verdict for `method='native'` extractions, and `publish_ocr_output` (`processing/ocr.rs:48`) replaces the edited text.
- **S3 Visible.** Where the batch tab shows why a page is skipped, a manually edited page says so («editada a mano» / "edited by hand"); counts stay consistent with what runs.
- **S4 Unchanged elsewhere.** Pages never edited by hand keep today's behavior in both batches; per-document commands are not changed.
- **S5 Re-transcribing an edited audio** asks before replacing a transcription the person edited (`transcription_edit`), if the UI offers re-transcription in that state; otherwise record that no path exists.

## Tasks

| ID | Specs | Route | Status | Commit |
|---|---|---|---|---|
| T1 | S1-S4 | worker: shared "edited by hand" rule + both batch decisions + claim + tests | done (206 processing + 7 asset_history tests) | (T1 commit) |
| T2 | S5 | worker: «Transcribir» on an edited transcription asks first | done (12 frontend, 43 transcription, 8 asset_history tests) | (T2 commit) |

## Log

- **L1** (user, verbatim): "bien, me parece bien, cubramos esos huecos" — after the explanation: the OCR-correction batch re-corrects pages edited by hand; audit transcription and Biblioteca.
- **L2** Audit (explorer, static): confirmed S2 (include-native) in addition to S1; transcription is not a batch op but `transcribe_audio` replaces an edited transcription (`transcription/mod.rs:727`, edit recorded as `transcription_edit`); Biblioteca: no overwrite or count gap confirmed; summary, NER, triples, embeddings, default OCR preserve manual edits.
- **L3** Signal: manual edits are recorded in `asset_process_events` as `text_edit` (`asset_history.rs:39`, coalesced per session, `record_text_edit`). That table is local only (not synced): an edit made on another device is not visible to this rule — record as a known limit unless a synced signal exists.
- **L4** T1 delivered: `asset_history::text_is_edited_by_hand` (latest of text_edit / ocr / ocr_correction / ocr_restore / layout_box_ocr / text_replace is a text_edit; ties ordered by rowid; no history table -> false); `CorrectionDecision::EditedByHand`; planning and claim guards for correction and include-native OCR. S3: the batch tab shows task states, not skip reasons, so no frontend change. Known limit (L3): history is local, edits made on another device are not seen.
- **L5** T2 delivered: UI path confirmed (ItemTextPanel «Transcribir» stays enabled once done). New read-only command `transcription_is_edited_by_hand` (asset_history rule: latest of transcription_edit vs automatic transcription is a transcription_edit). «Transcribir» on an edited transcription, or with an unsaved debounced edit, opens the existing ConfirmDialog; cancel does nothing; confirm flushes pending saves then transcribes. History lookup failure fails closed (asks).
