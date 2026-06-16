-- Deterministic ids for one-per-asset tables (DESIGN §4.6). Rewrites existing
-- rows so two devices that OCR/transcribe the same asset converge on a single
-- server row. Nothing references these ids by FK (verified), so the rewrite is
-- safe. Additive — no rebuild of a synced table, so no re-seed is required.
UPDATE extractions SET id = 'ext-' || asset_id;
UPDATE transcriptions SET id = 'trx-' || asset_id;
UPDATE layouts SET id = 'lay-' || asset_id;
