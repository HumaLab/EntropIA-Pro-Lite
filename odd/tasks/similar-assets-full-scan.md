# Similar assets — compare against every compatible page, not the first 2000

`similar_assets` (apps/desktop/src-tauri/src/nlp/commands.rs) scored an unordered `LIMIT 2000` subset of
`vec_assets`, so a closer page outside that subset never surfaced. Plan reviewed by Judgment Day (APPROVED,
JD-A-001 fixed in round 1); branch `fix/similar-assets-full-scan`.

## Specs

- S1 — Goal (L1): "recorrer todos los embeddings sin el tope de 2000, calcular la similitud de a una fila y quedarse solo con las 5 mejores. Es exacto, no agrega dependencias y el repo ya hace lo mismo en el RAG."
- S2 — Full scan test (L1, paso 1): "con más de 2000 vectores, el más parecido insertado al final tiene que aparecer en los resultados" — and the old query must be shown to exclude that winner in the fixture (L2, aviso 4).
- S3 — Effective contract (L2, JD-A-001): "resolver una vez el contrato efectivo con resolve_effective_embedding_contract(conn) [...] El contrato no se toma de la fila del asset de origen"; "si resolve_effective_embedding_contract falla (proveedor desconocido), propagar el error [...] sin comparar."
- S4 — Contract filter tests (L1, L2): "un vector de otro embedding_contract con la misma dimensión tiene que quedar afuera"; "un vector con el mismo embedding_contract y dimensions pero otro embedding_model tiene que quedar afuera".
- S5 — Incompatible source (L2): "si el vector del asset de origen no coincide con el contrato efectivo (por ejemplo, una fila legacy/legacy/0), el resultado es una lista vacía, aunque haya otros vectores legacy del mismo largo."
- S6 — Scoring (L2): "similarity = 1.0 - cosine_distance_to_blob(...), descartando no finitos"; numeric checks "idéntico → 1, ortogonal → 0, opuesto → −1".
- S7 — Top-k (L2): "conservar los limit mejores en un heap de tamaño fijo, con desempate por asset_id"; "limit 0, 1 y 7 [...] devuelven exactamente esa cantidad, todos hidratados".
- S8 — Hydration (L2): "al final, traer título, ruta y texto de esos ganadores con una sola consulta, y reordenarlos por similitud descendente". "El JSON que recibe ItemView.svelte no cambia de formato."
- S9 — Existing tests (L2): "los tests actuales siguen pasando (compara entre colecciones y se excluye a sí mismo)", with `setup_similar_assets_test_db` updated to the migration-0028 columns and canonical stamps.
- S10 — Measure (L2, paso 5): "benchmark con ~100.000 filas sintéticas, compilado en release, con la base fría y en caché, y con db_select concurrente midiendo la espera por el lock de ui_conn. Si el comando supera ~500 ms o la UI se traba visiblemente, se evalúan C, D o sacar el recorrido del lock de ui_conn en una tarea aparte."

## Tasks

- [x] T1 — S1-S9 — inline, test-first: full scan, effective-contract filter, top-k heap, hydration, fixture — commit: 54e405f8 (726 authored lines, one behavior unit: ~270 code, rest tests/fixture; slice boundary)
- [x] T2 — S10 — inline: ignored release benchmark + recorded numbers — commit: this commit (test(nlp): benchmark the similar assets full scan)
- [ ] T3 — S1-S10 — gentle-ai-verify (Risk item 4: scan holds ui_conn) — commit: n/a

## Log

- L1 — Owner plan (verbatim excerpt): "Plan para Assets similares: comparar contra todas las páginas, no contra las primeras 2000. Recomendación: recorrer todos los embeddings sin el tope de 2000, calcular la similitud de a una fila y quedarse solo con las 5 mejores."
- L2 — Revised plan after Judgment Day (JD-A-001 verified; warnings folded in), accepted by the owner: "dale" to step 0 then implementation on a new branch.
- L3 — Step 0 evidence: active DB `~/.local/share/com.entropia.shared/entropia.sqlite` (migration 0059) has 1367 `vec_assets` rows, all `baai/bge-m3 | bge-m3-6000-char-weighted-mean-l2-v1 | 1024`, 4096-byte blobs; settings `embedding_provider=api`, `openrouter_embedding_model=baai/bge-m3`. The effective contract matches every row, so the filter hides nothing today.
- L4 — Exploration: `rank_similarity_rows` has no caller besides this command and its unit test; `extractions` and `transcriptions` are unique per asset (migration 0016); with no settings the effective contract is canonical (`DEFAULT_OPENROUTER_EMBEDDING_MODEL = CANONICAL_EMBEDDING_MODEL`).
- L5 — RED evidence: 4 rule tests failed for the expected reasons. The first fixture guard caught a wrong premise: with
  `asset_id TEXT PRIMARY KEY`, SQLite walks the primary-key index for `WHERE asset_id != ?1 LIMIT 2000`, so the old cap
  took the first 2000 by asset id (UUID order in real archives), not insertion order. Winner renamed `zz-winner`.
- L6 — GREEN: focused 21/21; Pro (`--features local-ml`): clippy clean, 29 binaries ok (lib 3354 passed, 14 ignored); Lite `cargo fmt`, `clippy --lib --tests -D warnings` clean, full `cargo test` 28 binaries ok
  (lib 3082 passed, 14 ignored). Pro first attempt blocked before compiling the crate: `~/.local/bin/cmake` is a pip shim
  for a missing module; rerun with `CMAKE=~/.local/lib/python3.10/site-packages/cmake/data/bin/cmake` (environment only).
- L7 — Follow-up candidate if S10 is slow: `crates/vecscan` already provides an in-memory vector index used by
  `bibliography/vector_index.rs`, optimized even in dev (`Cargo.toml:173`).
- L8 — S10 benchmark (`cargo test --release --lib similar_assets_scan_benchmark -- --ignored --nocapture`, 100,000
  canonical 1024-dim vectors, `open_archive_connection` pragmas): fresh connection 260 ms; warm min 249 / median 260 /
  max 266 ms; a concurrent UI read waited 239 ms on `ui_conn`. Under the ~500 ms threshold, so C/D are not triggered.
  Not measured: cold disk (dropping the OS page cache needs root) and the unoptimized dev profile.
