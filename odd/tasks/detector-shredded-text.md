# Bibliography detector v3: shredded text

After the v2 reprocess ran to completion (batch-539c678e, 22 attachments,
finished 2026-10-10 14:41, 0 failed), Abulafia 1950 keeps 388 native pages:
358 rich (avg ~700 chars), 26 sparse, 4 empty. Owner-reviewed samples of the
rich ones are real garbage (`lJIanufacturas`, `J.mport.a-`,
`Cuadro N~7 G2Il yute`, tables of `r--- ¡ I , i -="`), missed by v2 because
the text is shredded into 1-2 char fragments: rule 2 never reaches its
10-judged-token gate and rule 3 excludes single repeated chars.
Branch `feat/detector-shredded-text`. Owner-authorized 2026-10-10.

Prototype (read-only on prueba-sync, `agent-scratch/detector/` scripts):
word = token with a run of 3+ Latin letters; shred = token of <= 2 chars that
is not a short word (es/en/fr/de/pt list) and not all digits. On rich native
rows: wordrate < 0.30 catches 240/304 Abulafia pages and 390 other-work pages;
adding shred >= 0.15 keeps most Abulafia catches while the remaining other-work
flags are mostly true positives (spaced-letter damage in 3 more works,
glyph-shift pages) plus TOC dot-leader and number-table pages as accepted
false positives (GLM reads those fine; owner accepts the overtreatment).

## Tasks

- [x] T1 — Detector v3 in Rust with tests: rule 4 shredded text, version bump
  to 3, unit tests from real samples (Abulafia shred, spaced-letter damage,
  TOC dot leaders and number tables as documented accepted FPs or guards).
- [ ] T2 — Measure v3 on a DB copy (recreate via the sqlite backup API; the
  940 MB copy was deleted twice already), update
  odd/reports/native-text-detector-measurement.md with a v3 section.
- [ ] T3 — Preview in tauri dev, record the new cost. No paid confirm without
  the owner.

## Evidence

- T1: RED observed (2 new detector tests failing under v2). GREEN:
  cargo test --lib 2460 passed; bibliography_reprocess 31 passed,
  bibliography_processing 167 passed (run after closing the dev app that
  locked the target exe); clippy -D warnings and fmt clean.
