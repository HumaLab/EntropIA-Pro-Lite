# Writing Image Node Design

## Goal

A writer inserts an image into the manuscript from the Escritura toolbar, sees it laid out inside the document margins, selects it, resizes it, optionally captions it, saves, reopens after restarting the application, and exports it to Markdown, HTML, and DOCX without a broken reference.

An inserted image stops depending on the path it came from. Moving, renaming, or deleting the original file on disk does not change the manuscript.

## Scope

One new block node, one new managed store for its bytes, and registration of that node across the three exporters and their guards.

Affected units:

- `packages/ui/src/components/WritingEditor/extensions.ts`: the `writingImage` node, its node view, and its paste/drop plugin, joining the three custom nodes already defined there (`extensions.ts:45-260`).
- `packages/ui/src/components/WritingEditor/WritingEditor.svelte`: the toolbar entry in the existing `insert` group (`WritingEditor.svelte:766-787`) and the insert command beside the existing citation commands (`WritingEditor.svelte:309-377`).
- `packages/ui/src/components/Button/ActionIcon.types.ts` and `ActionIcon.svelte`: one new icon name.
- `apps/desktop/src/lib/writing-images.ts` (new): content-addressed import and path resolution for manuscript images.
- `apps/desktop/src/lib/image-dimensions.ts` (new): intrinsic size decoded from file bytes, extracted from the decoders currently private to `export-images.ts:114-152`.
- `apps/desktop/src/lib/export-images.ts`, `export-html.ts`, `export-markdown.ts`, `export-docx.ts`, `export-fidelity.ts`, and `i18n.ts`.

Not in scope: the `assets` table and the investigation/OCR asset subsystem, the writing journal, document versioning, and `WRITING_SCHEMA_VERSION`. Citation quote images keep their present behaviour; the only change reaching them is the extraction of the byte-header dimension decoder into a shared module, which must leave their output identical.

## Node Shape

`writingImage` is a block node whose content is its caption:

    group: 'block'
    content: 'inline*'
    draggable: true
    selectable: true
    attrs: { src, alt, title, width, height, align }

`renderHTML` returns `['figure', { 'data-align': align }, ['img', { src, alt, title, width }], ['figcaption', 0]]`. The content hole sits in the caption, so the caption is ordinary document text: it is searched, counted, selected, and undone character by character like any other prose.

`parseHTML` matches only `figure[data-writing-image]`, which is what this node itself emits, so copying an image within or between manuscripts round-trips exactly. It deliberately does not match a bare `img[src]`. HTML pasted from elsewhere carries remote or foreign-filesystem sources, and admitting one would create a node whose bytes the archive does not hold — the precise failure this design exists to prevent. Such an `img` is handled by the paste plugin when its bytes can be obtained and imported, and is otherwise dropped rather than stored as a reference.

The caption is optional. An empty `figcaption` is collapsed while the figure is not selected and reveals a muted placeholder line while it is, so an uncaptioned image costs no vertical space and a writer who wants a caption can always reach one.

Choosing a content-bearing figure rather than an atom is what keeps the caption cheap. An atom node would have to change its content expression later to accept editable caption text, and that is a migration of every stored manuscript.

`src` holds a path relative to the shared data directory, never an absolute path. `width` is the author's chosen width in CSS pixels; `height` records the intrinsic aspect only and is never used to stretch the image. `align` is `left`, `center`, or `right`, defaulting to `center`.

## Managed Image Storage

`writing-images.ts` exposes one import function that every entry path uses:

1. Read the bytes. The file picker yields a path, read through `@tauri-apps/plugin-fs`; the clipboard and a drop yield bytes directly.
2. Hash the bytes with `crypto.subtle.digest('SHA-256', ...)` and render the digest as lowercase hex, matching the convention already used on the Rust sync side (`apps/desktop/src-tauri/src/sync/blobs.rs:1-80`).
3. Derive the extension from the detected media type, never from the original filename.
4. Write to `{dataDir}/writing-images/{sha256}.{ext}` only when that path does not already exist.
5. Return the relative path `writing-images/{sha256}.{ext}`.

Content addressing is what satisfies both the stable-identifier and the avoid-needless-copies requirements with one mechanism. The same image inserted twice, in one manuscript or in two, is one file, because the name is the content. There is no index to keep in step and nothing to reconcile after a crash.

This follows the storage pattern of `writing-crops.ts:69-94`, which already copies quote images into the data directory, and corrects its one weakness: that module names files with `crypto.randomUUID()` and therefore stores the same bytes as many times as they are quoted.

