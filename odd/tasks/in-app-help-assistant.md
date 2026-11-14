# In-app help assistant

Help chat, separate from the document chat, that guides the user through the
app from the user manual and helps when something fails. Branch
`feat/in-app-help-assistant`. Engram mirror: `odd/in-app-help-assistant/tasks`.

## Specs

- **S1 — Guide through the tool.** "Guides the user through the tool: how to
  import, extract text from a scanned PDF, transcribe, load the API keys, etc."
  A help chat "separate from the chat that queries the documents".
- **S2 — Help when something fails.** "Helps when something fails: says what
  happened and walks the user through the fix step by step."
- **S3 — Who pays.** "It runs on each user's own OpenRouter key, the same one
  already loaded in Settings. No server or credits of ours in between."
- **S4 — Knowledge from the manual.** "The assistant must read that same
  manual, so there are never two explanations that contradict each other:
  whatever gets fixed in the manual, the assistant learns."
  (`manual/manual-usuario/manual-usuario.md`, chapter 19 «¿Qué hago si…?» is
  the troubleshooting guide.) Owner decision (L2): the manual travels inside
  the app; per question the app picks the relevant chapters locally
  ("Capítulos relevantes": 2–4 sections, plus chapter 19 for a failure), never
  the whole manual.
- **S5 — Diagnostics, redacted, opt-in.** "To help with a failure it needs the
  app's configuration (chosen providers, missing keys) and the latest entries
  of the diagnostic logs. Before sending them to OpenRouter, strip every key
  and all content from the user's documents." Owner decision (L2): "Botón por
  pregunta": a «Incluir diagnóstico» checkbox, off by default, with «Ver qué se
  envía» showing the exact text.
- **S6 — No key.** "Without an OpenRouter key the assistant cannot run. Show a
  fixed help page instead, explaining how to load the key and that the account
  needs credit."
- **S7 — Variants.** "Must work the same in Lite and Pro."
- **S8 — How to test it.** "Use real failures found in user testing (e.g. the
  first AssemblyAI transcription failing until «Guardar cambios» is pressed,
  PDFs not opening on older macOS — see `macos-lite-1.3.0-followups.md`). If
  the assistant can't guide someone through those, it isn't useful."

- **S9 — Local index, rebuilt by hash.** From L11: "que tenga los documentos en local indexados en la sql y qeu funcione como un rag que como el manual se actualiza con cada actualizacion de la app la base verctorizada debe actualizarce cuando encuentre que el hash es distintos entre lo que tiene indexado". Owner decisions (L12): compare against what the installed app ships ("Lo que trae la app"), and index with SQLite FTS5 + trigram fuzzy search ("FTS5 + trigramas", no vectors), as asked: "se podría usar FST5 como indexador y recuperador con busqueda difusa".
- **S10 — Common questions answered locally.** "algunas preguntas basicas no necesitan viajar, pro ejemplo lo de como se importa un PDF no es necesario que viaje via api openrouter ya deberia esta indexado en lcoal, tendria que haber un set de 20 preguntas comunes o diudas comunes con respuesta detemrinsita qeu se actualicen con la actualizaciond e la app". Added (L13): "tambien endria que tener de forma detemrinista sin necesidad de api \"primeros paso\" por ejemplo ¿cómo crear mi `rimera colecciomn? como hacer OCR? como eliminar una imagen ??? ect etc" — a «Primeros pasos» group of deterministic answers alongside the common questions, both answered locally without key or network.
- **S11 — Screen map.** "se podría incorpora run mapa del repo que compelmente al manual". Owner decision (L12): "Mapa de pantallas": generated from the code, every screen, tab, button and error message with its real name; indexed with the manual.
- **S12 — Document tools, opt-in.** "que pueda usar las mismas herrameintas del MCP de EntropIA apra Agentes". Owner decision (L12): "Opt-in por pregunta": a «Buscar en mis documentos» checkbox, off by default; only with it on the assistant may use the agent MCP read tools and send fragments of the user's documents to OpenRouter. With it off, S5 stands unchanged. Owner decision (L19): "Búsqueda local inyectada (Recomendado)": with the box ticked, the app runs the Chat's lexical retrieval (FTS5, no embeddings, same in Lite and Pro) on the question and adds the top fragments with their titles to the prompt; no tool loop.
- **S13 — Floating bubble.** "quiero que a la vista el agente / asisten de ayuda sea una bubuja que se peida cliclar para desplegarse y no un boton el la cabecera, esa burbuja por defecto debería esta en el angulo inferior derechi pero debe poder moverse arrasterando con  el mausse la burbuja. la burbuja puede usar el logo vectorizado de EntropIA". Added (L23): "aparece babajo a la derecha emne l arranque minimizado pero cuando la app es mximizada queda fija en ese punto de la apntalla y no al angulo bajo derechoi" — the bubble must stay anchored to the bottom-right corner (or to wherever it was dragged, relative to that corner) when the window is resized or maximized; and "el chat es una ventana emergente y eso es correcto lo qeu falta es qeu se pueda maximizar" — the chat panel gets a maximize/restore control.
- **S14 — Settings tab.** "podría incorporarse la configuraciond e modelo contexto y otrs parametros en una nueva solapa en configuracion".

