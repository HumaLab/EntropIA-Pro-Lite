# Writing Image Node Design

## Goal

A writer inserts an image into the manuscript from the Escritura toolbar, sees it laid out inside the document margins, selects it, resizes it, optionally captions it, saves, reopens after restarting the application, and exports it to Markdown, HTML, and DOCX without a broken reference.

An inserted image stops depending on the path it came from. Moving, renaming, or deleting the original file on disk does not change the manuscript.

## Scope

One new block node, one new managed store for its bytes, and registration of that node across the three exporters and their guards.

Affected units:

- `packages/ui/src/components/WritingEditor/extensions.ts`: the `writingImage` node, its node view, and its paste/drop plugin, joining the three custom nodes already defined there (`extensions.ts:45-260`).
- `packages/ui/src/components/WritingEditor/trailing-paragraph.ts`: `writingImage` joins the `TRAPPING` set (`trailing-paragraph.ts:20-21`) so a manuscript ending in an image still has somewhere for the caret to land.
- `packages/ui/src/components/WritingEditor/WritingEditor.svelte`: the toolbar entry in the existing `insert` group (`WritingEditor.svelte:766-787`) and the insert command beside the existing citation commands (`WritingEditor.svelte:309-377`).
- `packages/ui/src/components/Button/ActionIcon.types.ts` and `ActionIcon.svelte`: one new icon name.
- `apps/desktop/src/lib/writing-images.ts` (new): content-addressed import and path resolution for manuscript images.
- `apps/desktop/src/lib/image-dimensions.ts` (new): intrinsic size decoded from file bytes, extracted from the decoders currently private to `export-images.ts:114-152`.
- `apps/desktop/src/lib/export-images.ts`, `export-html.ts`, `export-markdown.ts`, `export-docx.ts`, and `export-fidelity.ts`.

Not in scope: the `assets` table and the investigation/OCR asset subsystem, the writing journal, document versioning, and `WRITING_SCHEMA_VERSION`. Citation quote images keep their present behaviour; the only change reaching them is the extraction of the byte-header dimension decoder into a shared module, which must leave their output identical.

## Node Shape

`writingImage` is a block node whose content is its caption:

    group: 'block'
    content: 'inline*'
    draggable: true
    selectable: true
    attrs: { src, alt, title, width, height, align }

`renderHTML` returns `['figure', { 'data-writing-image': '', 'data-align': align }, ['img', { src, alt, title, width }], ['figcaption', 0]]`. The content hole sits in the caption, so the caption is ordinary document text: it is searched, counted, selected, and undone character by character like any other prose.

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

No code in this repository currently calls `crypto.subtle`, so this is a first-of-its-kind dependency for the project, and its availability in the `packages/ui`/`apps/desktop` Vitest `happy-dom` environment is not assumed here in either direction. The first implementation step exercises the hashing entry point directly in that test environment. If `crypto.subtle` proves unavailable there, the import function ships with a declared, tested fallback hash implementation instead of shipping on an untested assumption.

Content addressing is what satisfies both the stable-identifier and the avoid-needless-copies requirements with one mechanism. The same image inserted twice, in one manuscript or in two, is one file, because the name is the content. There is no index to keep in step and nothing to reconcile after a crash.

This follows the storage pattern of `writing-crops.ts:69-94`, which already copies quote images into the data directory, and corrects its one weakness: that module names files with `crypto.randomUUID()` and therefore stores the same bytes as many times as they are quoted.

On screen the relative path is resolved by the `resolveImage` option the editor already receives and threads into `createWritingExtensions` (`extensions.ts:279-323`), which resolves through `getAssetUrl` (`apps/desktop/src/lib/file-import.ts:302-304`) to the `asset:` protocol. That protocol is already enabled, with the same scope, in both variants (`apps/desktop/src-tauri/tauri.conf.json:55-58`, `tauri.lite.conf.json:7-10`), and `fs` and `dialog` permissions are already granted: `fs:scope` (`capabilities/default.json:1-27`), the write permissions `fs:allow-mkdir`, `fs:allow-exists`, `fs:allow-write-file`, `fs:allow-remove` (`capabilities/default.json:30-33`), and `dialog:allow-open`, `dialog:allow-save` (`capabilities/default.json:34-35`). No Tauri configuration changes.

