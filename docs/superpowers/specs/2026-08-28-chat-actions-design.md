# Add actions to the research chat

Add response copying, conversation search, and per-conversation PDF downloads to the research chat. The existing selection, deletion, navigation, persistence, and message rendering behavior remains unchanged.

## Decision

| Topic | Decision |
|---|---|
| Copy control | Render an icon-only `IconButton` with the existing `copy` action icon on assistant messages only. |
| Clipboard payload | Copy the original assistant `message.content` string, including its line breaks. Do not copy HTML, source cards, or controls. |
| Copy feedback | Keep feedback local to the message and show it briefly without replacing or mutating message content. |
| Search scope | Search persisted conversation titles and the textual content of every persisted user and assistant message. |
| Search ownership | Keep query/results UI state in `RagChatView`; keep the active conversation in `RagChatStore`. Search must never call `select`. |
| Search transport | Add one read-only Tauri search command returning conversation summaries in the existing updated-date order. |
| Search matching | Trim the query and perform literal, case-insensitive substring matching. Empty queries restore the complete summary list and do not issue a search command. |
| PDF engine | Reuse the existing native `pdfmake` byte generator; do not introduce another PDF library or raster fallback. |
| PDF content | Include the conversation title plus every persisted question and answer in chronological order. Answers use the existing safe Markdown renderer; questions are emitted as escaped text. |
| PDF destination | Save automatically to the platform Downloads directory. Do not open a Save As dialog. |
| PDF filename | Derive a safe filename from the conversation title and append a short conversation-id suffix to avoid collisions between equal titles. |
| UI conventions | Reuse `ActionIcon`, `IconButton`, existing sizes, focus rings, hover states, and `title` tooltips. Add localized labels in Spanish and English. |

## Scope and invariants

- Every persisted assistant message has one copy action; user messages never expose it.
- The copy operation uses the raw text value and never changes rendered Markdown or source cards.
- Copy success and failure are represented by transient local feedback. No feedback is inserted into the message array.
- The conversation search list is a projection of persisted summaries. Filtering it never changes `activeConversationId`, `messages`, `draft`, or the persisted active-conversation setting.
- Search results use the same summary shape and updated-at descending order as the existing conversation list.
- A stale asynchronous search response must not replace newer results or restore results after the query was cleared.
- Each PDF action uses the id attached to its own list row. It must work when that row is not active and must not call the selection flow.
- PDF output contains all persisted messages in `sort_index` order, with no message omitted because another conversation is active.
- PDF generation is local and offline. A failed load, conversion, or write reports an error and does not claim a successful download.
- Existing selection, deletion confirmation, source navigation, new-conversation behavior, loading state, composer behavior, and Markdown rendering remain intact.
- The existing native OCR PDF adapter and its public interfaces remain unchanged.

## Module design

### `apps/desktop/src/lib/rag.ts`

Add a read-only frontend API function:

```typescript
export function ragSearchConversations(query: string): Promise<RagConversationSummary[]>
```

It invokes `rag_search_conversations` with the trimmed query. Existing list/get/delete APIs remain unchanged.

### `apps/desktop/src-tauri/src/rag/store.rs`

Add a store-level search operation that returns `RagConversationSummary` values matching the title or any message content. It reuses the existing summary projection and ordering. Matching is literal and case-insensitive; SQL wildcard characters and search operators have no special meaning.

The operation is read-only and does not load or mutate the active-conversation setting.

### `apps/desktop/src-tauri/src/rag/commands.rs` and `apps/desktop/src-tauri/src/lib.rs`

Expose the store operation as the `rag_search_conversations` Tauri command and register it alongside the existing RAG conversation commands. The command uses the same worker connection and `spawn_blocking` boundary as list/get/delete.

### `apps/desktop/src/views/RagChatView.svelte`

Add three independent UI concerns without moving active-conversation state out of `RagChatStore`:

1. **Assistant response action**
   - Add a small `IconButton` to each assistant bubble.
   - Use the `copy` icon at the same visual scale as existing chat controls.
   - Call `navigator.clipboard.writeText(message.content)`, then set transient success/error feedback for that message.
   - Keep the accessible name and tooltip bound to localized `ragChat.copyResponse` text, whose Spanish value is exactly `Copiar respuesta`.

2. **Conversation search**
   - Add an `IconButton` with the `search` icon to the conversations header.
   - Bind its active state to whether the search field is open.
   - Show a search input when open, with Escape clearing and closing the search state.
   - Debounce requests while typing, guard responses with a monotonically increasing request token, and immediately restore the unfiltered summaries when the query becomes blank.
   - Render a distinct no-match message when summaries exist but the current query returns no rows.
   - Use the original conversation button callback for selection, so filtering cannot change the active conversation by itself.

3. **Conversation PDF action**
   - Add a small download `IconButton` to each conversation row next to deletion.
   - Pass the row's `conversation.id` directly to the export handler.
   - Show per-row busy/success/error feedback and prevent duplicate clicks while that row is generating.

