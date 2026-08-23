# OCR Rich Text Viewer Design

## Goal

Render the GLM OCR output as rich content only in the left-panel `Texto extraído` tab of the main item viewer. The display must support mixed Markdown and basic HTML, sanitize untrusted HTML, and resolve OCR image-region references such as `![](page=0,bbox=[536,508,1507,1112])` against the selected source asset without changing the stored OCR text.

## Scope and invariants

- Target surface: `apps/desktop/src/views/ItemAssetPanel.svelte`, the non-audio `left-panel-text` pane.
- The `Documento` pane and its `DocumentViewer` props/behavior remain unchanged.
- The right-panel `ItemTextPanel.svelte` and its editable text area remain unchanged.
- OCR state, extraction persistence, backend payloads, layout persistence, and source assets remain unchanged.
- The source string (`ocrEditedText`) is never rewritten. Rich output is derived in memory only.
- Failed image resolution preserves the original OCR image reference as escaped text at its original position.
- `page` is zero-based. `bbox` is `[left, top, right, bottom]` in the OCR/layout coordinate space.

## Architecture

Add an OCR-specific frontend renderer rather than changing `apps/desktop/src/lib/markdown.ts`. The existing renderer is shared by RAG output and escapes raw HTML, so extending it would create unrelated behavior changes.

The new renderer consists of:

1. `apps/desktop/src/lib/ocr-rich-text.ts`
   - Markdown-it configuration and custom image-token extraction.
   - Safe HTML sanitization for the OCR display.
   - Bbox parsing, coordinate scaling, and crop-source helpers.
2. `apps/desktop/src/components/OcrRichText.svelte`
   - Owns asynchronous page/image loading and render-generation cancellation.
   - Produces sanitized HTML for `{@html}` only after all image placeholders are replaced.
3. `apps/desktop/src/views/ItemAssetPanel.svelte`
   - Replaces the literal OCR interpolation with `OcrRichText`.
   - Keeps transcription rendering and all document-viewer markup intact.
4. `apps/desktop/src/views/ItemView.svelte`
   - Passes the existing source URL/type and persisted layout reference dimensions to `ItemAssetPanel` for display only.
5. `apps/desktop/package.json` and `pnpm-lock.yaml`
   - Declare `markdown-it` as a direct desktop dependency; it is already present transitively but must not be relied on as an undeclared import.

## Markdown and HTML pipeline

Configure Markdown-it with `html: true`, `breaks: true`, `linkify: false`, and table support enabled. Markdown-it handles headings, emphasis, strong text, paragraphs, line breaks, lists, and GFM tables. Raw HTML such as `<div align="center">` remains in the token stream; blank-line-separated Markdown inside the block is parsed as Markdown.

The image renderer recognizes the OCR region syntax instead of treating it as a network URL. It creates an internal placeholder and records the parsed page, bbox, and original reference. Malformed syntax remains ordinary text.

Before binding the result with `{@html}`, walk the generated DOM through an OCR-specific allowlist sanitizer:

- Allowed elements: paragraphs, `h1`–`h6`, `div`, `section`, `blockquote`, `br`, `strong`, `b`, `em`, `i`, `u`, `code`, `pre`, `ul`, `ol`, `li`, `table`, `caption`, `thead`, `tbody`, `tfoot`, `tr`, `th`, and `td`, plus generated `img` elements.
- Allowed attributes: `align` with `left|center|right`; safe `href` values (`http`, `https`, `mailto`, `tel`, and fragment links); bounded numeric `colspan`/`rowspan`; `scope`; generated image `src`/`alt`.
- Strip scripts, styles, iframes, objects, embeds, forms, event-handler attributes, unsafe URLs, and unknown attributes. Do not preserve arbitrary CSS.
- Convert failed placeholders to escaped original-reference text before the final sanitization pass.

## Image-region resolution

`OcrRichText` receives `viewerSrc`, `viewerType`, and the layout reference dimensions already derived by `ItemView`. Resolution is display-only:

### Image assets

Load `viewerSrc` into an `HTMLImageElement`. Use the natural source dimensions as the crop surface. Convert the OCR bbox proportionally from the reference dimensions; when no reference dimensions exist, use the natural image dimensions. Draw the validated crop to an offscreen canvas and encode it as a temporary PNG data URL.

### PDF assets

Load `viewerSrc` with the same `pdfjs-dist` package and worker strategy already used by `DocumentViewer`. For each referenced page, call `getPage(page + 1)` and render the page to an offscreen canvas at a scale matching the OCR reference dimensions when available. Convert and crop the bbox in that rendered coordinate space. Cache the rendered page by source URL and zero-based page index so multiple regions on one page share one render.

The implementation must reject non-finite values, empty or reversed boxes, invalid page indexes, and out-of-range coordinates. A failed load/render/crop returns the original reference as text. A monotonically increasing render generation prevents stale asynchronous crops from replacing content after an asset or OCR text change. No crop is written to disk or persisted as an asset.

GLM normally creates per-page PNG child assets for scanned PDFs; those assets are selected as ordinary image assets, so PDF.js is only the fallback for a selected PDF source.

## Component behavior and styling

The existing empty-state and OCR metadata remain unchanged. The rich body keeps the existing scrolling container and theme tokens, replacing `white-space: pre-wrap` with semantic content styles for headings, paragraphs, lists, tables, code, generated images, and centered HTML blocks. Generated images receive responsive sizing, block/inline flow consistent with their source position, and descriptive alt text such as `OCR region from page 1`.

Only the non-audio OCR branch uses `OcrRichText`. Audio transcription remains plain text in the same tab.

## Verification

Add focused tests in `apps/desktop/src/lib/ocr-rich-text.test.ts` for:

- headings, bold, italics, paragraphs, soft line breaks, lists, and Markdown tables;
- mixed HTML/Markdown including centered `div` blocks;
- removal of script/style/handler/unsafe-link injection;
- parsing zero-based page and `[left, top, right, bottom]` bboxes;
- coordinate scaling and invalid-box rejection;
- successful placeholder replacement and textual fallback.

Add a focused component regression for `ItemAssetPanel` (or its renderer seam) proving rich OCR output appears in the left text pane, while the document pane and right-panel editable text path remain unchanged.

Run the targeted Vitest tests, Lite typecheck (`VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck`), and a UI smoke check against the main viewer with Markdown/HTML/table content plus an inline crop. Confirm that the persisted extraction text remains equal to the source fixture after rendering.

## Non-goals

- No changes to Rust OCR parsing or API requests.
- No changes to OCR/database schemas or stored content.
- No changes to the shared Markdown renderer, `DocumentViewer`, or right-panel text editor.
- No support for arbitrary remote images beyond generated source-asset crops.