On screen the relative path is resolved by the `resolveImage` option the editor already receives and threads into `createWritingExtensions` (`extensions.ts:279-323`), which resolves through `getAssetUrl` (`apps/desktop/src/lib/file-import.ts:302-304`) to the `asset:` protocol. That protocol is already enabled, with the same scope, in both variants (`apps/desktop/src-tauri/tauri.conf.json:32-35`, `tauri.lite.conf.json:7-10`), and `fs` and `dialog` permissions are already granted (`apps/desktop/src-tauri/capabilities/default.json:1-27`). No Tauri configuration changes.

Stored images are never deleted. `writing_document_versions` keeps whole-content snapshots, so a file collected when an image is removed from the current revision would leave a hole in every earlier version that still references it. Content addressing keeps the cost of an orphan to one copy of bytes the archive once held. Reclaiming them is a separate decision with its own evidence, not a side effect of an edit.

## Entry Paths

Three ways in, one mechanism:

- The toolbar button opens `@tauri-apps/plugin-dialog`'s `open()` filtered to the accepted formats.
- A clipboard paste carrying image data is handled by a ProseMirror plugin on the node.
- A file dragged from the operating system onto the editor is handled by the same plugin.

All three call the same import function and the same insert command, so storage, deduplication, and node construction have one implementation. The plugin claims an event only when it actually carries image data; every other paste and drop falls through untouched.

Accepted formats are PNG, JPEG, WebP, and GIF.

SVG is excluded. An SVG is an executable document rather than a bitmap, and the DOCX writer cannot embed it with the same reliability as the raster formats. The requirement admitted SVG only if it could be handled safely under the current architecture, and that cannot be asserted here today.

## Node View and Resizing

`writingImage` introduces the first `addNodeView` in this repository. It is a plain ProseMirror DOM node view — `dom` is the `figure`, `contentDOM` is the `figcaption` — and deliberately not a Svelte node view, so it adds no rendering dependency and leaves the existing nodes' `renderHTML`-only convention intact for everything that does not need interaction.

Selecting the figure reveals corner handles. Dragging a handle changes `width` through a transaction, so resizing participates in undo and redo like any other edit. The aspect ratio is fixed, the width is floored at a minimum that keeps the image and its handles usable, and it is capped at the width of the editor's content area.

That cap is the one measurement this feature takes from live layout, and it is taken only while a handle is being dragged, which cannot happen outside a real browser. The clamping arithmetic is a pure function of the dragged width and the available width, so it is tested directly with numbers; no test drags a handle or reads a rendered size.

Alignment and the `alt` and `title` attributes are edited from a selection bubble built with the existing `ToolbarMenu` vocabulary rather than a permanent panel.

`selectable: true` gives node selection, and with it delete, backspace, cut, and copy, from ProseMirror itself. One keymap addition is required: backspace at the start of an empty caption selects the figure instead of joining it into the preceding block.

## Rendering and Layout

The stylesheet, not the stored attribute, is what guarantees the layout holds. The image carries `max-width: 100%` and `height: auto`, so a stored width larger than the column — from a narrower window, a different variant, or a hand-edited document — is clamped on render instead of overflowing. The figure stays inside the document margins and inherits theme tokens like every other block, so no new visual style is introduced.

Intrinsic dimensions are decoded from the file's header bytes by `image-dimensions.ts`, never by measuring a mounted element. This is the discipline `export-images.ts:114-152` already follows for PNG and JPEG; that decoder moves into the shared module and gains WebP and GIF, and `export-images.ts` consumes it from there. Measuring the DOM instead would also be untestable: in happy-dom an `<img>` never fires `load` and reports a natural size of zero, so a test that waited on it would pass while proving nothing.

## Serialization and Compatibility

Content is stored as canonical TipTap JSON in `writing_documents.current_content_json`, wrapped as `{ schemaVersion, doc }` (`packages/ui/src/components/WritingEditor/document-contract.ts:29-32`). A `writingImage` node serializes as its attributes plus, when captioned, its inline content. Nothing else about the envelope changes.

`WRITING_SCHEMA_VERSION` is not bumped. Adding a node type does not change the envelope's shape, which is what that number describes (`extensions.ts:292-293`), and bumping it would make every future save unreadable to an older build even for manuscripts that contain no image at all. The narrower consequence is the correct one: an older build opening a manuscript that does contain an image is refused by `classify()` as an unknown node (`document-contract.ts:83-94`) rather than shown blank. Failing closed on unreadable content is the existing contract, and it is preserved.

Existing manuscripts are untouched. They contain no `writingImage` node, so no stored byte changes and no migration runs. Citations, footnotes, note links, tables, links, text styles, search, and the delta journal are all unaffected: the node is additive and shares no attribute namespace with them.

## Exports

Every exporter walks the canonical JSON with its own `block`/`inline` switch and ends in `default: return children()` (`export-html.ts:253`, `:178`). An unregistered node does not throw there; the traversal recurses into its content and drops only the node's own wrapper and attributes. For `writingImage` that failure is quiet and lossy in a particular way: the caption would survive as a bare paragraph and the image, which lives in the attributes, would vanish without a trace. Each exporter therefore gets an explicit case, and a test asserts the image is present rather than asserting the export merely succeeded.

