# OCR Extracted Text Actions and Export Design

## Goal

Add compact Copy and Download actions to the header of the left-panel `Texto extraído` tab. Copy must place the complete OCR source string on the clipboard without changing Markdown, HTML, whitespace, or OCR state. Download must offer Markdown, PDF, and DOCX exports derived from the complete rendered OCR content, including generated `page/bbox` inline crops, while leaving every other tab and OCR persistence path unchanged.

## Scope and invariants

- Target surface: `apps/desktop/src/views/ItemAssetPanel.svelte`, only the non-audio `left-panel-text` pane.
- The existing `via <method> · <character count>` metadata remains unchanged.
- `Documento`, its `DocumentViewer`, the right-panel `ItemTextPanel.svelte`, transcription rendering, and other tabs remain unchanged.
- Copy uses the exact `ocrEditedText` string. It must not trim, normalize, render, sanitize, or rewrite the source.
- OCR state, extraction persistence, backend payloads, layout persistence, and source assets remain unchanged.
- Export resolution is display/export-only. Generated crops are kept in memory and are never persisted as OCR or asset data.
- Valid `page/bbox` references are embedded in generated exports. Failed references become the readable marker `Imagen OCR no disponible` at their original position.
- `page` remains zero-based and `bbox` remains `[left, top, right, bottom]` in the existing OCR/layout coordinate space.

## User experience

The OCR metadata header becomes a layout row containing the existing title, metadata, and a compact action group aligned to the end of the row:

- Copy: existing `IconButton` with `size="sm"`, the existing `copy` `ActionIcon`, accessible label, and tooltip.
- Download: existing `IconButton` with `size="sm"`, the existing `download` `ActionIcon`, accessible label, tooltip, and active state while its menu is open.

The buttons are icon-only visually, with accessible names. The download menu is local to the OCR header, uses `role="menu"` and `role="menuitem"`, and follows the menu positioning, border, surface, focus-ring, and spacing conventions already used by the application. It contains exactly:

1. Documento Markdown (.md)
2. Documento PDF (.pdf)
3. Microsoft Word (.docx)

The menu closes after selection, on Escape, on outside pointer interaction, or when the user changes tab or asset. Copy and export feedback is local to this surface. Export actions are disabled while an export is being generated so concurrent jobs cannot write partial or conflicting files.

## Architecture

Add an OCR export module rather than extending the shared Markdown renderer:

1. `apps/desktop/src/lib/ocr-export.ts`
   - Defines the export input and prepared-document types.
   - Builds Markdown and sanitized visual HTML from the source OCR string.
   - Reuses `renderOcrMarkup`, `sanitizeOcrHtml`, and `resolveOcrRegion` from `ocr-rich-text.ts`.
   - Replaces valid OCR-region placeholders with embedded PNG data URIs and failed placeholders with readable fallback content.
   - Provides format-specific byte generation and native save helpers.

2. `apps/desktop/src/views/ItemAssetPanel.svelte`
   - Owns the menu, copy action, export action state, and local feedback.
   - Passes the existing asset URL, source type, and layout reference dimensions into the export module.
   - Does not modify OCR text or introduce actions in the document/transcription branches.

3. `apps/desktop/src/lib/i18n.ts`
   - Adds typed Spanish and English labels, statuses, and error messages for the two actions, three formats, and fallback marker.

4. `apps/desktop/package.json` and `pnpm-lock.yaml`
   - Add direct frontend dependencies for browser-side PDF and DOCX generation.
   - The planned generators are `html2pdf.js` for PDF and `html-docx-js` for DOCX. Both consume the same prepared HTML and support browser execution with embedded data-URI images.

No Rust command, database schema, OCR payload, or shared UI component API change is required.

## Shared export preparation pipeline

The export module receives:

- `source`: the exact `ocrEditedText` value;
- `assetUrl`: the current `viewerSrc`;
- `sourceType`: `image` or `pdf`;
- `referenceWidth` and `referenceHeight`: the current layout reference dimensions.

Preparation proceeds as follows:

1. Normalize line endings only inside the derived export representation, matching `renderOcrMarkup` behavior. The source and clipboard value are never normalized.
2. Parse the source through `renderOcrMarkup`, which extracts valid OCR region references while preserving other Markdown and HTML content.
3. Resolve all extracted references through the existing `resolveOcrRegion` crop pipeline. Image and PDF assets therefore use exactly the same coordinate validation, rasterization, scaling, and in-memory data-URI generation as the viewer.
4. For a successful reference, create both representations:
   - Markdown image syntax with an embedded PNG data URI and descriptive alt text;
   - sanitized HTML `<img>` with the same data URI and descriptive alt text.
