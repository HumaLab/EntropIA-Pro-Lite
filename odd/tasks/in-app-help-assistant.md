# In-app help assistant

**Status:** pending, not started. Requested by the owner 2026-11-05. Needs
agreement with the manual's maintainer before starting.

## What it is

A help chat, separate from the chat that queries the documents, that does two
things:

1. **Guides the user through the tool**: how to import, extract text from a
   scanned PDF, transcribe, load the API keys, etc.
2. **Helps when something fails**: says what happened and walks the user
   through the fix step by step.

## Who pays

It runs on **each user's own OpenRouter key**, the same one already loaded in
Settings. No server or credits of ours in between.

## Where its knowledge comes from

The user manual already exists and is complete:
`manual/manual-usuario/manual-usuario.md` (21 chapters, updated 2026-10-21),
published at
https://humalab.github.io/EntropIA-Pro-Lite/manual-usuario/manual-usuario.html.
Chapter 19, «¿Qué hago si…?», is already a troubleshooting guide.

**The assistant must read that same manual**, so there are never two
explanations that contradict each other: whatever gets fixed in the manual,
the assistant learns.

## To resolve in the design

- To help with a failure it needs the app's configuration (chosen providers,
  missing keys) and the latest entries of the diagnostic logs. **Before
  sending them to OpenRouter, strip every key and all content from the
  user's documents.**
- Without an OpenRouter key the assistant cannot run. Show a fixed help page
  instead, explaining how to load the key and that the account needs credit.
- Must work the same in Lite and Pro.
- Overlaps with a first-run setup assistant (pending from earlier usability
  audits): design both together.
- Touches the manual, so agree on it before starting.

## How to test it

Use real failures found in user testing (e.g. the first AssemblyAI
transcription failing until «Guardar cambios» is pressed, PDFs not opening on
older macOS — see `macos-lite-1.3.0-followups.md`). If the assistant can't
guide someone through those, it isn't useful.