Out of scope (L2): the first-run setup assistant ("No, después"); it gets its
own card later and can reuse this base.

## Decisions

- D1 The manual is compiled into the binary (`include_str!`), not added as a
  Tauri resource: identical in every variant and platform, no installer
  overlay to keep in step, and every build carries the manual of its own
  commit.
- D2 Section choice is local and free: split by `##`/`###` headings, score by
  accent-folded word overlap with the question, top sections up to a character
  budget; chapter 19 is always added when diagnostics are attached or the
  question reads as a failure.
- D3 Diagnostics text is built in Rust (`help_diagnostics_preview`); what the
  user previews is what `help_ask` sends, re-scrubbed in Rust before leaving.
  Settings travel only from an allowlist of mode/model keys; API keys travel
  only as "configurada / no configurada". Log lines lose file paths and quoted
  text (document names and snippets live there) besides the credential
  redaction they already get. Known limit: a document title written in a log
  line without quotes, path or extension cannot be told apart from ordinary
  words; the opt-in checkbox and the exact preview are the control for it.
- D4 The conversation lives in memory for the session only; nothing is stored.
- D5 Model: the general `openrouter_model` (same fallback chain as the other
  flows); not streamed, like the RAG chat.

- D6 (supersedes D2 for retrieval) The help index lives in its own SQLite file under the cache dir (`help-index.sqlite`), never in the user's archive: no migration, no sync, rebuildable. It stores the hash of the bundled knowledge (manual + screen map + FAQ); at first use after start, a different hash drops and rebuilds the index. FTS5 with the `trigram` tokenizer for fuzzy matching, bm25 ranking.
- D7 The 20 common questions live in the repo (`manual/manual-usuario/preguntas-frecuentes.md`), drafted from the manual for the owner's review, bundled like the manual. A question that matches one above a fixed score threshold is answered locally with that text, marked «Respuesta local, sin consultar a OpenRouter», without a key and without network.
- D8 The screen map is generated by a script from the app's Spanish UI strings and committed (`manual/manual-usuario/mapa-de-pantallas.md`); a test fails when it is stale, as `version:check` does for versions.
- D9 The bubble replaces the header button and the `help` root view: one entry point. Position persisted per device (localStorage), clamped to the window; keyboard: focusable, Enter/Space opens, Escape closes.
- D10 Help settings keys: `help_model` (empty = inherit `openrouter_model`), `help_context_chars`, `help_max_tokens`, `help_temperature`, `help_history_turns`.

## Tasks

