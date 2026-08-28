# Chat Response Copy Citations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Include a title-only `Fuentes:` block in copied assistant responses without changing persisted messages or visible chat rendering.

**Architecture:** Keep clipboard formatting local to `RagChatView.svelte`, next to the existing `copyResponse` handler. A small plain-text formatter will combine the unchanged assistant content with ordered `[index] itemTitle` lines, while the existing async clipboard call, feedback, and stale-context guards remain untouched.

**Tech Stack:** Svelte 5, TypeScript, Vitest, Testing Library for Svelte, pnpm/Turbo monorepo.

## Global Constraints

- For a nonempty assistant `sources` array, clipboard text is `<original content>\n\nFuentes:\n[index] itemTitle` with one source per line.
- Preserve the original `message.content` exactly; do not trim, normalize, or inject citations into persisted content.
- Preserve the existing `message.sources` array order and each source-provided `index`; do not renumber or sort.
- Copy only `itemTitle`; exclude snippet, collection name, timestamps, scores, OCR text, provenance, and all other source metadata.
- Source titles remain literal plain text in the clipboard payload; do not HTML-escape or copy rendered DOM markup.
- With no sources, clipboard payload remains exactly `message.content`; user messages and all non-copy behavior remain unchanged.
- Do not add dependencies or modify persistence, Rust/Tauri contracts, PDF export, visible source rendering, localization, or feedback lifecycle.
- Follow strict TDD: the new/updated regression must fail for the current implementation before production code changes.

---

### Task 1: Add citations to copied assistant responses

**Files:**
- Modify: `apps/desktop/src/views/RagChatView.svelte:120-146` — add the plain-text formatter and use it in `copyResponse`.
- Test: `apps/desktop/src/views/RagChatView.test.ts:262-282` — replace the outdated raw-content expectation with the approved citation payload and add metadata/order assertions.
- Create: none.

**Interfaces:**
- Consumes: `UiMessage.sources?: RagSource[]` from `apps/desktop/src/lib/rag-chat.ts`; `RagSource.index` and `RagSource.itemTitle` from `apps/desktop/src/lib/rag.ts`.
- Produces: an internal `copiedResponseText(message: UiMessage): string` helper used only by `copyResponse`.

- [ ] **Step 1: Write the failing regression test**

Update the existing test named `copies only assistant content and shows transient feedback` so it verifies the approved contract instead of the old raw-content behavior. Keep the existing `answerWithSources` response and extend the answer returned by the test with a second source without changing the shared fixture:

```ts
const answerWithMultipleSources: RagAnswer = {
  ...answerWithSources,
  sources: [
    answerWithSources.sources[0]!,
    {
      ...answerWithSources.sources[0]!,
      index: 2,
      assetId: 'asset-2',
      itemId: 'item-2',
      itemTitle: 'Acta & <anexo>',
      snippet: 'metadata-sentinel-snippet',
      collectionName: 'metadata-sentinel-collection',
      startSeconds: 12,
      endSeconds: 15,
      provenance: {
        retrievalUnit: 'chunk',
        sourceKind: 'ocr',
        sourceId: 'metadata-sentinel-source',
        chunkIds: ['metadata-sentinel-chunk'],
        startChar: 10,
        endChar: 20,
      },
    },
  ],
}
```

Use that answer in `setupBackend({ ask: () => answerWithMultipleSources })`, click the assistant copy button, and assert the exact payload:

```ts
expect(writeText).toHaveBeenCalledWith(
  'La huelga comenzó en junio de 1966 [1].\n\nFuentes:\n[1] Entrevista 12\n[2] Acta & <anexo>',
)
expect(writeText.mock.calls[0]?.[0]).not.toContain('metadata-sentinel-snippet')
expect(writeText.mock.calls[0]?.[0]).not.toContain('metadata-sentinel-collection')
expect(writeText.mock.calls[0]?.[0]).not.toContain('metadata-sentinel-source')
```

Retain the existing assertions for the copy button, transient `Copiado` feedback, and visible answer. The expected string must fail against the current implementation because the current handler sends only `message.content`.

Add a separate test named `copies assistant content unchanged when no sources exist` with an active conversation whose assistant message has an empty `sources` array:

```ts
const noSourceConversation: RagConversation = {
  id: 'conv-no-sources',
  title: 'Salarios del SOIP',
  messages: [
    {
      id: 'msg-no-source-user',
      role: 'user',
      content: '¿Cuánto ganaban en el SOIP?',
      sources: [],
      createdAt: 1600000000000,
    },
    {
      id: 'msg-no-source-answer',
      role: 'assistant',
      content: 'El jornal rondaba los 200 pesos.',
      sources: [],
      createdAt: 1600000001000,
    },
  ],
}
setupBackend({
  storedActiveId: 'conv-no-sources',
  conversations: { 'conv-no-sources': noSourceConversation },
})
```

After rendering and clicking the assistant copy button, assert:

```ts
expect(writeText).toHaveBeenCalledWith('El jornal rondaba los 200 pesos.')
```

This dedicated assertion proves that an assistant message with no sources still copies exactly its original content without coupling the check to the conversation-switch feedback test.

- [ ] **Step 2: Run the focused test and verify RED**

Run from the worktree root:

```bash
pnpm --filter @entropia-pro/desktop test -- src/views/RagChatView.test.ts
```

Expected result before implementation: the focused test file fails only at the clipboard payload assertion because it receives `La huelga comenzó en junio de 1966 [1].` instead of the expected `Fuentes:` block. Existing unrelated tests may still pass.

- [ ] **Step 3: Implement the minimal formatter and wire it into copying**


```ts
function copiedResponseText(message: UiMessage): string {
  if (message.role !== 'assistant' || !message.sources?.length) return message.content

  const sourceLines = message.sources
    .map((source) => `[${source.index}] ${source.itemTitle}`)
    .join('\n')
  return `${message.content}\n\nFuentes:\n${sourceLines}`
}
```

Change only the clipboard payload expression in the existing `try` block:

```ts
await navigator.clipboard.writeText(copiedResponseText(message))
```

Do not alter the generation/context checks, feedback timers, button markup, or source rendering.

- [ ] **Step 4: Run the focused test and verify GREEN**

Run:

```bash
pnpm --filter @entropia-pro/desktop test -- src/views/RagChatView.test.ts
```

Expected result: `1` test file passes, including the source order, source index, literal special characters, metadata exclusion, dedicated no-source copy assertion, visible answer, and feedback assertions.

- [ ] **Step 5: Run required verification**

Run each command from the worktree root:

```bash
pnpm --filter @entropia-pro/desktop test -- src/views/RagChatView.test.ts src/lib/rag-chat.test.ts src/lib/rag-chat-export.test.ts
pnpm test
pnpm lint
pnpm typecheck
cmd.exe /d /s /c "set VITE_LOCAL_ML=0&& pnpm --filter @entropia-pro/desktop typecheck"
git diff --check
```

Expected result: every command exits `0`; focused and full tests report no failures; default and Lite typechecks report zero diagnostics; `git diff --check` reports no whitespace errors.

- [ ] **Step 6: Commit the implementation**

```bash
git add apps/desktop/src/views/RagChatView.svelte apps/desktop/src/views/RagChatView.test.ts
git commit -m "fix(desktop): copy chat response citations"
```

The commit must contain only the formatter wiring and its regression coverage; the already-committed design spec remains a separate commit.