5. For a failed reference, create a readable textual/visual marker rather than silently dropping the source position.
6. Sanitize the completed visual HTML with the existing OCR-specific sanitizer. No arbitrary source CSS, scripts, event attributes, unsafe URLs, or unsupported elements are admitted into generated documents.
7. Return one prepared document consumed by all three format adapters. This prevents PDF and DOCX from diverging in content or crop resolution.

Ordinary non-OCR image syntax continues to follow the existing OCR renderer behavior. The export feature guarantees generated `page/bbox` crops; it does not introduce a new remote-image fetch policy.

## Format adapters

### Markdown

The `.md` adapter uses the derived Markdown representation as UTF-8 text. It preserves all original Markdown and HTML except for the synthetic OCR region references, which become self-contained embedded images. No companion image files are required. The adapter writes the bytes through the existing Tauri save dialog and filesystem APIs with an `.md` filter.

### PDF

The `.pdf` adapter converts a temporary, offscreen export container containing the prepared HTML with `html2pdf.js`. It applies an export-only light stylesheet:

- white page background and dark text;
- readable system sans-serif typography;
- stable heading and paragraph spacing;
- visible table borders and cell padding;
- distinct preformatted/code blocks;
- responsive images constrained to the page content width;
- page margins and CSS-aware page breaks;
- avoidance of splitting images and table rows where supported.

The generated PDF bytes are passed to the native save dialog and written with `writeFile`. The temporary container is removed after success or failure.

### DOCX

The `.docx` adapter passes the same prepared, sanitized HTML and export stylesheet-compatible structure to `html-docx-js`. All generated OCR images are already base64 data URIs, preventing broken links in Word. The returned browser `Blob` is converted to `Uint8Array`, then saved through the native dialog and filesystem APIs with a `.docx` filter.

The HTML structure is intentionally limited to headings, paragraphs, lists, blockquotes, tables, code blocks, links, and images so Word rendering remains predictable. The DOCX adapter does not attempt to reproduce application theme colors or arbitrary source CSS.

## State and error handling

`ItemAssetPanel.svelte` maintains local state for:

- `downloadMenuOpen`;
- `copyFeedback`;
- `exportingFormat`;
- `exportError`.

When the selected asset changes, the menu and transient status state reset. A successful copy reports localized feedback. Clipboard rejection reports a localized error without fallback mutation. A canceled save dialog returns normally without writing. Any crop, conversion, or file-write failure reports an error, cleans up temporary DOM state, re-enables the actions, and leaves OCR content untouched.

Asynchronous export operations capture the current asset/source identity before starting. If the asset changes while an operation is pending, stale completion must not update the new asset's status. The same generation/guard approach already used by `OcrRichText` should be reused or factored into the export orchestration.

## Accessibility and interaction checks

- Buttons expose localized `aria-label` values and tooltips.
- The download trigger exposes `aria-expanded` and `aria-controls` while the menu is open.
- Menu items are keyboard reachable and activate with Enter/Space.
- Escape closes the menu and returns focus to the trigger.
- Focus-visible styling uses the existing focus-ring token.
- Disabled/exporting states are conveyed by the native disabled attribute and localized status text where applicable.
- The menu does not trap focus or interfere with the surrounding tab list.

## Verification

Add focused tests for:

- exact clipboard input, including Markdown, HTML, CRLF, and trailing whitespace;
- menu presence and three option labels only within the extracted-text pane;
- menu dismissal and disabled state during export;
- Markdown/HTML preservation during preparation;
- valid page/bbox replacement with embedded data-URI images;
- readable fallback markers for rejected or failed references;
- PDF and DOCX adapters consuming the same prepared HTML;
- save cancellation and failure paths without OCR mutation;
- Spanish and English translation keys.

Run the focused desktop Vitest tests, the Lite frontend typecheck (`VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck`), and a surface smoke check that opens an asset with mixed Markdown/HTML and at least one valid page/bbox reference. Confirm visually that the two compact header actions and three-item menu match existing application styling, and confirm the copied/source OCR value remains unchanged after all export operations.

## Non-goals

- No changes to OCR storage, extraction requests, Rust OCR code, or asset schemas.
- No changes to the `Documento` tab, `DocumentViewer`, right-panel text editor, transcription output, or shared Markdown rendering.
- No arbitrary remote-image downloading or new asset persistence.
- No batch export, collection export, print preferences, or export history.
