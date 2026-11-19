# In-progress state survives navigation

Objective: a button whose process is still running keeps showing it after the user leaves the screen and comes back. Branch: main. Delivery: ask-on-risk. Runner: Vitest (`pnpm --filter @entropia-pro/desktop test`).

## Specs

- S1 (user, verbatim): "no siempre los procesos en curso se muestran en curso, por ejemplo los botones, todos, deberín quedar en proceso hasta que terminen independientemente de que el usuario d emueva d eun lugar a otro"
- S2: A shared, app-lifetime registry (`lib/inflight.svelte.ts`) records running processes by key. It survives component unmount, clears an entry when its awaited promise settles, and clears event-driven entries on the backend completion or error event (`nlp:*`, `llm:*`, `transcription:*`, model download events).
- S3: ItemView per-asset jobs (embeddings, NER, FTS, triples, summary, OCR correction, transcription) show as pending again after unmount and remount while the job is still running, and stop showing it once the completion/error event arrives. OCR keeps its existing queue re-read.
- S4: Settings model downloads (LLM, embedding, reranker, GLM-OCR, Qwen correction, llama-server, Parakeet) show as downloading after remount until their complete/error event.
- S5: Awaited long actions keep their busy state across remount: backup, dependency install and EntropIA preparation, assistant connect, collection import/export, full resync, bibliography annotation sync, Home sample load (narrowed in L3: the others are short or already read a surviving state).
- S6: Each affected view gets a test that unmounts, remounts and still sees the in-progress state, and sees it clear after completion.
- Assumption: the registry lives in frontend memory; a full app reload clears it (the backend queues are in memory too, except OCR/embedding, which keep the queue re-read).

## Tasks

- T1 S2: registry `lib/inflight.svelte.ts` + `inflight.test.ts` (7 tests); AppShell starts the event tracking; `test-setup.ts` resets it after each test. Route: inline. Done.
- T2 S3, S6: ItemView marks embeddings, NER, FTS, triples, summary, OCR correction and transcription, and restores them on mount and on each asset load (`restoreInflightJobs`); `LlmStore.restoreActiveJob`. Tests: TRIPLET and EMBED remount, TRIPLET ended while away. Mutation-checked. Route: inline. Done.
- T3 S4, S6: Settings download flags start from the registry (`mirrorInflight`), downloads register their ending events (`registerModelDownloads`); GLM-OCR/Qwen end on load once both files are on disk. Test: reranker remount. Mutation-checked. Route: inline. Done.
- T4 S5, S6: backup, Dependencias (install all, preparation, per-row model downloads), assistant connect, collection import/export, full resync, Zotero annotation sync, Home sample load. One remount test each (export has none). Route: inline. Done.
- T5: checks, audit md, Engram. Done: tests Pro 4637 passed; Lite 4590 passed plus one timeout in `binary-provenance.test.ts` (5.1 s under the full suite, 17/17 when run alone); lint, typecheck Pro/Lite and prettier on the changed files are green.

## Log

- L1 (2026-11-18, user): "arreglá el punto 17"
- L2: mapping found that only OCR and embeddings run through the durable queue; NER, triples, summary, correct_ocr and transcription use in-memory queues with no status query, so the state must be tracked in the frontend. Writers stay inline: the units depend on T1.
- L3: left out on purpose, with the reason. Batch tab retry and retry-all are short enqueue calls. Investigation: answering a round is a short request; the run's progress comes from the backend detail. Writing sync already reads the global `syncStore` state. Report and Writing downloads wait on the save dialog. Internet Archive already reattaches through `ia_list_jobs`. OCR keeps its queue re-read.
- L4: limits. A full app reload clears the registry. A queued job whose backend never emits a completion or error event stays in progress until the app restarts, as it already did while the view stayed open. After a remount the progress percentage starts at 0 until the next progress event.
- L5: `test-setup.ts` must load the registry lazily: a static import there bound it to the setup file's `listen` mock, before a test file installs its own, so the registry never heard the events in those files.
