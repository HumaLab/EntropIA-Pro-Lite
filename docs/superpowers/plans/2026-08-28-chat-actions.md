# Chat Actions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add copy-response, conversation search, and per-conversation automatic PDF download actions to the research chat without changing active conversation behavior.

**Architecture:** Keep active conversation state in `RagChatStore`. Add a read-only Rust/Tauri search command that returns the existing conversation-summary shape, while `RagChatView` owns only search projection and transient action feedback. Add a focused chat-export module that fetches the clicked conversation id directly, reuses the existing native `pdfmake` generator, and writes bytes to the platform Downloads directory.

**Tech Stack:** Svelte 5 runes, TypeScript, Vitest, Testing Library for Svelte, Tauri 2, Rust/rusqlite, `@tauri-apps/plugin-fs`, `@tauri-apps/api/path`, existing `pdfmake`/`pdfjs-dist` stack.

## Global Constraints

- Copy only the original assistant `message.content`, including line breaks; never copy HTML, source cards, or controls.
- Every persisted assistant message has one copy action; user messages never expose it.
- Search persisted titles and every persisted user/assistant message with literal, case-insensitive substring matching.
- Empty search queries restore the complete summary list and do not issue a search command.
- Search must never call `select` or change `activeConversationId`, `messages`, `draft`, or the active-conversation setting.
- Discard stale asynchronous search results with a monotonically increasing request token.
- Each PDF action must use its own row's `conversation.id`, including when that conversation is not active.
- PDF output contains the title and every persisted question and answer in chronological `sort_index` order.
- Answers use the existing safe Markdown renderer; questions are escaped text.
- Reuse the existing native `pdfmake` generator; add no PDF library and no raster fallback.
- Save PDFs automatically to the platform Downloads directory with `downloadDir()` and `writeFile()`; do not open a Save As dialog.
- Add only the narrow `fs:allow-download-write` capability.
- Reuse `ActionIcon`, `IconButton`, existing sizes, focus rings, hover states, and `title` tooltips.
- Spanish labels are exactly `Copiar respuesta`, `Buscar conversaciones`, and `Descargar conversación en PDF`.
- Existing selection, deletion confirmation, source navigation, new-conversation behavior, loading state, composer behavior, and Markdown rendering remain unchanged.
- The existing native OCR PDF adapter and its public interfaces remain unchanged.
- Generated technical artifacts, code, tests, and comments remain in English; localized runtime strings are added in Spanish and English.
- Do not run formatters, linters, or project-wide test suites during individual implementation tasks; run bounded checks in the final verification task.

---

## File Map

| File | Responsibility |
|---|---|
| `apps/desktop/src-tauri/src/rag/store.rs` | Read-only title/message search over persisted RAG conversations and unit tests. |
| `apps/desktop/src-tauri/src/rag/commands.rs` | Tauri boundary for the search operation. |
| `apps/desktop/src-tauri/src/lib.rs` | Register the new RAG command. |
| `apps/desktop/src/lib/rag.ts` | Typed frontend wrapper for `rag_search_conversations`. |
| `apps/desktop/src/lib/rag-chat-export.ts` | Safe conversation-to-HTML conversion and automatic PDF file output. |
| `apps/desktop/src/lib/rag-chat-export.test.ts` | Export HTML, filename, byte-generation, and write sequencing tests. |
| `apps/desktop/src/views/RagChatView.svelte` | Copy controls, search controls, per-row PDF controls, transient feedback, and styles. |
| `apps/desktop/src/views/RagChatView.test.ts` | Observable chat action and regression tests. |
| `apps/desktop/src/lib/i18n.ts` | Spanish and English labels, feedback, and state messages. |
| `apps/desktop/src-tauri/capabilities/default.json` | Permission for automatic writes to Downloads. |

## Interfaces Between Tasks

Task 1 produces:

```typescript
export function ragSearchConversations(query: string): Promise<RagConversationSummary[]>
```

and registers:

```text
rag_search_conversations(query: String) -> Result<Vec<RagConversationSummary>, String>
```

Task 2 produces:

```typescript
export function buildRagConversationPdfHtml(conversation: RagConversation): string
export async function downloadRagConversationPdf(conversationId: string): Promise<string>
```

Task 3 consumes both interfaces and passes the clicked row id unchanged to `downloadRagConversationPdf`.

---

### Task 1: Add read-only conversation search

**Files:**
- Modify: `apps/desktop/src-tauri/src/rag/store.rs`
- Modify: `apps/desktop/src-tauri/src/rag/commands.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/lib/rag.ts`
- Test: `apps/desktop/src-tauri/src/rag/store.rs` unit-test module

**Interfaces:**
- Consumes: existing `list_conversations`, `RagConversationSummary`, `rag_messages`, and the existing `spawn_blocking` worker-connection pattern.
- Produces: the `ragSearchConversations` TypeScript wrapper and the registered `rag_search_conversations` command used by Task 3.

- [ ] **Step 1: Write the failing Rust search contract test**

Append a test to the existing `apps/desktop/src-tauri/src/rag/store.rs` test module. Use the existing `setup_conn()` helper and insert three conversations with different `updated_at` values and user/assistant messages:

