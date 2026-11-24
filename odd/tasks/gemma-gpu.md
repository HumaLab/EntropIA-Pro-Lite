# Gemma on the GPU

Run the local Gemma text model on the GPU through the managed `llama-server`
that GLM-OCR already uses, keeping today's in-process CPU engine as the
fallback. Branch `feat/gemma-gpu`. Strategy: ask-on-risk. Runner: cargo test
(lean + `--features local-ml`) and Vitest.

## Specs

- **S1 Gemma uses the GPU like GLM-OCR.** "consulta, el modelo gemma no puede correr en GPU como hace GLM-OCR???" → "Gemma en GPU". The interactive local flows (RAG chat answer, intent, query rewrite, help chat) generate on the GPU through the same managed `llama-server` build GLM-OCR uses (CUDA / HIP / Vulkan / Metal, chosen by `ocr::llama_gpu`).
- **S2 CPU stays the fallback, never an error.** Gemma generates in-process on CPU exactly as today when any of these holds: no qualifying GPU, the managed `llama-server` is not installed, the server fails to start or answer, the request does not fit the server's context window, or the GPU server is busy with OCR. The user still gets an answer.
- **S3 OCR is never interrupted by the chat.** While the GPU server runs an OCR or correction job, a chat request does not stop it (it uses CPU, S2). An idle Gemma server is stopped when OCR needs the GPU, as today's one-model rule does; a Gemma request in flight is not killed.
- **S4 Same answers.** Prompt text, temperature, seed, token budget and output sanitizing stay the same as the CPU path (raw `gemma_wrap` prompt through `/completion`, not a second chat template).
- **S5 Visible backend.** The app log records which backend served each local generation (`gpu:<backend>` or `cpu`) and why a GPU request fell back.
- Assumption (not asked): background jobs (NER, summaries, triples, OCR correction through Gemma) stay on CPU in this feature; they can follow once S1-S5 are proven.
- Assumption (not asked): the server's `llama-server` download is not triggered by Gemma; Gemma uses the GPU only once the build is installed (any OCR model download installs it).

## Tasks

| ID | Specs | Route | Status | Commit |
|---|---|---|---|---|
| T1 | S1, S2 | inline — `local_server`: text-only assets (optional mmproj), `ServerKind::Text`, Gemma config (slots 1, ctx from measurement); no behavior change for OCR | done | feature commit |
| T2 | S4 | inline — Gemma server client: `/completion` body (prompt, n_predict, temperature/seed, cache_prompt), response parse, `/tokenize`; pure request/response functions with tests | done | feature commit |
| T3 | S1, S2, S5 | inline — route `generate_chat_with_ctx` callers: GPU when installed + qualifying + fits, else CPU; log backend and fallback reason | done | feature commit |
| T4 | S3 | inline — server ownership: chat never stops a busy OCR server; OCR stops an idle Gemma server; in-flight Gemma request not killed. Risk item 4 (concurrency) → independent verifier | done, independent verifier run (L6) | feature commit (shares `local_server.rs` with T1, so not split) |
| T5 | S1-S5 | parent — measure in the dev app (same question as the CPU baseline), docs, pendientes | done (L7) | feature commit |

## Log

