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
  redaction they already get.
- D4 The conversation lives in memory for the session only; nothing is stored.
- D5 Model: the general `openrouter_model` (same fallback chain as the other
  flows); not streamed, like the RAG chat.

## Tasks

| ID | Specs | Route | Status | Commit |
|---|---|---|---|---|
| T1 | S4 | inline, TDD — Rust `help::manual`: embed, split into sections, local selection | pending | |
| T2 | S5 | inline, TDD — Rust `help::diagnostics`: allowlisted config + scrubbed recent logs, `help_diagnostics_preview` (Risk item 2) | pending | |
| T3 | S1-S3, S5 | inline, TDD — Rust `help_ask`: key gate, prompt assembly, OpenRouter call | pending | |
| T4 | S1, S2, S5-S7 | inline, TDD — frontend: `lib/help.ts`, `HelpView`, header button, no-key page, i18n ES/EN | pending | |
| T5 | S4 | inline — manual: short section on the help chat | pending | |
| T6 | S5, S8 | independent verify (Risk item 2), then owner check with the real failures of S8 | pending | |

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