```rust
#[test]
fn search_conversations_matches_titles_and_all_message_text_literally() {
    let conn = setup_conn();

    conn.execute(
        "INSERT INTO rag_conversations(id, title, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4)",
        params!["conv-old", "Salarios del SOIP", 100, 200],
    )
    .expect("insert old conversation");
    conn.execute(
        "INSERT INTO rag_conversations(id, title, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4)",
        params!["conv-new", "Acta sindical", 300, 400],
    )
    .expect("insert new conversation");
    conn.execute(
        "INSERT INTO rag_conversations(id, title, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4)",
        params!["conv-empty", "Sin coincidencias", 500, 600],
    )
    .expect("insert unmatched conversation");

    let messages = [
        ("msg-old-user", "conv-old", 0, "¿Cuánto ganaban?"),
        ("msg-old-assistant", "conv-old", 1, "El 100% efectivo del jornal."),
        ("msg-new-user", "conv-new", 0, "Pregunta sobre la huelga"),
        ("msg-new-assistant", "conv-new", 1, "La respuesta menciona operadores."),
    ];
    for (id, conversation_id, sort_index, content) in messages {
        conn.execute(
            "INSERT INTO rag_messages(id, conversation_id, sort_index, role, content, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                conversation_id,
                sort_index,
                if sort_index == 0 { "user" } else { "assistant" },
                content,
                100 + sort_index,
            ],
        )
        .expect("insert search message");
    }

    let title_match = search_conversations(&conn, "SALARIOS").expect("title search");
    assert_eq!(
        title_match.iter().map(|conversation| conversation.id.as_str()).collect::<Vec<_>>(),
        vec!["conv-old"]
    );

    let assistant_match = search_conversations(&conn, "OPERADORES").expect("assistant search");
    assert_eq!(
        assistant_match.iter().map(|conversation| conversation.id.as_str()).collect::<Vec<_>>(),
        vec!["conv-new"]
    );

    let literal_match = search_conversations(&conn, "%").expect("literal percent search");
    assert_eq!(
        literal_match.iter().map(|conversation| conversation.id.as_str()).collect::<Vec<_>>(),
        vec!["conv-old"]
    );

    let no_match = search_conversations(&conn, "AND").expect("operator-like search");
    assert!(no_match.is_empty());

    let all = search_conversations(&conn, " ").expect("empty search");
    assert_eq!(
        all.iter().map(|conversation| conversation.id.as_str()).collect::<Vec<_>>(),
        vec!["conv-empty", "conv-new", "conv-old"]
    );
}
```

- [ ] **Step 2: Run only the new Rust test and verify it fails**

Run:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml search_conversations_matches_titles_and_all_message_text_literally --lib
```

Expected: FAIL because `search_conversations` does not exist.

- [ ] **Step 3: Implement the store search with Unicode-safe literal matching**

Add `use std::collections::HashSet;` to `store.rs` and implement the operation after `list_conversations`:

```rust
/// Literal, case-insensitive search across titles and all message content.
/// Results preserve the order returned by `list_conversations`.
pub(crate) fn search_conversations(
    conn: &Connection,
    query: &str,
) -> Result<Vec<RagConversationSummary>, String> {
    let needle = query.trim().to_lowercase();
    let conversations = list_conversations(conn)?;
    if needle.is_empty() {
        return Ok(conversations);
    }

    let mut stmt = conn
        .prepare("SELECT conversation_id, content FROM rag_messages")
        .map_err(|e| format!("Failed to prepare RAG conversation search query: {e}"))?;
    let mut message_matches = HashSet::new();
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("Failed to run RAG conversation search query: {e}"))?;

    for row in rows {
        let (conversation_id, content) =
            row.map_err(|e| format!("Failed to read RAG conversation search row: {e}"))?;
        if content.to_lowercase().contains(&needle) {
            message_matches.insert(conversation_id);
        }
    }

    Ok(conversations
        .into_iter()
        .filter(|conversation| {
            conversation.title.to_lowercase().contains(&needle)
                || message_matches.contains(&conversation.id)
        })
        .collect())
}
```

This deliberately performs matching in Rust rather than SQL `LIKE`, so `%`, `_`, and operator-like text remain literal and Unicode lowercasing works consistently with the frontend expectation.

- [ ] **Step 4: Run the Rust search test and verify it passes**

Run the same focused command:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml search_conversations_matches_titles_and_all_message_text_literally --lib
```

Expected: PASS.

- [ ] **Step 5: Add the Tauri command and register it**

Add the command after `rag_list_conversations` in `commands.rs`:

```rust
/// Busca conversaciones por título o contenido textual, sin modificar la activa.
#[tauri::command]
pub async fn rag_search_conversations(
    query: String,
    db: tauri::State<'_, crate::db::state::AppDbState>,
) -> Result<Vec<RagConversationSummary>, String> {
    let conn_arc = db.worker_conn.clone();
    tokio::task::spawn_blocking(move || -> Result<Vec<RagConversationSummary>, String> {
        let conn = conn_arc.lock().map_err(|e| e.to_string())?;
        store::search_conversations(&conn, &query)
    })
    .await
    .map_err(|e| format!("RAG search task panicked: {e}"))?
}
```

Register it next to the existing list/get/delete commands in `apps/desktop/src-tauri/src/lib.rs`:

```rust
rag::commands::rag_list_conversations,
rag::commands::rag_search_conversations,
rag::commands::rag_get_conversation,
```

- [ ] **Step 6: Add the typed frontend wrapper**

Add this function after `ragListConversations` in `apps/desktop/src/lib/rag.ts`:

```typescript
/** Search persisted conversation titles and user/assistant message text. */
export function ragSearchConversations(query: string): Promise<RagConversationSummary[]> {
  return invoke<RagConversationSummary[]>('rag_search_conversations', {
    query: query.trim(),
  })
}
```

- [ ] **Step 7: Run the focused Rust test again and commit Task 1**

Run:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml search_conversations_matches_titles_and_all_message_text_literally --lib
```

Expected: PASS.

Commit:

```bash
git add apps/desktop/src-tauri/src/rag/store.rs apps/desktop/src-tauri/src/rag/commands.rs apps/desktop/src-tauri/src/lib.rs apps/desktop/src/lib/rag.ts
git commit -m "feat(rag): add conversation search command"
```

---

### Task 2: Add native automatic conversation PDF export

**Files:**
- Create: `apps/desktop/src/lib/rag-chat-export.ts`
- Create: `apps/desktop/src/lib/rag-chat-export.test.ts`
- Modify: `apps/desktop/src-tauri/capabilities/default.json`

**Interfaces:**
- Consumes: `RagConversation`, `ragGetConversation`, `renderMarkdown`, `generateNativeOcrPdfBytes`, Tauri `downloadDir`/`join`, and plugin-fs `writeFile`.
- Produces: `buildRagConversationPdfHtml` and `downloadRagConversationPdf` for Task 3.

- [ ] **Step 1: Write failing export tests with isolated adapters**

Create `apps/desktop/src/lib/rag-chat-export.test.ts` with module mocks for the conversation loader, PDF generator, path API, and filesystem writer:

```typescript
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { RagConversation } from './rag'
import { downloadDir, join } from '@tauri-apps/api/path'
import { writeFile } from '@tauri-apps/plugin-fs'

const { getConversationMock, generatePdfMock } = vi.hoisted(() => ({
  getConversationMock: vi.fn(),
  generatePdfMock: vi.fn(),
}))

vi.mock('./rag', () => ({ ragGetConversation: getConversationMock }))
vi.mock('./ocr-pdf', () => ({ generateNativeOcrPdfBytes: generatePdfMock }))
vi.mock('@tauri-apps/api/path', () => ({
  downloadDir: vi.fn(),
  join: vi.fn(),
}))
vi.mock('@tauri-apps/plugin-fs', () => ({ writeFile: vi.fn() }))

const conversation: RagConversation = {
  id: 'conv-42',
  title: 'Acta <sindical>',
  messages: [
    { id: 'm1', role: 'user', content: 'Pregunta\ncon dos líneas', sources: [], createdAt: 1 },
    { id: 'm2', role: 'assistant', content: '**Respuesta** con `formato`', sources: [], createdAt: 2 },
  ],
}

beforeEach(() => {
  vi.resetModules()
  vi.mocked(getConversationMock).mockReset()
  vi.mocked(generatePdfMock).mockReset()
  vi.mocked(downloadDir).mockReset()
  vi.mocked(join).mockReset()
  vi.mocked(writeFile).mockReset()
})

describe('buildRagConversationPdfHtml', () => {
  it('escapes title and questions, renders answers, and preserves message order', async () => {
    const { buildRagConversationPdfHtml } = await import('./rag-chat-export')
    const html = buildRagConversationPdfHtml(conversation)

    expect(html).toContain('<h1>Acta &lt;sindical&gt;</h1>')
    expect(html).toContain('Pregunta<br>con dos líneas')
    expect(html).toContain('<strong>Respuesta</strong>')
    expect(html).toContain('<code>formato</code>')
    expect(html.indexOf('Pregunta')).toBeLessThan(html.indexOf('Respuesta'))
    expect(html).not.toContain('<sindical>')
  })
})

describe('downloadRagConversationPdf', () => {
  it('loads the requested id and writes generated bytes to Downloads', async () => {
    const bytes = Uint8Array.from([1, 2, 3])
    getConversationMock.mockResolvedValue(conversation)
    generatePdfMock.mockResolvedValue(bytes)
    vi.mocked(downloadDir).mockResolvedValue('C:/Users/test/Downloads')
    vi.mocked(join).mockResolvedValue('C:/Users/test/Downloads/Acta sindical - conv-42.pdf')
    vi.mocked(writeFile).mockResolvedValue(undefined)

    const { downloadRagConversationPdf } = await import('./rag-chat-export')
    await expect(downloadRagConversationPdf('conv-42')).resolves.toBe(
      'C:/Users/test/Downloads/Acta sindical - conv-42.pdf'
    )

    expect(getConversationMock).toHaveBeenCalledWith('conv-42')
    expect(generatePdfMock).toHaveBeenCalledWith(expect.stringContaining('Acta &lt;sindical&gt;'))
    expect(join).toHaveBeenCalledWith(
      'C:/Users/test/Downloads',
      'Acta sindical - conv-42.pdf'
    )
    expect(writeFile).toHaveBeenCalledWith(
      'C:/Users/test/Downloads/Acta sindical - conv-42.pdf',
      bytes
    )
  })

  it('does not write when PDF generation fails', async () => {
    getConversationMock.mockResolvedValue(conversation)
    generatePdfMock.mockRejectedValue(new Error('pdf failed'))

    const { downloadRagConversationPdf } = await import('./rag-chat-export')
    await expect(downloadRagConversationPdf('conv-42')).rejects.toThrow('pdf failed')
    expect(writeFile).not.toHaveBeenCalled()
  })
})
```

- [ ] **Step 2: Run the new export tests and verify they fail**

Run:

```bash
pnpm --filter @entropia-pro/desktop test -- src/lib/rag-chat-export.test.ts
```

Expected: FAIL because `rag-chat-export.ts` does not exist.

- [ ] **Step 3: Implement safe HTML conversion, filename creation, and automatic output**

Create `apps/desktop/src/lib/rag-chat-export.ts`:

```typescript
import { downloadDir, join } from '@tauri-apps/api/path'
import { writeFile } from '@tauri-apps/plugin-fs'