Stored images are never deleted. `writing_document_versions` keeps whole-content snapshots, so a file collected when an image is removed from the current revision would leave a hole in every earlier version that still references it. Content addressing keeps the cost of an orphan to one copy of bytes the archive once held. Reclaiming them is a separate decision with its own evidence, not a side effect of an edit.

## Entry Paths

Three ways in, one mechanism:

- The toolbar button opens `@tauri-apps/plugin-dialog`'s `open()` filtered to the accepted formats.
- A clipboard paste carrying image data is handled by a ProseMirror plugin on the node.
- A file dragged from the operating system onto the editor is handled by the same plugin.

All three call the same import function and the same insert command, so storage, deduplication, and node construction have one implementation. The plugin claims an event only when it actually carries image data; every other paste and drop falls through untouched.

Accepted formats are PNG, JPEG, and GIF.

WebP is excluded. Every accepted format must be one the DOCX writer can draw: `drawnImage` (`export-docx.ts:420-426`) returns `null` for any media type absent from `DOCX_IMAGE_TYPES` (`export-docx.ts:393-398`: PNG, JPEG, GIF, BMP), and that `null` is silent — no image and no warning. An accepted WebP would therefore vanish from a DOCX export while `NODE_FIDELITY` claimed native support in all three formats. Admitting it needs a tested transcode into one of those types on the DOCX path, a separate change with its own evidence. Until then a WebP file takes the unsupported-file path in Failure Handling: it is reported, nothing is stored, and the document is unchanged.

SVG is excluded. An SVG is an executable document rather than a bitmap, and the DOCX writer cannot embed it with the same reliability as the raster formats. The requirement admitted SVG only if it could be handled safely under the current architecture, and that cannot be asserted here today.

## Node View and Resizing

`writingImage` introduces the first `addNodeView` in this repository. It is a plain ProseMirror DOM node view — `dom` is the `figure`, `contentDOM` is the `figcaption` — and deliberately not a Svelte node view, so it adds no rendering dependency and leaves the existing nodes' `renderHTML`-only convention intact for everything that does not need interaction.

Because `contentDOM` (the `figcaption`) is narrower than `dom` (the `figure`), everything outside `contentDOM` — the `<img>`, the resize handles, any alignment chrome — is marked `contentEditable="false"`. Without it the caret can land inside non-content DOM and click-to-select becomes unreliable; this matters more here than it would for a typical node view, because `writingImage` is the first node view in the repository to carry a `contentDOM` at all.

Selecting the figure reveals corner handles. Dragging a handle changes `width` through a transaction, so resizing participates in undo and redo like any other edit. The aspect ratio is fixed, the width is floored at a minimum that keeps the image and its handles usable, and it is capped at the width of the editor's content area.

That cap is the one measurement this feature takes from live layout, and it is taken only while a handle is being dragged, which cannot happen outside a real browser. The clamping arithmetic is a pure function of the dragged width and the available width, so it is tested directly with numbers; no test drags a handle or reads a rendered size.

Alignment and the `alt` and `title` attributes are edited from a selection bubble built with the existing `ToolbarMenu` vocabulary rather than a permanent panel.

`selectable: true` gives node selection, and with it delete, backspace, cut, and copy, from ProseMirror itself. One keymap addition is required: backspace at the start of an empty caption selects the figure instead of joining it into the preceding block.

Because `content: 'inline*'` makes `writingImage` a textblock, ProseMirror's default `splitBlock` on Enter would create a second `writingImage` carrying no `src` — a node pointing at no stored bytes. Enter inside the caption is therefore overridden: it never splits the figure. It exits the node and creates an ordinary paragraph after it.

A manuscript ending in a `writingImage` node also needs somewhere after it for the caret to land. `trailing-paragraph.ts:20-21` already exists for exactly this failure — it defines a `TRAPPING` set (`table`, `blockquote`, `codeBlock`, `footnotes`, `horizontalRule`) of block types that must never be the document's last node without an empty paragraph appended after them — and `writingImage` joins that set. A gap cursor does not solve this: the extension has to be told about the new block type explicitly.

## Rendering and Layout

