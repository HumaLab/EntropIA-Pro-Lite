# Generate OCR PDFs with native text

Replace the raster-only `html2pdf.js` adapter with a semantic `pdfmake` adapter. Text in exported PDFs becomes selectable, searchable, copyable, and extractable while headings, inline formatting, lists, tables, links, and OCR-region images retain their document meaning.

## Decision

| Topic | Decision |
|---|---|
| PDF engine | Use `pdfmake` 0.3.x in the browser. |
| Source | Convert the existing prepared, sanitized HTML; do not parse unsanitized OCR input again. |
| Text | Emit native PDF text objects for every textual node. |
| Images | Keep generated OCR-region crops as embedded raster images. |
| Fonts | Bundle pdfmake's Roboto VFS so Spanish text and accents work offline. |
| Fidelity | Preserve semantic hierarchy and current visual intent, not pixel-identical browser rendering. |
| Other formats | Leave Markdown and DOCX generation unchanged. |

`html2pdf.js` cannot satisfy the contract because it delegates HTML rendering to `html2canvas`. The verified output looked correct but PDF.js found zero text items. `jsPDF.html()` uses the same canvas path. `pdf-lib` can draw native text, but would require the application to own line layout, pagination, lists, and table composition. `pdfmake` supplies those behaviors behind a smaller document-definition interface.

## Scope and invariants

- Every visible textual value in the prepared export must be represented as native PDF text.
- Text must remain selectable, searchable, copyable, and extractable through PDF.js.
- Generated OCR-region crops remain images; their alt description is not rendered as duplicate visible text.
- Existing source normalization, sanitization, crop resolution, fallback wording, save dialog, filename, and filesystem behavior remain unchanged.
- The `OcrExportFormat`, `PreparedOcrExport`, `OcrExportGenerators`, and `exportOcrText` interfaces remain unchanged.
- PDF generation stays browser-local and offline. No Tauri command or network request is introduced.
- Markdown and DOCX byte generation remain byte-for-byte on their existing paths.
- The change does not promise tagged-PDF accessibility or pixel-identical CSS reproduction.

## Module design

### `apps/desktop/src/lib/ocr-pdf.ts`

A new deep module owns semantic conversion and PDF generation behind two interfaces:

```typescript
export function buildOcrPdfDefinition(html: string): TDocumentDefinitions
export async function generateNativeOcrPdfBytes(html: string): Promise<Uint8Array>
```

`buildOcrPdfDefinition` is pure with respect to PDF generation. It parses the already-sanitized export HTML and returns a document definition suitable for structural tests. `generateNativeOcrPdfBytes` dynamically loads pdfmake plus its VFS fonts, registers the VFS once, creates the document, obtains a `Blob`, and returns its bytes.

### `apps/desktop/src/lib/ocr-export.ts`

The existing module continues to prepare shared Markdown and HTML. Its PDF adapter delegates the full prepared export HTML to `generateNativeOcrPdfBytes`. DOCX continues consuming the same HTML. The temporary off-screen DOM and `html2pdf.js` options disappear because semantic PDF generation does not render a browser canvas.

### Dependencies

- Add runtime dependency `pdfmake` at the current 0.3 release line.
- Add `@types/pdfmake` for document-definition and promise-based browser output types.
- Remove `html2pdf.js`.
- Remove only the obsolete `html2pdf.js` declaration from `apps/desktop/src/types/ocr-export-libraries.d.ts`; retain the DOCX declarations.
- Update `pnpm-lock.yaml` through pnpm, not by hand.

## Semantic conversion

The converter walks the sanitized DOM in document order. Unsupported wrapper elements are unwrapped rather than discarded, preserving their text.

