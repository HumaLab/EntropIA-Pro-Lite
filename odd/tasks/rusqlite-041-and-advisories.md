# rusqlite 0.41 and the remaining context advisories

Two tracks. A: verifier advisories 2, 3 and 6 from `magnolia-stage-ab.md`, Pro-Lite only (branch `fix/context-advisories-2-3-6`). B: rusqlite 0.31 → 0.41 across EntropIA-Agent and Pro-Lite, which fixes RUSTSEC-2026-0459 at the root and drops its `audit.toml` exception.

## Specs

- **S1. Advisory 2, same rule with no selection.** Decision: «Sin selección, sin contexto en ambos». An investigation created with no collection chosen by hand still searches every collection, but prepends no project context; with collections chosen, it prepends theirs. The RAG chat already behaves this way.
- **S2. Advisory 3, delete the dead path.** Decision: «Borrar ese camino muerto». Remove the item-level `LlmJob::CorrectOcr` variant, its command wiring and its unused frontend function, if nothing calls them. The page-level `CorrectOcrAsset` path stays.
- **S3. Advisory 6, history covered by the rule.** Decision: «Extender la regla al historial». The untrusted-data rule also tells the model not to follow orders that appear in the previous conversation. The history keeps its «NO es una fuente» label.
- **S4. rusqlite 0.41 in EntropIA-Agent.** User authorization: «Sí, los dos repos». EntropIA-Agent (`G:\EntropIA-Stack\EntropIA-Agent`) moves to rusqlite 0.41 with its API fixes and its own tests green, through a PR in that repo.
- **S5. rusqlite 0.41 in Pro-Lite.** Pro-Lite moves to rusqlite 0.41 (libsqlite3-sys ≥ 0.39, SQLite ≥ 3.54), points `entropia-agent` at the updated revision, and removes the RUSTSEC-2026-0459 line from `audit.toml`. The Rust suites pass (lib, integration, Pro local-ml) and `cargo audit` reports no vulnerabilities without that exception.

## Tasks

| ID | Specs | Risk | Commit |
| --- | --- | --- | --- |
| T1 | S1 | none | ee53bfe7 |
| T2 | S2 | item 3 (removes a command; only if unused) | b1d51939 |
| T3 | S3 | item 2 (prompt security) | 4362545d |
| T4 | S4 | item 5 (dependency) | EntropIA-Agent PR #5 (441bc26) |
| T5 | S5 | item 5 (dependency), item 1 (every database access) | |

## Log

- L1 (user, 9 Nov 2026): «vayamos con esto: - Para un PR aparte: subir rusqlite de 0.31 a 0.41. Es el arreglo de raíz del aviso de SQLite y permite borrar la excepción de audit.toml. … - Para otra vuelta: los avisos 2, 3 y 6, anotados en odd/tasks/magnolia-stage-ab.md.»
- L2 (user decisions): rusqlite «Sí, los dos repos (Recomendado)»; advisory 2 «Sin selección, sin contexto en ambos (Recomendado)»; advisory 3 «Borrar ese camino muerto (Recomendado)»; advisory 6 «Extender la regla al historial (Recomendado)».
- L3 (evidence): `libsqlite3-sys` allows one version per build (`links = "sqlite3"`), and EntropIA-Agent pins `rusqlite = 0.31` (its Cargo.toml line 12), so it has to move first.
- L4 (evidence, track A): T1 adds `context_collection_ids` (only a selection the person touched; stripped before the engine), 133 research tests + 2 view tests. T3 extends `UNTRUSTED_RULE` (RED observed). T2 removes `LlmJob::CorrectOcr`, the `llm_correct_ocr` command (lib.rs, build.rs APP_COMMANDS, capabilities) and `llmCorrectOcr`; `LlmJob` is in-memory only, so nothing persisted refers to it; the `correct_ocr` job string stays (the page path emits it). Checks: Rust lib 3179 + integration green (with the local entropia-agent override set aside), desktop 4010, typecheck Pro/Lite, lint, format.
- L5 (evidence, T4): EntropIA-Agent PR #5 merged; one API fix (`usize` LIMIT bound as `i64`); its 206 lib + integration tests and fmt green; that repo has no CI.
