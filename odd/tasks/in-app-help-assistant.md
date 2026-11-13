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

## Tasks

| ID | Specs | Route | Status | Commit |
|---|---|---|---|---|
| T1 | S4 | inline, TDD — Rust `help::manual`: embed, split into sections, local selection | done | bd30782d |
| T2 | S5 | inline, TDD — Rust `help::diagnostics`: allowlisted config + scrubbed recent logs, `help_diagnostics_preview` (Risk item 2) | done (builder; command lands in T3) | bd30782d |
| T3 | S1-S3, S5 | inline, TDD — Rust `help_ask`: key gate, prompt assembly, OpenRouter call | done | dbf36925 |
| T4 | S1, S2, S5-S7 | inline, TDD — frontend: `lib/help.ts`, `HelpView`, header button, no-key page, i18n ES/EN | done | 64b70c7e |
| T5 | S4 | inline — manual: short section on the help chat | done | 9e971b52 |
| T6a | S5 | fixes from the independent verify (TDD) + owed T1 mutation check | done | see L9 |
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