import { renderMarkdown } from './markdown'
import { generateNativeOcrPdfBytes } from './ocr-pdf'
import { ragGetConversation, type RagConversation } from './rag'

const EXPORT_CLASS = 'ocr-export-document'
const MAX_FILENAME_STEM_LENGTH = 80

function escapeHtml(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;')
}

function questionHtml(content: string): string {
  const normalized = content.replace(/\r\n?/g, '\n')
  return `<p>${escapeHtml(normalized).replaceAll('\n', '<br>')}</p>`
}

function messageHtml(role: 'user' | 'assistant', content: string): string {
  const roleLabel = role === 'user' ? 'Pregunta' : 'Respuesta'
  const body = role === 'assistant' ? renderMarkdown(content) : questionHtml(content)
  return `<h2>${roleLabel}</h2>${body || '<p></p>'}`
}

function filenameStem(title: string): string {
  const cleaned = title
    .normalize('NFKC')
    .replace(/[<>:"/\\|?*\u0000-\u001f]/g, '')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, MAX_FILENAME_STEM_LENGTH)
  return cleaned || 'conversation'
}

function conversationFilename(conversation: RagConversation): string {
  return `EntropIA - ${filenameStem(conversation.title)} - ${conversation.id.slice(0, 8)}.pdf`
}

export function buildRagConversationPdfHtml(conversation: RagConversation): string {
  const messages = conversation.messages
    .map((message) => messageHtml(message.role, message.content))
    .join('')

  return `<!doctype html><html><head><meta charset="utf-8"></head><body><div class="${EXPORT_CLASS}"><h1>${escapeHtml(conversation.title)}</h1>${messages}</div></body></html>`
}

export async function downloadRagConversationPdf(conversationId: string): Promise<string> {
  const conversation = await ragGetConversation(conversationId)
  const bytes = await generateNativeOcrPdfBytes(buildRagConversationPdfHtml(conversation))
  const directory = await downloadDir()
  const path = await join(directory, conversationFilename(conversation))
  await writeFile(path, bytes)
  return path
}
```

The title and questions are escaped before entering the existing semantic converter. Assistant content passes through `renderMarkdown`, which escapes untrusted HTML before emitting supported semantic tags. The id suffix makes equal titles distinct while the `EntropIA - ` prefix avoids Windows reserved bare filenames.

- [ ] **Step 4: Add the narrow Downloads write capability**

Insert this permission in `apps/desktop/src-tauri/capabilities/default.json` next to the existing Downloads read permission:

```json
"fs:allow-download-write"
```

Do not change the existing `$APPDATA/**/*` scope or add recursive write access.

- [ ] **Step 5: Run the focused export tests and commit Task 2**

Run:

```bash
pnpm --filter @entropia-pro/desktop test -- src/lib/rag-chat-export.test.ts
```

Expected: PASS.

Commit:

```bash
git add apps/desktop/src/lib/rag-chat-export.ts apps/desktop/src/lib/rag-chat-export.test.ts apps/desktop/src-tauri/capabilities/default.json
git commit -m "feat(desktop): export chat conversations to PDF"
```

---

### Task 3: Add copy, search, and download controls to the chat view

**Files:**
- Modify: `apps/desktop/src/views/RagChatView.svelte`
- Modify: `apps/desktop/src/views/RagChatView.test.ts`
- Modify: `apps/desktop/src/lib/i18n.ts`

**Interfaces:**
- Consumes: `ragSearchConversations` from Task 1 and `downloadRagConversationPdf` from Task 2.
- Produces: the complete user-visible action surface while retaining all existing selection/deletion/navigation callbacks.

- [ ] **Step 1: Extend test mocks and write failing view tests**

In `RagChatView.test.ts`, add the export mock to the existing `vi.hoisted` block and mock module:

```typescript
const { navigateMock, downloadRagConversationPdfMock } = vi.hoisted(() => ({
  navigateMock: vi.fn(),
  downloadRagConversationPdfMock: vi.fn(),
}))

vi.mock('$lib/rag-chat-export', () => ({
  downloadRagConversationPdf: downloadRagConversationPdfMock,
}))
```

Extend the backend test state with an optional search result and handle the new command in `setupBackend`:

```typescript
interface BackendState {
  storedActiveId: string | null
  summaries: RagConversationSummary[]
  searchResults?: RagConversationSummary[]
  conversations: Record<string, RagConversation>
  ask: (args: { question: string; conversationId?: string }) => Promise<RagAnswer> | RagAnswer
}
```

Add this switch branch before `rag_get_conversation`:

```typescript
case 'rag_search_conversations':
  return state.searchResults ?? state.summaries
```

Add the following tests to `describe('RagChatView')`:

```typescript
it('copies only assistant content and shows transient feedback', async () => {
  setupBackend({ ask: () => answerWithSources })
  const writeText = vi.fn().mockResolvedValue(undefined)
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText },
  })

  render(RagChatView)
  await sendQuestion('¿Cuándo comenzó la huelga?')

  const copy = await screen.findByRole('button', { name: 'Copiar respuesta' })
  expect(screen.queryByRole('button', { name: 'Copiar respuesta' })).toBeInTheDocument()
  expect(copy).toHaveAttribute('title', 'Copiar respuesta')
  expect(screen.queryByRole('button', { name: 'Copiar pregunta' })).not.toBeInTheDocument()

  await fireEvent.click(copy)

  expect(writeText).toHaveBeenCalledWith('La huelga comenzó en junio de 1966 [1].')
  expect(screen.getByText('Copiado')).toBeInTheDocument()
  expect(screen.getByText('La huelga comenzó en junio de 1966 [1].')).toBeInTheDocument()
})

