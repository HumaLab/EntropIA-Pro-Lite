# Chat Response Copy Citations Design

## Goal

Make the assistant response copy action include the same minimal source citations already present in conversation PDF exports.

## User-visible contract

When copying an assistant message with one or more sources, the clipboard text must be:

```text
<original assistant response>

Fuentes:
[1] source-title
[2] another-source-title
```

The response content remains unchanged. The source list follows the existing `message.sources` array order and uses each source's existing `index` and `itemTitle`. Source snippets, collection names, timestamps, scores, OCR text, provenance, and every other metadata field are excluded. Source titles are plain text in the clipboard payload; no HTML is generated.

When an assistant message has no sources, copying remains exactly the current `message.content` payload. User messages have no copy action and are unaffected.

## Architecture and data flow

`RagChatView.svelte` owns the copy button and already receives each `UiMessage` with its `sources` array. The current `copyResponse` handler sends only `message.content` to `navigator.clipboard.writeText`. Add a small local plain-text formatter beside the handler that appends the `Fuentes:` block only for nonempty source arrays, then pass that computed payload to the existing clipboard call.

No changes are needed to persistence, the Rust/Tauri RAG contract, PDF generation, navigation, feedback state, or localization. The format is intentionally local to the copy action because the PDF exporter emits HTML and the UI source renderer includes metadata that must not be copied.

## Formatting and edge cases

- Preserve the original assistant content exactly; append the citation block without normalizing or trimming it.
- Insert one blank line before `Fuentes:`.
- Preserve source array order, including the source-provided numbering rather than renumbering entries.
- Do not append anything for an empty or missing source array.
- Copy source titles as literal text, so characters such as `&`, `<`, and Unicode remain unchanged in the clipboard.
- Keep the existing asynchronous copy feedback and stale conversation/message guards unchanged.

## Testing strategy

Extend `apps/desktop/src/views/RagChatView.test.ts` at the existing clipboard action seam. Add a regression that clicks an assistant copy button with multiple sources and asserts the exact plain-text payload, including the blank line, `Fuentes:` heading, source order/index, and a title containing special characters. Assert source metadata sentinels are absent. Keep the existing no-source copy behavior covered and verify the existing success/error feedback behavior remains intact.

Run the focused view test first, then the desktop and workspace verification commands required by the repository.

## Alternatives considered

1. **Copy rendered DOM text:** rejected because the rendered source block intentionally includes snippets, collection names, and timestamps; it would violate the title-only copy contract and couple clipboard output to presentation markup.
2. **Add citations to `message.content` at persistence time:** rejected because persisted assistant content is shared by chat rendering, search, and export; mutating it would duplicate citations and alter stored conversation data.
3. **Reuse the PDF HTML formatter:** rejected because clipboard output is plain text and should not carry HTML parsing or PDF-specific structure.

## Scope and non-goals

In scope: the assistant copy payload and its regression test.

Out of scope: changing visible chat source rendering, PDF exports, source ordering/data, persistence schema, translations, or copy feedback behavior.