The existing row layout, active styling, delete button, and selection button remain separate controls with their current behavior.

### `apps/desktop/src/lib/rag-chat-export.ts`

Create a focused export module with a deep boundary around PDF preparation and filesystem output:

```typescript
export function buildRagConversationPdfHtml(conversation: RagConversation): string
export async function downloadRagConversationPdf(conversationId: string): Promise<string>
```

`buildRagConversationPdfHtml` emits a safe document fragment containing the title and message role/content blocks in source order. It escapes title and user text and delegates assistant Markdown to `renderMarkdown`, which already escapes untrusted HTML before emitting supported tags.

`downloadRagConversationPdf` loads the requested conversation with `ragGetConversation`, builds the HTML, delegates bytes to `generateNativeOcrPdfBytes`, resolves `downloadDir()`, joins the safe filename, and writes the bytes with `writeFile`. It returns the written path only after a successful write.

### `apps/desktop/src-tauri/capabilities/default.json`

Add the narrow `fs:allow-download-write` permission required for automatic writes to the platform Downloads directory. Do not broaden the existing app-data scope or add unrelated filesystem permissions.

### `apps/desktop/src/lib/i18n.ts`

Add Spanish and English strings for:

- copy response label and copied/error feedback;
- search conversations label, input placeholder, and no-match state;
- download conversation label and busy/success/error feedback.

The Spanish action labels are `Copiar respuesta`, `Buscar conversaciones`, and `Descargar conversación en PDF`.

## Data flow

### Copy

1. The user activates an assistant-message copy button.
2. The view passes that message's original `content` to the browser clipboard API.
3. On success or failure, the view updates only the transient feedback state for that message.
4. The rendered message and its sources remain unchanged.

### Search

1. The user activates the conversations-header search button.
2. The view opens the input without changing the active conversation.
3. Debounced input calls `ragSearchConversations` for non-empty queries.
4. The Tauri command reads persisted titles/messages and returns matching summaries.
5. The view replaces only the rendered list projection, rejecting stale responses by request token.
6. Clearing the query restores `ragChat.conversations` without loading or selecting any conversation.

### PDF

1. The user activates the download button in a specific conversation row.
2. The view marks only that row as busy and passes its id to `downloadRagConversationPdf`.
3. The export module fetches that id directly, independent of the active store state.
4. The module converts title/questions/answers to safe semantic HTML.
5. The existing native pdfmake generator emits selectable text bytes.
6. The module writes the bytes to Downloads and returns the path.
7. The row receives transient success or error feedback; active chat state is untouched.

## Failure behavior

- Clipboard rejection shows localized failure feedback and leaves the response unchanged.
- Search command failures leave the last valid list projection visible and show localized search error feedback; they do not clear or select the active conversation.
- A late search result is discarded when its request token is no longer current.
- Conversation-load, PDF-generation, or filesystem-write failures show localized row feedback and do not report a completed download.
- Empty or unsafe title characters produce the fallback filename stem `conversation` plus the id suffix.
- No browser HTML-to-canvas or image-only PDF fallback is allowed.

## Verification

### View behavior tests

Extend `RagChatView.test.ts` to verify:

- assistant-only copy buttons expose the exact Spanish accessible label and tooltip;
- copying preserves multiline text and shows transient feedback;
- the search button opens the input and search results match title and message content;
- clearing or changing search does not call `rag_get_conversation`, alter active `aria-current`, or replace active messages;
- every conversation row exposes the exact Spanish PDF label;
- downloading a non-active row calls `rag_get_conversation` with that row's id and leaves the active conversation rendered.

### Export module tests

Add focused tests for:

- escaped title and question text;
- assistant Markdown conversion;
- chronological role/content order;
- safe deterministic filenames;
- successful byte generation and write sequencing;
- propagation of generation/write failures without success feedback.

### Backend tests

Extend RAG store tests to cover:

- title matches;
- user-message matches;
- assistant-message matches;
- non-matches;
- updated-at descending result order;
- literal handling of `%`, `_`, and operator-like text.

### Regression checks

- Run the focused desktop chat/export tests.
- Run Lite frontend typecheck with `VITE_LOCAL_ML=0`.
- Run the relevant Rust RAG tests.
- Smoke-test the actual desktop chat: copy a multiline answer, search by answer text while another conversation is active, and download a non-active conversation to Downloads.

## Non-goals

- Changing RAG persistence schema or message ordering.
- Adding full-text indexing, ranking, highlighting, or search suggestions.
- Copying source metadata or rendered HTML to the clipboard.
- Exporting source cards, citations metadata, or unrelated application data to the PDF.
- Adding a Save As dialog, browser download prompt, or remote export service.
- Changing the existing OCR export API, PDF converter, Markdown export, or DOCX export.
- Refactoring the general icon system or adding a new tooltip component.