The stylesheet, not the stored attribute, is what guarantees the layout holds. The image carries `max-width: 100%` and `height: auto`, so a stored width larger than the column — from a narrower window, a different variant, or a hand-edited document — is clamped on render instead of overflowing. The figure stays inside the document margins and inherits theme tokens like every other block, so no new visual style is introduced.

Intrinsic dimensions are decoded from the file's header bytes by `image-dimensions.ts`, never by measuring a mounted element. This is the discipline `export-images.ts:114-152` already follows for PNG and JPEG; that decoder moves into the shared module and gains GIF, and `export-images.ts` consumes it from there. Measuring the DOM instead would also be untestable: in happy-dom an `<img>` never fires `load` and reports a natural size of zero, so a test that waited on it would pass while proving nothing.

## Serialization and Compatibility

Content is stored as canonical TipTap JSON in `writing_documents.current_content_json`, wrapped as `{ schemaVersion, doc }` (`packages/ui/src/components/WritingEditor/document-contract.ts:29-32`). A `writingImage` node serializes as its attributes plus, when captioned, its inline content. Nothing else about the envelope changes.

`WRITING_SCHEMA_VERSION` is not bumped. Adding a node type does not change the envelope's shape, which is what that number describes: it is "bumped only for a change the old reader cannot understand" (`packages/ui/src/components/WritingEditor/document-contract.ts:26`). The same principle is already applied to typography marks at `extensions.ts:292-293`. Bumping it would make every future save unreadable to an older build even for manuscripts that contain no image at all. The narrower consequence is the correct one: an older build opening a manuscript that does contain an image is refused by `classify()` as an unknown node (`document-contract.ts:83-94`) rather than shown blank. Failing closed on unreadable content is the existing contract, and it is preserved.

Existing manuscripts are untouched. They contain no `writingImage` node, so no stored byte changes and no migration runs. Citations, footnotes, note links, tables, links, text styles, search, and the delta journal are all unaffected: the node is additive and shares no attribute namespace with them.

## Exports

Every exporter walks the canonical JSON with its own `block`/`inline` switch. The block-level fallback ends in `default: return children()` (`export-html.ts:253`); the inline-level fallback ends in `default: return inline(childrenOf(node), context, notes)` (`export-html.ts:178`). An unregistered node does not throw there, but `writingImage`'s failure is not a lossy recursion into surviving content: none of the exporters' `block()` switches has a `text` case, and `childrenOf` (`apps/desktop/src/lib/export-document.ts:154-156`) returns `[]` for a text node, because its content lives in `.text`, not `.content`. So an unregistered `writingImage` produces nothing at all: the image and the caption both vanish, yielding an empty string, not a bare paragraph. Each exporter therefore gets an explicit case, and a test asserts the image is present rather than asserting the export merely succeeded.

`export-images.ts:38-58` currently collects only `documentCitation.attrs.quotedParts` image sources. It is extended to also collect `writingImage.attrs.src`, so all three exporters receive the bytes through the `ExportContext.images` record they already share (`export-document.ts:48-72`). This is the reuse the feature needs; no second image-loading path is added.

- **DOCX** embeds the bytes through `ImageRun`, which is already written and used for quote images (`export-docx.ts:420-425`), sized by the existing column-fitting helper (`export-docx.ts:405-417`) so the author's width is honoured up to the page column. The caption is emitted as the paragraph following the image.
- **HTML** emits a `figure` with the image as a `data:` URI, matching how quote images already travel.
- **Markdown** emits `![alt](data:...)` followed by the caption as a paragraph. This is the policy the Markdown exporter already states for quote images at `export-markdown.ts:137-141`: a Markdown file is one file, and a path into the archive would break the moment the file left this machine. Reusing it keeps one rule in the exporter instead of two.

## Export Guards

`supportOfNode` treats anything absent from its table as unsupported (`export-fidelity.ts:185-189`), so `writingImage` is registered in `NODE_FIDELITY` as native in all three formats.

Two guard tests are affected by that registration; only one needs a change:

- `export-vocabulary.test.ts:36-46` requires a `writing.exportElement.*` label in both locales only for an element that is ever a fallback or unsupported: `nameable()` collects `REQUIRED_BY_SPEC` plus every element with at least one non-`native` support value. `writingImage` is registered as `native` in all three formats and `REQUIRED_BY_SPEC` is left untouched, so the node never enters that set: no label is required, and none is added. The label becomes mandatory only if a format's support is later downgraded from `native`, or the node is added to `REQUIRED_BY_SPEC` — either case pulls in the label for both locales.
- `export-pattern.test.ts:276-280` asserts that a DOCX export of the canonical pattern document warns about `noteLink` and nothing else. The pattern document gains a captioned, resized image, and that assertion is re-verified against it.

`REQUIRED_BY_SPEC` is left alone. Listing the node there would make `losesRequiredElement` (`export-fidelity.ts:300-304`) refuse a whole DOCX export on any downgrade, and an image is not an element whose absence should void the manuscript.

## Failure Handling

- Import failure inserts nothing. A node is never created pointing at bytes that were not written.
- An unsupported or unreadable file is reported and the document is unchanged.
- A stored file missing at render time draws a placeholder in place of the image. The node is kept: the manuscript records that an image belongs there, and silently dropping it would destroy content the writer did not delete.
- A stored file missing at export time is left out of the export, and the export still succeeds. This follows the policy `loadExportImages` (`export-images.ts:60-64`) already states for quote images — a manuscript is worth more than one image, and the words are drawn without it. Adding a warning here would give the repository two rules for one problem; if that warning is ever wanted, it belongs to both image paths at once, as its own change.
- Caption text survives every one of these paths, because it is document content rather than an attribute of the file.

## Non-Goals

Garbage collection of unreferenced stored images. Image cropping or rotation inside the editor. Remote image URLs. A per-document image table. Any change to the investigation `assets` table or to `findByItem`-based asset resolution, which is an item-and-OCR subsystem with different ownership and lifetime rules.

## Verification

TDD covers these observable contracts:

1. The insert command creates a `writingImage` node at the cursor and an existing text selection is preserved rather than replaced.
2. The hashing entry point (`crypto.subtle.digest`, or its declared fallback if unavailable) runs successfully in the actual `packages/ui`/`apps/desktop` Vitest `happy-dom` environment.
3. A document containing the node serializes to canonical JSON and reloads into an equal document, caption and attributes included.
4. Importing the same bytes twice yields one stored file and one path.
5. The stored path is relative and contains no drive letter or home directory.
6. Deleting a selected image removes the node; undo restores it with its width, alignment, alt text, and caption; redo removes it again.
7. Pressing Enter inside the caption never splits the figure: it exits the node and creates an ordinary paragraph after it.
8. A manuscript ending in a `writingImage` node gets a trailing empty paragraph appended, so the caret has somewhere to land after it.
9. A resize transaction changes `width` and is undone as a single step.
10. A stored width wider than the column does not widen the rendered figure, and the clamping function refuses a negative, zero, or sub-minimum width and never returns more than the available width.
11. Pasting HTML that carries a remote or foreign-filesystem `img` never produces a node holding that source: the bytes are imported, or nothing is inserted.
12. Intrinsic dimensions are decoded from header bytes for PNG, JPEG, and GIF.
13. HTML export emits a figure with an embedded image and its caption.
14. Markdown export emits an embedded image and its caption, with no filesystem path in the output.
15. DOCX export embeds the image bytes and emits the caption, and the pattern document's DOCX warnings remain exactly `noteLink`.
16. A manuscript with no image serializes byte-identically to its pre-change serialization.
17. A missing stored file leaves the node in the document, and the export omits that image and still succeeds.
18. A WebP file offered through the picker, the clipboard, or a drop is refused before anything is stored or inserted: it is reported and the document is unchanged.

Beyond the suite, the feature is complete only after the full path is exercised by hand in the running application: open a document, place the cursor, insert from the toolbar, see the image, resize it, caption it, save, leave the document, reopen it, close EntropIA, start it again, reopen the document, confirm image, dimensions, alignment, and caption, then export to all three formats and open the results. Paste and drag-and-drop are exercised the same way.

Completion evidence is the focused `packages/ui` and `apps/desktop` suites, the Svelte autofixer, the Lite frontend typecheck under `VITE_LOCAL_ML=0`, and that manual run. Because the editor is a native Tauri window, the visual and interaction checks are confirmed by the user, not asserted here.
