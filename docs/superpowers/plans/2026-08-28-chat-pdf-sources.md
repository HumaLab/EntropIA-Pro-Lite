# Conversation PDF Sources Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Append each assistant message's ordered, numbered source titles to conversation PDF exports without exporting snippets or metadata.

**Architecture:** Keep the change inside the existing conversation HTML exporter. Pass each message's `RagSource[]` into the existing message renderer, append a small semantic `h2`/`ul` fragment only for assistant messages with sources, and let the existing pdfmake conversion and download pipeline remain unchanged.

**Tech Stack:** TypeScript, Vitest, `RagConversation`/`RagSource`, existing HTML escaping and markdown renderer, pdfmake HTML conversion.

## Global Constraints

- Use only `source.index` and `source.itemTitle` for exported source rows.
- Preserve `message.sources` array order and each existing 1-based source index.
- Do not export snippets, OCR/content text, timestamps, collection names, scores, provenance, or other metadata.
- Add `Fuentes` only after assistant answers that have at least one source.
- Keep user-message rendering, markdown rendering, filename generation, PDF generation, and download behavior unchanged.
- Escape source titles with the exporter's existing `escapeHtml` helper.
- Add behavioral exporter tests before production changes; do not rely on source-string assertions alone.

---

### Task 1: Render source titles in conversation PDF HTML

**Files:**
- Modify: `apps/desktop/src/lib/rag-chat-export.ts:6,25-29,50-56`
- Test: `apps/desktop/src/lib/rag-chat-export.test.ts:19-49`

**Interfaces:**
- Consumes: `RagConversation.messages`, where each `RagMessage` has `sources: RagSource[]`.
- Produces: `buildRagConversationPdfHtml(conversation: RagConversation): string` containing source rows after assistant answers.

- [ ] **Step 1: Extend the exporter fixture and write the failing behavior test**

Add a dedicated conversation fixture in `rag-chat-export.test.ts` with two assistant sources. Give the sources visible titles containing HTML-sensitive characters and distinct sentinel values in `snippet`, `collectionName`, `startSeconds`, `endSeconds`, `score`, and `provenance`. Then add a test shaped like:

```ts
it('appends only ordered numbered source titles after assistant answers', async () => {
  const sourceConversation: RagConversation = {
    ...conversation,
    messages: [
      {
        id: 'm1',
        role: 'assistant',
        content: 'Respuesta exportada',
        sources: [
          {
            index: 1,
            assetId: 'asset-1',
            itemId: 'item-1',
            itemTitle: 'Archivo <uno>',
            collectionId: 'collection-1',
            collectionName: 'Colección secreta',
            snippet: 'SNIPPET-NO-EXPORTAR',
            score: 0.91,
            startSeconds: 12,
            endSeconds: 34,
            provenance: {
              retrievalUnit: 'chunk',
              sourceKind: 'ocr',
              sourceId: 'source-1',
              chunkIds: ['chunk-1'],
              startChar: 10,
              endChar: 20,
            },
          },
          {
            index: 2,
            assetId: 'asset-2',
            itemId: 'item-2',
            itemTitle: 'Archivo & dos',
            collectionId: 'collection-2',
            collectionName: 'Otra colección',
            snippet: 'OCR-NO-EXPORTAR',
            score: 0.82,
            startSeconds: null,
            endSeconds: null,
            provenance: null,
          },
        ],
        createdAt: 1,
      },
    ],
  }

  const { buildRagConversationPdfHtml } = await import('./rag-chat-export')
  const html = buildRagConversationPdfHtml(sourceConversation)

  expect(html).toContain('<h2>Fuentes</h2>')
  expect(html).toContain('<li>[1] Archivo &lt;uno&gt;</li>')
  expect(html).toContain('<li>[2] Archivo &amp; dos</li>')
  expect(html.indexOf('[1] Archivo')).toBeLessThan(html.indexOf('[2] Archivo'))
  expect(html.indexOf('Fuentes')).toBeGreaterThan(html.indexOf('Respuesta exportada'))
  expect(html).not.toContain('SNIPPET-NO-EXPORTAR')
  expect(html).not.toContain('OCR-NO-EXPORTAR')
  expect(html).not.toContain('Colección secreta')
  expect(html).not.toContain('Otra colección')
  expect(html).not.toContain('source-1')
})
```

Also assert the existing no-source conversation does not contain `<h2>Fuentes</h2>`.

- [ ] **Step 2: Run the focused test and verify it fails**

Run:

```bash
pnpm --filter @entropia-pro/desktop test -- src/lib/rag-chat-export.test.ts -t "ordered numbered source titles"
```

Expected: FAIL because the current exporter has no `Fuentes` fragment and discards `message.sources`.

- [ ] **Step 3: Implement the smallest exporter change**

Import the `RagSource` type and add a local helper:

```ts
function sourceListHtml(sources: RagSource[]): string {
  if (sources.length === 0) return ''

  const rows = sources
    .map((source) => `<li>[${source.index}] ${escapeHtml(source.itemTitle)}</li>`)
    .join('')

  return `<h2>Fuentes</h2><ul>${rows}</ul>`
}
```

Change `messageHtml` to accept `sources: RagSource[]`, keep its existing user/assistant body logic, and append `sourceListHtml(sources)` only when `role === 'assistant'`. Update `buildRagConversationPdfHtml` to pass `message.sources` through. Do not modify the download function or any persisted data.

- [ ] **Step 4: Run focused exporter tests and verify the contract**

Run:

```bash
pnpm --filter @entropia-pro/desktop test -- src/lib/rag-chat-export.test.ts
```

Expected: all exporter tests pass, including title escaping, source order/index preservation, metadata exclusion, no-source omission, filename handling, and PDF-generation error propagation.

- [ ] **Step 5: Commit the implementation**

```bash
git add apps/desktop/src/lib/rag-chat-export.ts apps/desktop/src/lib/rag-chat-export.test.ts
git commit -m "fix(desktop): include sources in chat PDF exports"
```

---

### Task 2: Verify the complete change

**Files:**
- Verify: `apps/desktop/src/lib/rag-chat-export.ts`
- Verify: `apps/desktop/src/lib/rag-chat-export.test.ts`

**Interfaces:**
- Consumes: Task 1's committed exporter and focused tests.
- Produces: Test, lint, typecheck, and whitespace-clean evidence for the final candidate.

- [ ] **Step 1: Run the focused adjacent chat tests**

```bash
pnpm --filter @entropia-pro/desktop test -- src/lib/rag-chat-export.test.ts src/lib/rag-chat.test.ts src/views/RagChatView.test.ts
```

Expected: all selected desktop tests pass.

- [ ] **Step 2: Run repository quality checks**

```bash
pnpm test
pnpm lint
pnpm typecheck
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck
```

Expected: every command exits zero; no new errors or warnings are reported.

- [ ] **Step 3: Check the final worktree**

```bash
git diff --check
git status --short
```

Expected: `git diff --check` emits no diagnostics and only the intended implementation commits are present; no uncommitted source changes remain.