- **L1** (user, verbatim, 2026-11-24): "consulta, el modelo gemma no puede correr en GPU como hace GLM-OCR???" → plan offered (route Gemma through the same `llama-server`) → "si" → after the benchmark, "Gemma en GPU".
- **L2** Evidence (map, spot-checked): one global `SERVER` mutex holds a single `llama-server` and restarts it when kind/slots/ctx/assets differ (`src/ocr/local_server.rs:220-258`); the launcher always passes `--mmproj` (`:330-339`); `ServerKind` is `Ocrh | Correction` (`:33-36`); `llama_gpu::choose` has no CPU backend and gates AMD/Intel/Metal at 3.5 GiB (`src/ocr/llama_gpu.rs:23,156-189`); Gemma is one `Arc<Mutex<LlmEngine>>`, `n_gpu_layers 0` (`src/llm/mod.rs:331-350`); callers of `generate_chat_with_ctx`: `rag/commands.rs` (answer, direct), `rag/intent.rs:173`, `rag/query_rewrite.rs:82`, `help/commands.rs:92`; the Gemma 4 Jinja template fails in llama-cpp-2 and callers use `gemma_wrap` (`src/llm/engine.rs:495-507`).
- **L3** Benchmark, this machine (RX 560 4 GB, 6C/12T), managed Vulkan `llama-server` b8783, 12,575-token prompt, 64 output: GPU `-ngl 99` 58 prefill tok/s, 6.5 gen tok/s, 223 s; CPU `-t 6` 31.1 / 8.1 / 409 s; CPU `-t 4` 31.2 / 6.7 / 409 s. VRAM fully offloaded: 1.4 GB weights on GPU (1.75 GB per-layer embeddings CPU-mapped), KV 108 MB at 16K, compute 545 MB. Manual launches need `env -u LLAMA_API_KEY` (the app scrubs it).
- **L4** Forecast: ~600-800 authored changed lines; T4 carries Risk item 4 and gets its own commit and an independent verifier.
- **L5** T3/T4 design: `generate_local_chat` tries `gemma_gpu::generate` and loads the CPU engine only on fallback, logging `Gemma en GPU (<backend>)` or `Gemma en CPU (<reason>)` with the elapsed time. The GPU window is 32K; a longer prompt (`/tokenize`) goes to CPU. A Gemma request holds a `Lease`; any server stop that is not app exit waits (holding `SERVER`, max 900 s) until no lease is held, and a lease never takes `SERVER`, so it cannot deadlock. The chat takes the server only when it is free or already Gemma's (`may_take_over`).
- **L6** Independent verifier (read-only, code reading; its test run was cut by the MNN build): S2 PASS, S3 PASS, S4 PARTIAL, S5 PASS, S1 FAIL.
  - **B1:** an idle OCR/correction server is never stopped, so after one OCR page the chat stayed on CPU all session.
    - Fix: OCR callers now hold a `Lease` (`OCR_IN_FLIGHT`) for each page or correction.
    - The chat takes a non-Gemma server only when no OCR lease is held and none ended within `OCR_IDLE_GRACE` (60 s), so it never slips in between two pages of a job.
    - Nothing waits on OCR leases, so an OCR caller asking again cannot block itself.
  - **B2:** app exit could wait up to 900 s behind a restart that was waiting for a chat request.
    - Fix: `shutdown` wakes every waiter (`abort_waits`) until it gets the lock.
    - A restart waits for chat leases only when it stops a Gemma server.
  - **A3 fixed:** the GPU uses the caller's `n_ctx` (`request_window`), so budgets and too-long prompts behave as on CPU.
  - **A5 fixed:** a failed server start keeps the session on CPU (`StartFailedBefore`).
  - **A6 fixed:** the log reads `Gemma gpu:<backend>` / `Gemma cpu: <reason>`, written before the CPU run.
  - **Accepted:** A1 (an engine switch or a build replace kills a chat request in flight, which falls back to CPU).
  - **Accepted:** A2 (the CPU path tries the Jinja template first; today it always fails to `gemma_wrap`).
  - **Accepted:** A4 and A7 (edge cases with no current trigger).
- **L7** Measured in the dev app (profile `pro-audit`, RX 560 Vulkan), question "¿Qué sindicatos aparecen en las cartas de 1965?":
  - The log reads `[rag][intent] Gemma gpu:Vulkan: 17.4 s` (server start included) and `[rag][local] Gemma gpu:Vulkan: 196.9 s`. The CPU baseline answer took ~333 s.
  - Checks: 593 focused Pro tests, 253 Lite OCR tests, clippy Pro and Lite clean.
  - At `rag_temperature 0.2` the GPU answer is not byte-identical to the CPU one (same sampler chain, different RNG and GPU numerics). This run repeated one list line; to watch.