`export-images.ts:38-58` currently collects only `documentCitation.attrs.quotedParts` image sources. It is extended to also collect `writingImage.attrs.src`, so all three exporters receive the bytes through the `ExportContext.images` record they already share (`export-document.ts:48-72`). This is the reuse the feature needs; no second image-loading path is added.

- **DOCX** embeds the bytes through `ImageRun`, which is already written and used for quote images (`export-docx.ts:420-425`), sized by the existing column-fitting helper (`export-docx.ts:405-417`) so the author's width is honoured up to the page column. The caption is emitted as the paragraph following the image.
- **HTML** emits a `figure` with the image as a `data:` URI, matching how quote images already travel.
- **Markdown** emits `![alt](data:...)` followed by the caption as a paragraph. This is the policy the Markdown exporter already states for quote images at `export-markdown.ts:137-141`: a Markdown file is one file, and a path into the archive would break the moment the file left this machine. Reusing it keeps one rule in the exporter instead of two.

## Export Guards

`supportOfNode` treats anything absent from its table as unsupported (`export-fidelity.ts:185-189`), so `writingImage` is registered in `NODE_FIDELITY` as native in all three formats.

Two guard tests follow from that registration and are updated in the same change rather than afterwards:

- `export-vocabulary.test.ts:36-46` requires a `writing.exportElement.*` label in both locales for any element that is ever a fallback or unsupported, and for anything in `REQUIRED_BY_SPEC`. The label is added to both locales in `i18n.ts`.
- `export-pattern.test.ts:276-280` asserts that a DOCX export of the canonical pattern document warns about `noteLink` and nothing else. The pattern document gains a captioned, resized image, and that assertion is re-verified against it.

`REQUIRED_BY_SPEC` is left alone. Listing the node there would make `losesRequiredElement` (`export-fidelity.ts:300-304`) refuse a whole DOCX export on any downgrade, and an image is not an element whose absence should void the manuscript.

## Failure Handling

- Import failure inserts nothing. A node is never created pointing at bytes that were not written.
- An unsupported or unreadable file is reported and the document is unchanged.
- A stored file missing at render time draws a placeholder in place of the image. The node is kept: the manuscript records that an image belongs there, and silently dropping it would destroy content the writer did not delete.
- A stored file missing at export time produces a warning through the existing fidelity-warning channel. It is not silently skipped.
- Caption text survives every one of these paths, because it is document content rather than an attribute of the file.

## Non-Goals

Garbage collection of unreferenced stored images. Image cropping or rotation inside the editor. Remote image URLs. A per-document image table. Any change to the investigation `assets` table or to `findByItem`-based asset resolution, which is an item-and-OCR subsystem with different ownership and lifetime rules.

## Verification

TDD covers these observable contracts:

1. The insert command creates a `writingImage` node at the cursor and an existing text selection is preserved rather than replaced.
2. A document containing the node serializes to canonical JSON and reloads into an equal document, caption and attributes included.
3. Importing the same bytes twice yields one stored file and one path.
4. The stored path is relative and contains no drive letter or home directory.
5. Deleting a selected image removes the node; undo restores it with its width, alignment, alt text, and caption; redo removes it again.
6. A resize transaction changes `width` and is undone as a single step.
7. A stored width wider than the column does not widen the rendered figure, and the clamping function refuses a negative, zero, or sub-minimum width and never returns more than the available width.
8. Pasting HTML that carries a remote or foreign-filesystem `img` never produces a node holding that source: the bytes are imported, or nothing is inserted.
9. Intrinsic dimensions are decoded from header bytes for PNG, JPEG, WebP, and GIF.
10. HTML export emits a figure with an embedded image and its caption.
11. Markdown export emits an embedded image and its caption, with no filesystem path in the output.
12. DOCX export embeds the image bytes and emits the caption, and the pattern document's DOCX warnings remain exactly `noteLink`.
13. A manuscript with no image serializes byte-identically to its pre-change serialization.
14. A missing stored file leaves the node in the document and produces a warning on export.

Beyond the suite, the feature is complete only after the full path is exercised by hand in the running application: open a document, place the cursor, insert from the toolbar, see the image, resize it, caption it, save, leave the document, reopen it, close EntropIA, start it again, reopen the document, confirm image, dimensions, alignment, and caption, then export to all three formats and open the results. Paste and drag-and-drop are exercised the same way.

Completion evidence is the focused `packages/ui` and `apps/desktop` suites, the Svelte autofixer, the Lite frontend typecheck under `VITE_LOCAL_ML=0`, and that manual run. Because the editor is a native Tauri window, the visual and interaction checks are confirmed by the user, not asserted here.
