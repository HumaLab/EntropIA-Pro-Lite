# Pi model fallbacks

## Goal

Configure ordered, automatic transient-error model fallbacks for the Judgment Day agents and the Gentle Pi orchestrator, using Pi's canonical provider/model identifiers and avoiding API billing for Codex subscription calls.

## Scope

- Project-local Pi extension under `.pi/extensions/`.
- Project-local subagent model profiles under `.pi/subagents.json`.
- The existing Judgment Day chains remain limited to Gentle Agents child processes.
- The orchestrator receives one exact parent-only chain: `openai-codex/gpt-5.6-luna` -> `xai/grok-4.7` -> `zai/glm-5.3`.
- No global settings, credentials, or provider calls are changed or required for verification.
- `xai/grok-4.7` is not in the local Pi catalog yet; the parent extension must use a fallback-only compatibility alias derived from the known `xai/grok-4.6` metadata and must not replace the built-in xAI catalog.

## Resolved provider mappings

- Judge A: `openai-codex/gpt-5.6-sol` -> `opencode/muse-spark-1.3` -> `xai/grok-4.6`.
- Judge B: `zai/glm-5.3` -> `opencode-go/glm-5.3` -> `opencode-go/deepseek-v4-flash-vision-exp`.
- Fix agent: `opencode/claude-fable-5-1` -> `openai-codex/gpt-6-astra`.

Pi's current catalog does not expose `opencode-go/muse-spark-1.3` or `deepseek/deepseek-v4-flash-vision-exp`; the mappings above preserve the requested model order using the available Pi provider entries. Pi's Codex subscription provider is `openai-codex`, not `openai`.

## Tasks

1. Implement a project-local Pi extension that wraps only the configured primary provider models, buffers failed attempts, and advances only on transient failures.
2. Configure the requested Judgment Day primary models in `.pi/subagents.json`.
3. Add the parent/orchestrator chain without changing the child-only Judgment Day routes; skip or resolve unavailable candidates safely.
4. Verify TypeScript/syntax, JSON, model catalog resolution, and extension discovery without making paid model calls.

## Evidence

- Task 1 (tracking): completed before source writes; the task artifact and Engram mirror were created.
- Task 2 (implementation): completed by the delegated worker; extension written under `.pi/extensions/`.
- Task 3 (configuration): completed by the delegated worker; exactly three project model profiles written.
- Task 4 (baseline verification): completed. Parent read-only checks passed for the existing child configuration: JSON parse, extension parse/default factory, catalog resolution for the existing mapped models, child RPC startup without a prompt, and `git diff --check`.
- The configured `gentle-ai-verify` child failed before running commands, so the parent performed the equivalent read-only checks and recorded that fallback.
- Commit: not created; user did not request a commit.
- Follow-up investigation: Oh My Pi's native `retry.fallbackChains` resolves ordered candidates, skips unavailable models, and only switches on eligible provider failures; Pi's current runtime has no equivalent native chain setting, so the project extension remains the implementation seam.
- Local catalog evidence: `pi --list-models` shows `xai/grok-4.6` and `zai/glm-5.3`, but no `xai/grok-4.7`. Official xAI documentation names the API model `grok-4.7`; the extension will therefore treat it as a fallback-only alias until Pi's catalog catches up.
- Parent chain implementation: completed in `.pi/extensions/judgment-day-model-fallback.ts`; non-child sessions install only the `openai-codex/gpt-5.6-luna` wrapper, while child sessions retain the three existing child-only wrappers.
- Parent fallback behavior: verified with a no-network Bun mock. A transient primary error advanced to the compatibility-resolved `xai/grok-4.7` candidate and emitted the winning model identity; the child scope registered exactly `openai-codex`, `zai`, and `opencode`.
- Catalog checks: `openai-codex/gpt-5.6-luna` and `zai/glm-5.3` resolve locally; `xai/grok-4.7` is still absent, so the extension clones only `xai/grok-4.6` dispatch metadata when resolving that fallback and leaves the catalog unchanged.
- Syntax/startup checks: Bun imported the extension in both scopes, both offline Pi RPC startup checks exited 0 with no stderr, a focused `tsc` run passed with a temporary local declaration shim for Pi's type-only import, Prettier and `git diff --check` passed, and no provider request was made.
- A direct `tsc` run without the shim reports only the expected missing local `@earendil-works/pi-coding-agent` declaration; the project relies on the globally installed Pi runtime at execution time.