it('searches title and message text without changing the active conversation', async () => {
  setupBackend({
    storedActiveId: 'conv-1',
    summaries: conversationSummaries,
    searchResults: [conversationSummaries[1]!],
    conversations: { 'conv-1': storedConversation },
  })

  render(RagChatView)
  await waitFor(() => {
    expect(screen.getByText('La huelga comenzó en junio de 1966 [1].')).toBeInTheDocument()
  })

  await fireEvent.click(screen.getByRole('button', { name: 'Buscar conversaciones' }))
  const search = screen.getByRole('searchbox', { name: 'Buscar conversaciones' })
  await fireEvent.input(search, { target: { value: 'jornal' } })
  await waitFor(() => {
    expect(callsFor('rag_search_conversations')).toEqual([
      ['rag_search_conversations', { query: 'jornal' }],
    ])
  })

  expect(screen.getByText('Salarios del SOIP')).toBeInTheDocument()
  expect(
    screen.queryByRole('button', { name: /¿Cuándo comenzó la huelga\?/ })
  ).not.toBeInTheDocument()
  expect(screen.getByText('La huelga comenzó en junio de 1966 [1].')).toBeInTheDocument()
  expect(callsFor('rag_get_conversation')).toEqual([
    ['rag_get_conversation', { conversationId: 'conv-1' }],
  ])
})

it('downloads the conversation attached to a non-active row', async () => {
  setupBackend({
    storedActiveId: 'conv-1',
    summaries: conversationSummaries,
    conversations: { 'conv-1': storedConversation },
  })
  downloadRagConversationPdfMock.mockResolvedValue('C:/Users/test/Downloads/Salarios.pdf')

  render(RagChatView)
  await waitFor(() => {
    expect(screen.getByText('Salarios del SOIP')).toBeInTheDocument()
  })
  const downloadButtons = screen.getAllByRole('button', {
    name: 'Descargar conversación en PDF',
  })
  expect(downloadButtons[1]).toHaveAttribute('title', 'Descargar conversación en PDF')
  await fireEvent.click(downloadButtons[1]!)

  expect(downloadRagConversationPdfMock).toHaveBeenCalledWith('conv-2')
  expect(screen.getByText('La huelga comenzó en junio de 1966 [1].')).toBeInTheDocument()
})
```

Use the existing test fixture's active conversation and retain all current regression tests for selecting, deleting, source navigation, loading, and new conversations.

- [ ] **Step 2: Run the new view tests and verify they fail**

Run:

```bash
pnpm --filter @entropia-pro/desktop test -- src/views/RagChatView.test.ts
```

Expected: FAIL because the new buttons, searchbox, labels, and export mock calls do not exist.

- [ ] **Step 3: Add localized runtime strings**

Add these keys to both dictionaries in `apps/desktop/src/lib/i18n.ts`.

Spanish values:

```typescript
'ragChat.copyResponse': 'Copiar respuesta',
'ragChat.copiedResponse': 'Copiado',
'ragChat.copyResponseError': 'No se pudo copiar la respuesta.',
'ragChat.searchConversations': 'Buscar conversaciones',
'ragChat.searchConversationsPlaceholder': 'Buscar por título o contenido…',
'ragChat.searchingConversations': 'Buscando conversaciones…',
'ragChat.searchConversationsError': 'No se pudieron buscar las conversaciones.',
'ragChat.noMatchingConversations': 'No se encontraron conversaciones',
'ragChat.downloadConversation': 'Descargar conversación en PDF',
'ragChat.downloadingConversation': 'Generando PDF…',
'ragChat.downloadedConversation': 'PDF descargado',
'ragChat.downloadConversationError': 'No se pudo descargar la conversación.',
```

English values:

```typescript
'ragChat.copyResponse': 'Copy response',
'ragChat.copiedResponse': 'Copied',
'ragChat.copyResponseError': 'The response could not be copied.',
'ragChat.searchConversations': 'Search conversations',
'ragChat.searchConversationsPlaceholder': 'Search by title or content…',
'ragChat.searchConversationsError': 'The conversations could not be searched.',
'ragChat.searchingConversations': 'Searching conversations…',
'ragChat.noMatchingConversations': 'No conversations found',
'ragChat.downloadConversation': 'Download conversation as PDF',
'ragChat.downloadingConversation': 'Generating PDF…',
'ragChat.downloadedConversation': 'PDF downloaded',
'ragChat.downloadConversationError': 'The conversation could not be downloaded.',
```

- [ ] **Step 4: Implement copy feedback and conversation search state**

Update imports in `RagChatView.svelte`:

```typescript
import { onDestroy, tick } from 'svelte'
import { ragSearchConversations, type RagConversationSummary } from '$lib/rag'
import { downloadRagConversationPdf } from '$lib/rag-chat-export'
```

Add the following state and handlers after the existing delete state. The token and timer keep stale searches from changing the current projection:

```typescript
type ActionFeedback = 'success' | 'error'
type DownloadFeedback = 'loading' | 'success' | 'error'

