# Citation Source Landing Design

## Goal

Following a manuscript citation opens the cited asset's extracted-text pane, centers the cited passage in view, and marks the complete passage even when it crosses formatting elements or paragraphs.

## Scope

Change only citation landing. Citation creation, storage, integrity verification, and source selection remain unchanged.

Affected units:

- `apps/desktop/src/views/ItemView.svelte`: forwards the verified citation range to the asset panel.
- `apps/desktop/src/views/ItemAssetPanel.svelte`: selects the extracted-text pane on a new citation arrival and invokes marking after asynchronous OCR rendering.
- `apps/desktop/src/lib/rendered-text-map.ts`: converts a verified raw extraction range back to its visible range.
- `apps/desktop/src/lib/highlight-fragment.ts`: marks every rendered text-node segment covered by that visible range and scrolls to its first mark.

## Data Flow

1. `resolveCitationTarget` continues to decide whether the source is unchanged and may be highlighted.
2. Navigation carries the verified `{ start, end, text }` range to `ItemView`.
3. `ItemView` passes that complete range to `ItemAssetPanel`; it no longer reduces the request to the quoted text alone.
4. A newly received citation range selects `Texto extraído`. Normal asset navigation without a citation continues to select `Documento`.
5. Once `OcrRichText` finishes rendering, the marking path builds the existing visible/raw alignment from `ocrEditedText` and the rendered container text.
6. The map returns the visible range corresponding to the raw citation offsets. An uncertain mapping returns `null`; no guessed location is marked.
7. The marker walks rendered text nodes, wraps every intersecting substring in `<mark class="citation-hit">`, preserves existing markup, and calls `scrollIntoView({ block: 'center' })` on the first mark.

## Mapping Contract

`RenderedTextMap` gains a raw-to-visible operation alongside `toRaw`. It uses the same alignment table and markup mask that already protect citation creation. The returned half-open visible range begins at the first confidently aligned visible character inside the raw range and ends after the last one. Unmapped markup and separators between those endpoints remain inside the visual span; they never create a new anchor.

The operation returns `null` when the raw range is empty, outside the extraction, or has no confidently aligned visible characters. This preserves the existing rule: failure to prove a location must not point to a different sentence.

## Pane State

A citation arrival is distinct from ordinary selection:

- New asset with citation: open extracted text.
- Same asset with a new citation: reopen extracted text and mark the new range.
- New asset without citation: keep the current default of opening the document.
- Navigation away from the cited item: discard the old range; it must not mark matching words in another item.
- After arrival, the user may switch back to `Documento`; the component does not continuously force the text pane.

## DOM Marking

The marker creates one mark per covered text-node segment rather than surrounding one cross-element `Range`. This preserves headings, emphasis, tables, and paragraph boundaries because no ancestor structure is rebuilt. Segments are collected before mutation and wrapped from the end toward the beginning so earlier DOM edits cannot invalidate later positions.

Before applying a new citation, existing `mark.citation-hit` elements are unwrapped and their parents normalized. The first mark is the scroll target; all marks represent one citation.

## Failure Handling

If rendering or mapping cannot locate the verified raw range, the extracted-text pane still opens. No partial prefix, repeated-text guess, or fallback occurrence is highlighted. Existing source-modified handling remains authoritative and unchanged.

## Verification

TDD covers these observable contracts:

1. A raw range maps back to the correct visible range across Markdown formatting and paragraph separators.
2. A citation crossing elements and paragraphs produces marks for every covered text segment while preserving the original element structure.
3. The first mark receives centered scrolling.
4. Repeated visible wording resolves through raw offsets rather than the first textual occurrence.
5. Citation navigation opens `Texto extraído` without a manual click.
6. A second citation on the same asset reopens the text pane and replaces the marks.
7. Navigating to another item without a citation does not retain the previous mark.

Focused desktop tests, Svelte autofixer, Lite frontend typecheck, and browser verification of the actual citation-following path provide completion evidence.