| Sanitized HTML | pdfmake representation |
|---|---|
| Text node, `p`, `div`, `section` | Native text paragraph or stack |
| `h1`–`h6` | Native text with descending heading styles and page-break-safe margins |
| `strong`, `b` | Bold inline span |
| `em`, `i` | Italic inline span |
| `u` | Underlined inline span |
| `br` | Native newline inside the current text flow |
| `a[href]` | Inline text with the sanitized link target |
| `ul`, `ol`, `li` | Nested pdfmake unordered or ordered lists |
| `blockquote` | Indented stack with muted text and a left rule |
| `pre`, `code` | Preserved whitespace with code styling and wrapping |
| `table` family | pdfmake table body with borders, padding, header rows, and cell alignment |
| `caption` | Native caption text above the table |
| `th`, `td` spans | `colSpan`/`rowSpan` plus required placeholder cells in a rectangular grid |
| `img[data:]` | Embedded image constrained to available page width |
| OCR fallback `span` | Existing readable fallback text |

Inline descendants are accumulated into one native text array so emphasis, links, and line breaks do not split a paragraph unnecessarily. Block descendants become stacks. Empty structural wrappers do not create blank pages.

Table conversion tracks occupied grid coordinates created by row and column spans. It inserts empty placeholder cells where pdfmake requires them and derives a stable column count before emitting the table. Malformed residual table structure degrades to its textual content rather than aborting the entire export.

## Document styling

Use A4 portrait pages with 12 mm-equivalent margins, matching the current adapter. Define centralized pdfmake styles for:

- 11 pt body text with approximately 1.55 line height;
- six heading levels with dark text and decreasing sizes;
- paragraph/list/table spacing;
- muted, indented blockquotes;
- shaded code blocks;
- bordered table headers and cells;
- images fitted within the writable page width.

Roboto normal, medium, italic, and medium-italic variants are loaded from pdfmake's bundled VFS. This keeps generation offline and preserves Spanish characters such as `á`, `é`, `í`, `ó`, `ú`, `ü`, and `ñ` in both rendering and extraction.

## Data flow

1. `prepareOcrExport` produces sanitized HTML and resolves OCR-region references to safe data-URI images.
2. `generateOcrExportBytes('pdf', ...)` passes the same full export HTML used by DOCX to the native PDF adapter.
3. The semantic converter creates a typed pdfmake document definition.
4. pdfmake lays out native text, tables, lists, links, and images across pages.
5. The generated `Blob` becomes a `Uint8Array` and is written through the existing save path.

## Failure behavior

- Dynamic import, font registration, semantic conversion, image decoding, or PDF generation failures reject the existing export promise.
- The existing UI reports the localized export error and writes no partial file.
- Missing or malformed supported structure degrades to readable native text where possible.
- No raster fallback is allowed: silently returning an image-only PDF would violate the contract.

## Verification

### Red-capable contract test

Generate a real PDF containing accented text, a heading, formatted inline text, a list, a table, a link, and one data-URI image. Load the bytes with the existing `pdfjs-dist` dependency and assert that normalized `getTextContent()` output contains every visible textual value. Against the current `html2pdf.js` adapter this fails with zero text items.

### Structural tests

Test `buildOcrPdfDefinition` directly for:

- heading and paragraph order;
- bold, italic, underline, links, and line breaks;
- nested ordered and unordered lists;
- blockquotes and whitespace-preserving code blocks;
- table headers, captions, `rowSpan`, `colSpan`, and placeholders;
- image fit constraints and fallback text;
- safe unwrapping of unsupported wrappers.

### Regression and surface checks

- Existing Markdown and DOCX adapter tests continue passing.
- Save cancellation and write behavior remain unchanged.
- Lite frontend typecheck passes.
- A Chromium smoke test opens the generated PDF, visually confirms the layout and embedded crop, then confirms text selection/extraction through PDF.js.

## Non-goals

- Pixel-perfect reproduction of browser CSS.
- Tagged-PDF/UA conformance, forms, annotations, headers, footers, or a table of contents.
- OCR text hidden behind source-page images.
- Remote fonts, remote images, or any network-dependent export step.
- Changes to OCR persistence, Markdown export, DOCX export, or the download menu.