let copyFeedback = $state<{ messageIndex: number; tone: ActionFeedback } | null>(null)
let copyFeedbackTimer: ReturnType<typeof setTimeout> | null = null
let conversationSearchOpen = $state(false)
let conversationQuery = $state('')
let conversationSearchResults = $state<RagConversationSummary[] | null>(null)
let conversationSearchError = $state<string | null>(null)
let conversationSearchLoading = $state(false)
let conversationSearchTimer: ReturnType<typeof setTimeout> | null = null
let conversationSearchRequest = 0
let downloadFeedback = $state<Record<string, DownloadFeedback>>({})
let downloadFeedbackTimers = new Map<string, ReturnType<typeof setTimeout>>()

let visibleConversations = $derived(
  conversationQuery.trim() ? (conversationSearchResults ?? []) : $ragChat.conversations
)

function showCopyFeedback(messageIndex: number, tone: ActionFeedback) {
  if (copyFeedbackTimer) clearTimeout(copyFeedbackTimer)
  copyFeedback = { messageIndex, tone }
  copyFeedbackTimer = setTimeout(() => {
    if (copyFeedback?.messageIndex === messageIndex) copyFeedback = null
    copyFeedbackTimer = null
  }, 1500)
}

async function copyResponse(message: UiMessage, messageIndex: number) {
  try {
    await navigator.clipboard.writeText(message.content)
    showCopyFeedback(messageIndex, 'success')
  } catch {
    showCopyFeedback(messageIndex, 'error')
  }
}

function scheduleConversationSearch(value: string) {
  conversationQuery = value
  conversationSearchError = null
  conversationSearchRequest += 1
  const request = conversationSearchRequest
  if (conversationSearchTimer) clearTimeout(conversationSearchTimer)

  if (!value.trim()) {
    conversationSearchResults = null
    conversationSearchLoading = false
    return
  }

  conversationSearchLoading = true
  conversationSearchTimer = setTimeout(async () => {
    conversationSearchTimer = null
    try {
      const results = await ragSearchConversations(value)
      if (request !== conversationSearchRequest) return
      conversationSearchResults = results
      conversationSearchLoading = false
    } catch {
      if (request !== conversationSearchRequest) return
      conversationSearchLoading = false
      conversationSearchError = t('ragChat.searchConversationsError')
    }
  }, 250)
}

async function openConversationSearch() {
  conversationSearchOpen = true
  await tick()
  conversationSearchInput?.focus()
}

function closeConversationSearch() {
  conversationSearchRequest += 1
  if (conversationSearchTimer) clearTimeout(conversationSearchTimer)
  conversationSearchTimer = null
  conversationSearchOpen = false
  conversationQuery = ''
  conversationSearchResults = null
  conversationSearchError = null
  conversationSearchLoading = false
}

function toggleConversationSearch() {
  if (conversationSearchOpen) closeConversationSearch()
  else void openConversationSearch()
}