| ID | Specs | Route | Status | Commit |
|---|---|---|---|---|
| T1 | S4 | inline, TDD — Rust `help::manual`: embed, split into sections, local selection | done | bd30782d |
| T2 | S5 | inline, TDD — Rust `help::diagnostics`: allowlisted config + scrubbed recent logs, `help_diagnostics_preview` (Risk item 2) | done (builder; command lands in T3) | bd30782d |
| T3 | S1-S3, S5 | inline, TDD — Rust `help_ask`: key gate, prompt assembly, OpenRouter call | done | dbf36925 |
| T4 | S1, S2, S5-S7 | inline, TDD — frontend: `lib/help.ts`, `HelpView`, header button, no-key page, i18n ES/EN | done | 64b70c7e |
| T5 | S4 | inline — manual: short section on the help chat | done | 9e971b52 |
| T6a | S5 | fixes from the independent verify (TDD) + owed T1 mutation check | done | see L9 |
| T7 | S13 | inline, TDD — floating draggable bubble with the EntropIA logo; remove header button and `help` route | done | see git log (feat(help): open the help chat from a draggable floating bubble) |
| T8 | S9 | inline, TDD — Rust `help::index`: FTS5 trigram index in `help-index.sqlite`, hash rebuild, replaces BM25 selection | done | see git log (feat(help): index the manual …) |
| T9 | S10 | inline, TDD — «Primeros pasos» + 20 common questions file, local deterministic answers (no key, no network) | done | see git log (feat(help): answer first steps …) |
| T10 | S11 | inline, TDD — screen map generator script + committed map + staleness test, indexed | done | see git log (feat(help): generate a screen map …) |
| T11 | S14 | inline, TDD — Settings tab «Ayuda» (D10) wired into help_ask | done | see git log (feat(help): add an Ayuda settings tab …) |
| T12 | S12 | TDD — «Buscar en mis documentos» opt-in: local lexical retrieval (`rag::retrieval::hybrid_retrieve_candidates` with no embedding + `pack_sources`) injected into the prompt (Risk item 2) | done | see git log (feat(help): search the user's documents locally …) |
| T13a | S8-S14 | manual § 13.5 update + «Ayuda» row in the settings table | done | bf099db0 |
| T13b | S5, S9-S14 | independent verify of v2 (Risk item 2) | done (static only; 2 findings fixed in T13c) | — |
| T13c | S12, S14 | fix verify findings (TDD) | done | see git log (fix(help): keep document answers out …) |
| T14 | S13 | TDD — bubble anchored to the bottom-right corner across resize/maximize; maximize/restore for the chat panel | done | see git log (fix(help): anchor the bubble …) |
| T15 | S13 | panel layout: real header row with close/maximize, compact HelpView inside the panel, messages scroll, diagnostics preview bounded, composer and send always visible | pending | |
| T6 | S8 | owner check with the real failures of S8 (needs a real OpenRouter key) | pending | |

## Log

- L1 (2026-11-05, owner request, original card text): see S1–S8. "Needs
  agreement with the manual's maintainer before starting." "Overlaps with a
  first-run setup assistant (pending from earlier usability audits): design
  both together."
- L2 (2026-11-13, owner answers): manual agreement — "Sí, acordado"; context —
  "Capítulos relevantes (Recomendado)"; diagnostics — "Botón por pregunta
  (Recomendado)"; first-run assistant — "No, después (Recomendado)".
- L3 (2026-11-13, exploration): manual 1,729 lines, not bundled (only GitHub
  Pages); RAG chat calls OpenRouter from Rust, not streamed
  (`llm/openrouter.rs:434`); key gate `llm/mod.rs:1461`
  `require_openrouter_key`; logs ring buffer in `app_logs.rs`, credential
  redaction at write time (`redact_sensitive`), `logs_get` returns 20 entries;
  no help button or manual link anywhere in the app.
- L4 (2026-11-13, T1+T2): `help::manual` (parse, BM25 selection over 5-letter stems, chapter 19 first for failures, sections skipped not cut over budget) and `help::diagnostics` (allowlisted settings, keys as configurada / no configurada, last 30 warn/error lines with URL paths, file paths, file names and quoted text replaced, credential redaction via `sanitize_field`). T1 RED not observed: the compile started after the implementation was written (7/7 green on first run); a mutation check is owed in T6. T2 RED 6 failing -> GREEN `cargo test --lib help::` 14/14. Commit bd30782d.
- L5 (2026-11-13, T3, delegated worker): `help/prompt.rs` (wire types camelCase, validation, history last 6 turns cut to 4000 chars, Spanish prompt with excerpts `chapter › title`, diagnostics labelled as data) and `help/commands.rs` (`help_status` from the raw setting, `help_diagnostics_preview`, `help_ask` re-scrubbing diagnostics, key gate error `help_no_openrouter_key`, model `openrouter_model` else `DEFAULT_OPENROUTER_MODEL`, `provider_defaults(1500, 0.2)`). RED 7 failing -> GREEN `cargo test --lib help::` 21/21; clippy lean `-D warnings` clean; rustfmt clean (worker report). No live OpenRouter call tested. Commit dbf36925.
- L6 (2026-11-13, T4, delegated worker + parent fixes): `lib/help.ts`, `HelpView.svelte` (root view `help`, header button «Ayuda»), no-key page (Configuración → APIs remotas → OpenRouter, credit note, manual link), «Incluir diagnóstico» off by default and reset after each send, «Ver qué se envía» preview sent verbatim, sources under each answer, memory-only chat. Worker RED (missing modules) -> GREEN focused suites. Parent fixes: `tab-meta.ts` exhaustive switch (`help` arms + test); native checkbox replaced by `@entropia/ui` `Checkbox` (design-tokens guard); duplicate `help` icon removed, uses the existing `circle-help`. Checks: desktop Pro 306 files / 4283 passed, Lite 4247 passed; ui 963 passed; svelte-check Pro and Lite 0/0; lint clean; prettier clean. Commit 64b70c7e.
- L7 (2026-11-13, T5): manual § 13.5 «Ayuda dentro de la app» (key, sections only, opt-in diagnostics and what it holds, nothing stored, explains only). `build_html.py` builds. Commit 9e971b52.
- L8 (2026-11-13, T6 independent verify, gentle-ai-verify): isolated setup failed once (pnpm junction symlinks in the scratch copy), not retried; static findings on S5: Windows path whose last segment has spaces leaked words; `file:///Users/...` kept the path; curly single quotes and apostrophes inside titles survived; a markerless 32-hex key survived `sanitize_field`; a long report could exceed MAX_REPORT_CHARS so the sent text differed from the preview. Unquoted titles: not detectable (see D3).
- L9 (2026-11-13, T6a): RED 2 new tests failing (adversarial leaks; longest report previewed == sent) -> GREEN `cargo test --lib help::` 23/23; clippy lean `-D warnings` clean. Fix: extension-aware final segment for Windows/UNC/Unix paths and any `scheme://` URL (scheme and host kept), curly single quotes, greedy single quotes, credential-shaped runs of 32+ chars with letters and digits -> `[redactado]`, log messages cut to 300 chars before scrubbing. T1 mutation check: ignoring the budget fails `a_section_over_the_budget_is_skipped_not_cut`; keeping images fails `parse_keeps_chapters_...`; restored, 23/23.
- L10 (2026-11-13, owner check, first run): user: "no me habilita porque dice que no tengo api ingresada". Cause: `help_status`, `help_diagnostics_preview` and `help_ask` were registered in `lib.rs` but missing from the app ACL (`build.rs` command list and `capabilities/default.json`), so every call was denied; `HelpView` read any failed status as a missing key. Fix: commands added to both; a failed status now leaves the chat usable (help_ask reports the real cause). RED 1 failing -> GREEN HelpView 16/16; `cargo test --test acl_manifest_guard --test app_acl` 7/7 + 15/15. Lesson: a new Tauri command needs the `build.rs` list and the capability, not only `invoke_handler`; unit tests with a mocked invoke cannot see it.
- L11 (2026-11-13, owner check S8, second run, verbatim): "funciona peor tiene mucho margen de mejora, voy apuntando lo que quiero: 1) que pueda usar las mismas herrameintas del MCP de EntropIA apra Agentes, que tenga los documentos en local indexados en la sql y qeu funcione como un rag que como el manual se actualiza con cada actualizacion de la app la base verctorizada debe actualizarce cuando encuentre que el hash es distintos entre lo que tiene indexado y lo que está en al web, adema´s qeu que se podría incorpora run mapa del repo que compelmente al manual, tambien creo qeu algunas preguntas basicas no necesitan viajar, pro ejemplo lo de como se importa un PDF no es necesario que viaje via api openrouter ya deberia esta indexado en lcoal, tendria que haber un set de 20 preguntas comunes o diudas comunes con respuesta detemrinsita qeu se actualicen con la actualizaciond e la app, por otra aprte se podría usar FST5 como indexador y recuperador con busqueda difusa, fiuanlemnte quiero que a la vista el agente / asisten de ayuda sea una bubuja que se peida cliclar para desplegarse y no un boton el la cabecera, esa burbuja por defecto debería esta en el angulo inferior derechi pero debe poder moverse arrasterando con  el mausse la burbuja. la burbuja puede usar el logo vectorizado de EntropIA, Finalmente podría incorporarse la configuraciond e modelo contexto y otrs parametros en una nueva solapa en configuracion". Specs pending the owner's answers to the open decisions (document access vs S5, refresh source, vectors vs FTS5, what the repo map is).
- L12 (2026-11-13, owner answers): documents — "Opt-in por pregunta (Recomendado)"; refresh — "Lo que trae la app (Recomendado)"; retrieval — "FTS5 + trigramas (Recomendado)"; map — "Mapa de pantallas (Recomendado)". Bundled SQLite already ships FTS5 (rusqlite 0.41 bundled); `OpenRouterClient` has no tool calling yet (T12 is the largest unit).
- L13 (2026-11-13, owner, verbatim): "tambien endria que tener de forma detemrinista sin necesidad de api \"primeros paso\" por ejemplo ¿cómo crear mi `rimera colecciomn? como hacer OCR? como eliminar una imagen ??? ect etc". S10 and T9 updated.
- L14 (2026-11-13, T8, delegated worker): `help/index.rs` (`<cache_dir>/help-index.sqlite`, `meta.content_hash` SHA-256, rebuild in one transaction on mismatch, `docs` + `docs_fts` trigram, bm25 title 3.0 / body 1.0, accents folded in Rust in `help/text.rs`), `pick_sections` now on the index, BM25 code removed. RED 8 failing -> GREEN `cargo test --lib help::` 28/28; clippy clean; rustfmt clean. Gap carried to T9: a single misspelled word ("impotar") matches nothing; trigram fallback needed.
- L15 (2026-11-13, T9, delegated worker): `preguntas-frecuentes.md` (10 «Primeros pasos» + 20 «Preguntas frecuentes», answers from the manual, every `Fuente:` checked against `manual::sections()` by test), `help/faq.rs` indexed as `Source::Faq`, trigram fallback for single misspellings, local answer before the key gate when coverage >= 0.60 and margin >= 0.15 over the second entry (never with diagnostics), `help_faq_list` command (lib.rs + build.rs + capability), badge «Respuesta local, sin consultar a OpenRouter», no-key page keeps starters and input. RED 11 Rust + 9 frontend failing -> GREEN `cargo test --lib help::` 42/42, HelpView+help+i18n 47/47 Pro and Lite, svelte-check 0/0 both, lint and prettier clean, ACL tests 7/7 + 15/15. Owner review owed: the 30 answers, especially the AssemblyAI 401 entry ("401" is not in the manual).
- L16 (2026-11-13, T10, delegated worker): `apps/desktop/scripts/build-help-screen-map.mjs` (ES block of i18n.ts + Views + settings tabs; `pnpm help:screen-map`, `--check`), `mapa-de-pantallas.md` 154 KB / 37 areas / 154 sections, staleness test `src/lib/help-screen-map.test.ts` (desktop vitest only includes `src/**`), `help/screenmap.rs` indexed as `Source::ScreenMap`; crowding rule: at most 1 map hit, after manual hits; quoted adjacent-word phrases in the FTS query. RED 5 frontend + 5 Rust failing -> GREEN `cargo test --lib help::` 51/51, map test 5/5, `--check` clean, clippy, rustfmt, lint, prettier clean.
- L17 (2026-11-13, T11, delegated worker): `help_model`, `help_context_chars` (4000-60000, default 24000), `help_max_tokens` (200-8000, 1500), `help_temperature` (0-1, 0.2), `help_history_turns` (0-12, 6), `help_local_answers` (default on); `help/settings.rs` resolver; tab «Ayuda» after «Asistentes», saved with the global «Guardar cambios»; keys added to the diagnostics allowlist; screen map regenerated. RED compile errors + missing component -> GREEN `cargo test --lib help::` 62/62, focused desktop 135 Pro / 125 Lite, svelte-check 0/0 both, lint, prettier, map --check clean.
- L18 (2026-11-13, T7, delegated worker + parent fix): `HelpBubble.svelte` (bottom-right default, pointer drag with 4 px click threshold, position in localStorage clamped and re-clamped on resize, Enter/Space opens, Escape closes and refocuses, panel opens inward, lazy-mounts HelpView), mounted once in AppShell through the overlay portal; header button and `help` route removed. No vector logo exists in the repo (only PNG/ICO/ICNS): uses `assets/entropia-mark.png` as a white mask; owner to supply an SVG if wanted. Parent fix for T11: the «Ayuda» settings fields showed raw names («contextChars (4000-60000)»), had no `number-field-label` (guard test failed) and `i18n.test.ts` was left out of the T11 commit; now translated labels `settings.help.label.*`, tests updated. Checks: desktop full Pro 4340 / Lite 4304 passed, svelte-check 0/0 both, lint clean, prettier clean, map --check clean.
- L19 (2026-11-13, T12 exploration + owner decision): agent MCP read tools are `rmcp` handlers that need a request context, audit every call and read through `ArchiveReader` (`automation/mcp.rs:797,831,870`, `read.rs:482,613,789`); `OpenRouterClient` has no `tools`/`tool_calls` (`llm/openrouter.rs:72-89,180-201`); archive-wide lexical retrieval exists in `rag/retrieval.rs` (`hybrid_retrieve_candidates` with `embedding: None`, `RetrievalUnit::Asset`; `pack_sources`). Owner chose "Búsqueda local inyectada (Recomendado)" over a tool loop.
- L20 (2026-11-13, T12, delegated worker): `help/documents.rs` (`hybrid_retrieve_candidates` with `embedding: None`, `RetrievalUnit::Asset`, `pack_sources`, `rag_params_from_settings`; up to 6 fragments / 8000 chars), `help_ask(..., documents)` reads the archive only when true and after the key gate, never a local FAQ answer with documents on, `HelpAnswer.documents` titles; «Buscar en mis documentos» checkbox off by default and reset after send, «De tus documentos: …» under answers. RED 3 behaviour + 7 compile + 5 frontend failing -> GREEN `cargo test --lib help::` 71/71, desktop full Pro 4347 / Lite 4311, svelte-check 0/0 both, clippy, rustfmt, lint, prettier, map --check clean.
- L21 (2026-11-13, T13b verify, gentle-ai-verify): isolated setup refused by the safety policy (".env" sensitive path), zero runtime probes by the verifier; static verdict NOT CLEARED: S12 FAIL (an answer built from documents stayed in `messages` and was resent as history on a later unchecked question, `HelpView.svelte:107-126`, `prompt.rs:225-232`); S14 FAIL (`HelpView.svelte:108` and `help.ts:61` sliced history to 6 before Rust applied `help_history_turns` up to 12). Spot-check of 5 local answers: no invented UI names. Parent runtime checks: `cargo test --lib help::` 71/71, ACL 7/7 + 15/15, help vitest 75/75 Pro and Lite.
- L22 (2026-11-13, T13c): document-derived answers are filtered out of history; frontend sends up to `HELP_HISTORY_MAX_TURNS` = 12 and Rust trims to the setting. RED 3 failing (verifier's S12 probe, 12-turn cap in HelpView and help.ts) -> GREEN 56/56 Pro and Lite; full desktop Pro 4348 / Lite 4312, svelte-check 0/0 both, lint, prettier clean.
- L23 (2026-11-13, owner check v2, verbatim): "varias cosas, priemro, aparece babajo a la derecha emne l arranque minimizado pero cuando la app es mximizada queda fija en ese punto de la apntalla y no al angulo bajo derechoi, en segundo lugar, el chat es una ventana emergente y eso es correcto lo qeu falta es qeu se pueda maximizar". Cause of the first: the position is stored as absolute left/top. S13 extended, T14 added.
- L24 (2026-11-13, T14, delegated worker): position stored as `{right, bottom}` offsets (legacy `{x, y}` converted with the current window size and rewritten), re-clamped on resize; panel maximize/restore («Maximizar» / «Restaurar», icons `expand` / new `restore`), maximized state in localStorage, HelpView not remounted. RED 31 unit + 24 component failing -> GREEN 62/62; full desktop Pro 4373 / Lite 4337, ui 963, typecheck 0/0 (Pro, Lite, ui), lint, prettier, map --check clean.
- L25 (2026-11-13, owner check v2, verbatim): "no hay ni cerrar ni maximizar, ademáscuando hago clic en ver lo qeu se envia se solapa y se corta informacion y botone" (screenshot: panel shows the HelpView «Ayuda» header card at the top, no close/maximize; the messages area collapsed to a cut line; the diagnostics preview pushes the composer down and the send button is cut at the bottom). Cause: `.help-bubble__actions` is absolutely positioned over HelpView's own header card; `.help-bubble__body` has `overflow: hidden` and HelpView's full-page layout does not fit the 560 px panel. T15 added.
