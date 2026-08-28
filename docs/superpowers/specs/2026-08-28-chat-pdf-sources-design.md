# Conversation PDF Sources Design

## Problem

Conversation PDF exports render user questions and assistant answers but omit the source list that is already attached to each persisted `RagMessage`. The chat UI renders each source as an indexed reference with a visible title/name and additional snippet, timestamp, and collection information. The PDF must include only the source title/name after each answer.

## Scope

Update the conversation PDF HTML builder in `apps/desktop/src/lib/rag-chat-export.ts` and its focused tests. No changes to persistence, the chat UI, source retrieval, markdown rendering, download flow, or the `RagSource` data model are required.

## Rendering contract

For every assistant message with one or more sources, append a `Fuentes` heading and an ordered list immediately after the rendered answer body:

```text
Fuentes
[1] First source title
[2] Second source title
```

- Iterate `message.sources` in its existing order.
- Render each source's existing 1-based `index`; do not recalculate or sort indexes.
- Render only `source.itemTitle` as the source name.
- HTML-escape source titles using the exporter's existing escaping helper.
- Omit the section for assistant messages without sources.
- User messages remain unchanged.
- Do not include snippets, OCR text, timestamps, collection names, scores, provenance, or any other metadata.

The heading and list use the same semantic HTML strategy as the existing message export (`h2` plus content). `generateNativeOcrPdfBytes` continues to convert the resulting HTML, and the download path remains unchanged.

## Implementation shape

Extend the assistant branch of the existing `messageHtml` helper to append a dedicated source-list fragment. Keep source-list rendering local to the exporter so the PDF contract is explicit and independent of UI DOM extraction. Reuse `escapeHtml` for titles and preserve the source array's order/index values verbatim.

## Verification

Add focused exporter coverage with multiple assistant sources whose titles contain HTML-sensitive characters and whose snippets/metadata contain sentinel values. Assert that:

1. `Fuentes` appears after the answer.
2. `[1]`, `[2]`, and source titles preserve input order.
3. Titles are escaped.
4. Snippets, OCR-like text, timestamps, collection names, scores, and provenance do not appear.
5. Messages without sources do not receive an empty section.

Existing filename/download and error-propagation tests remain unchanged and must continue to pass.