async function downloadConversation(conversationId: string) {
  if (downloadFeedback[conversationId] === 'loading') return
  downloadFeedback = { ...downloadFeedback, [conversationId]: 'loading' }
  try {
    await downloadRagConversationPdf(conversationId)
    downloadFeedback = { ...downloadFeedback, [conversationId]: 'success' }
  } catch {
    downloadFeedback = { ...downloadFeedback, [conversationId]: 'error' }
  }
  const previousTimer = downloadFeedbackTimers.get(conversationId)
  if (previousTimer) clearTimeout(previousTimer)
  const timer = setTimeout(() => {
    const next = { ...downloadFeedback }
    delete next[conversationId]
    downloadFeedback = next
    downloadFeedbackTimers.delete(conversationId)
  }, 1800)
  downloadFeedbackTimers.set(conversationId, timer)
}
```

Add these declarations near `messagesEl`:

```typescript
let conversationSearchInput = $state<HTMLInputElement | undefined>()
```


- [ ] **Step 5: Add assistant copy controls without changing message content**

Inside the existing assistant branch, keep the Markdown renderer unchanged and add an action row after the rendered content and source section:

```svelte
{#if message.role === 'assistant'}
  <div class="rag-chat__message-actions">
    <IconButton
      size="sm"
      label={$currentLocale && t('ragChat.copyResponse')}
      title={$currentLocale && t('ragChat.copyResponse')}
      onclick={() => void copyResponse(message, index)}
    >
      <ActionIcon name="copy" size={14} />
    </IconButton>
    {#if copyFeedback?.messageIndex === index}
      <span class="rag-chat__action-feedback" role="status">
        {copyFeedback.tone === 'success'
          ? ($currentLocale && t('ragChat.copiedResponse'))
          : ($currentLocale && t('ragChat.copyResponseError'))}
      </span>
    {/if}
  </div>
{/if}
```

Place this row inside the assistant bubble after the existing `sources` block. The copied payload remains `message.content`, while `messageContent(message)` continues to control the rendered no-results fallback.

- [ ] **Step 6: Add the conversations header search control and filtered projection**

Replace the current sidebar header contents with a title plus icon action:

```svelte
<header class="rag-chat__sidebar-header">
  <div class="rag-chat__sidebar-heading">
    <h2 class="rag-chat__sidebar-title">{$currentLocale && t('ragChat.conversations')}</h2>
    <IconButton
      size="sm"
      active={conversationSearchOpen}
      label={$currentLocale && t('ragChat.searchConversations')}
      title={$currentLocale && t('ragChat.searchConversations')}
      onclick={toggleConversationSearch}
    >
      <ActionIcon name="search" size={16} />
    </IconButton>
  </div>
  {#if conversationSearchOpen}
    <input
      bind:this={conversationSearchInput}
      class="rag-chat__conversation-search"
      type="search"
      role="searchbox"
      value={conversationQuery}
      placeholder={$currentLocale && t('ragChat.searchConversationsPlaceholder')}
      aria-label={$currentLocale && t('ragChat.searchConversations')}
      oninput={(event) => scheduleConversationSearch(event.currentTarget.value)}
      onkeydown={(event) => {
        if (event.key === 'Escape') {
          event.preventDefault()
          closeConversationSearch()
        }
      }}
    />
  {/if}
</header>
```

Render `visibleConversations` instead of `$ragChat.conversations`. Keep each row's existing selection button and `aria-current` expression unchanged. Distinguish empty states:

```svelte
{#if conversationSearchError}
  <p class="rag-chat__sidebar-empty" role="alert">{conversationSearchError}</p>
{:else if conversationSearchLoading}
  <p class="rag-chat__sidebar-empty" role="status">{$currentLocale && t('ragChat.searchingConversations')}</p>
{:else if visibleConversations.length === 0 && conversationQuery.trim()}
  <p class="rag-chat__sidebar-empty">{$currentLocale && t('ragChat.noMatchingConversations')}</p>
{:else if visibleConversations.length === 0}
  <p class="rag-chat__sidebar-empty">{$currentLocale && t('ragChat.noConversations')}</p>
{:else}
  <ul class="rag-chat__conversations">
    {#each visibleConversations as conversation (conversation.id)}
      <!-- existing selection button remains unchanged -->
    {/each}
  </ul>
{/if}
```

Add `ragChat.searchingConversations` to both locale dictionaries with `Buscando conversaciones…` and `Searching conversations…`.

- [ ] **Step 7: Add per-row PDF download control**

Wrap the existing delete control in a row-actions container and add the download control before it:

```svelte
<div class="rag-chat__conversation-actions">
  <IconButton
    size="sm"
    label={$currentLocale && t('ragChat.downloadConversation')}
    title={$currentLocale && t('ragChat.downloadConversation')}
    disabled={downloadFeedback[conversation.id] === 'loading'}
    aria-busy={downloadFeedback[conversation.id] === 'loading' ? 'true' : undefined}
    onclick={() => void downloadConversation(conversation.id)}
  >
    <ActionIcon name="download" size={14} />
  </IconButton>
  <IconButton
    size="sm"
    class="rag-chat__conversation-delete"
    label={$currentLocale && t('ragChat.deleteConversation')}
    title={$currentLocale && t('ragChat.deleteConversation')}
    onclick={() => {
      pendingDeleteId = conversation.id
    }}
  >
    <ActionIcon name="delete" size={14} />
  </IconButton>
  {#if downloadFeedback[conversation.id] && downloadFeedback[conversation.id] !== 'loading'}
    <span class="rag-chat__action-feedback" role="status">
      {downloadFeedback[conversation.id] === 'success'
        ? ($currentLocale && t('ragChat.downloadedConversation'))
        : ($currentLocale && t('ragChat.downloadConversationError'))}
    </span>
  {/if}
</div>
```

The download button keeps the exact accessible label `Descargar conversación en PDF` (localized through the same key) and uses the row id only in its click callback. The click callback never calls `ragChat.select`.

- [ ] **Step 8: Add homogeneous styles and lifecycle cleanup**

Add styles to `RagChatView.svelte` using existing tokens and focus behavior:

```css
.rag-chat__sidebar-heading {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-2);
}

.rag-chat__conversation-search {
  width: 100%;
  box-sizing: border-box;
  margin-top: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-input);
  background: var(--surface-input);
  color: var(--color-text-primary);
  font: inherit;
}

.rag-chat__conversation-search:focus {
  outline: none;
  border-color: var(--color-accent);
  box-shadow: var(--focus-ring);
}

.rag-chat__message-actions,
.rag-chat__conversation-actions {
  display: flex;
  align-items: center;
  gap: var(--space-1);
}

.rag-chat__message-actions {
  align-self: flex-end;
}

.rag-chat__conversation-actions {
  flex-shrink: 0;
  flex-wrap: wrap;
  margin: var(--space-2) var(--space-2) 0 0;
}

.rag-chat__conversation-actions :global(.rag-chat__conversation-delete) {
  margin: 0;
}

.rag-chat__action-feedback {
  color: var(--color-text-secondary);
  font-size: var(--font-size-xs);
}
```

Add an `onDestroy` cleanup for pending timers:

```typescript
onDestroy(() => {
  if (copyFeedbackTimer) clearTimeout(copyFeedbackTimer)
  if (conversationSearchTimer) clearTimeout(conversationSearchTimer)
  for (const timer of downloadFeedbackTimers.values()) clearTimeout(timer)
})
```

Keep the existing `.rag-chat__conversation:hover`, active-row, selection-button, and focus styles intact.

- [ ] **Step 9: Run the focused view tests and commit Task 3**

Run:

```bash
pnpm --filter @entropia-pro/desktop test -- src/views/RagChatView.test.ts
```

Expected: PASS, including all pre-existing chat tests and the new action tests.

Commit:

```bash
git add apps/desktop/src/views/RagChatView.svelte apps/desktop/src/views/RagChatView.test.ts apps/desktop/src/lib/i18n.ts
git commit -m "feat(desktop): add research chat actions"
```

---

### Task 4: Run bounded integration checks and smoke scenarios

**Files:**
- No source changes expected.
- Review: all files from Tasks 1–3.

**Interfaces:**
- Consumes: the exact implementations and tests committed by Tasks 1–3.
- Produces: verification evidence for the approved chat-actions contract.

- [ ] **Step 1: Run the complete focused desktop chat/export test set**

Run:

```bash
pnpm --filter @entropia-pro/desktop test -- src/views/RagChatView.test.ts src/lib/rag-chat-export.test.ts
```

Expected: PASS for both files, including the existing selection/deletion/navigation regressions.

- [ ] **Step 2: Run the Lite frontend typecheck**

Run from the repository root:

```bash
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck
```

Expected: `svelte-check` exits successfully with no new diagnostics.

- [ ] **Step 3: Run the focused RAG Rust tests**

Run:

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml rag::store::tests --lib
```

Expected: PASS for the existing store tests and the new literal title/message search test.

- [ ] **Step 4: Launch the Lite desktop surface for a smoke test**

From `apps/desktop`, run:

```bash
VITE_LOCAL_ML=0 pnpm exec tauri dev --config src-tauri/tauri.lite.conf.json
```

Exercise the actual chat surface:

1. Open a conversation containing a multiline assistant answer and activate `Copiar respuesta`; verify the clipboard contains the complete multiline raw answer and the `Copiado` feedback appears briefly.
2. Open the conversation search button, enter a term that appears only in an answer, and verify the list filters while the currently rendered conversation and composer draft remain unchanged.
3. Activate the PDF button on a non-active conversation row; verify a PDF appears in the system Downloads directory and contains the title, questions, and answers in chronological order.
4. Select a conversation, delete a different conversation, open a cited source, and start a new conversation; verify all four existing flows remain functional.

- [ ] **Step 5: Review the final diff and record bounded evidence**

Run:

```bash
git diff HEAD~3..HEAD --stat
git status --short
```

Expected: only the planned chat action, search command, PDF export, capability, localization, tests, and committed design/plan files are present; no generated build output or unrelated source changes are included.

If all bounded checks pass, mark the verification task complete. If a check fails, correct only the affected task's implementation, rerun that focused check once, and then rerun Task 4 Step 1 before reporting the result.

---

## Plan Self-Review

- **Spec coverage:** Copy payload/feedback is covered by Task 3 Steps 1, 4, and 5. Search title/message scope, literal matching, ordering, empty query, stale-request protection, and active-state isolation are covered by Task 1 and Task 3 Steps 1, 4, and 6. PDF chronology, safe conversion, direct row id, native generator, Downloads output, capability scope, and failure propagation are covered by Task 2 and Task 3 Steps 1 and 7. UI consistency, localization, regression behavior, and verification are covered by Task 3 Steps 3, 8, and 9 and Task 4.
- **Type consistency:** Task 1's `ragSearchConversations(query: string)` output matches Task 3's import and the Tauri command returns `Vec<RagConversationSummary>`. Task 2's `downloadRagConversationPdf(conversationId: string)` output matches Task 3's row callback. `RagConversation` and `RagMessage` fields match `apps/desktop/src/lib/rag.ts`.
- **Scope:** No RAG schema migration, active-state rewrite, OCR adapter rewrite, new tooltip component, or unrelated UI refactor is planned.
- **Implementation ordering:** Each source task starts with a failing focused test, implements the smallest contract, runs its focused check, and commits before the next dependent task.
