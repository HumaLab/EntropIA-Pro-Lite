# Following a citation back to its source lands badly

## Objective

A citation clicked in the manuscript should land the reader on the cited
words, visible and fully marked — not near them.

## Problem

Found by hand on 2026-09-20 against the real archive, verifying 1.0.13.
The anchor travels correctly: `navigation.ts` carries
`citationRange { start, end, text }`, and the source opens on the right asset.
What fails is the landing:

1. **Wrong pane.** A text citation opens on `Documento`, which shows the
   image. The marked text lives on `Texto extraído`, and the reader has to
   switch panes by hand to see what they clicked for.
2. **No scroll.** Once on the text pane, a fragment below the fold is simply
   not in view. Nothing scrolls it into sight.
3. **Only the first paragraph is marked.** A citation spanning several
   paragraphs highlights the first one and leaves the rest unmarked, so the
   reader cannot see where the quotation ends.

Defect 3 has a documented cause. `source-selection.ts` converts a selection to
offsets only when both ends sit in **one text node** — the raw pane is a single
interpolated node, by design. The *rendered* pane is not: `renderOcrHtml`
rewrites it into paragraphs and images, so a range that crosses a paragraph
crosses nodes. `rendered-text-map.ts` exists to map offsets onto that rendered
structure; the marking path does not use it across node boundaries.

## Scope

- `apps/desktop/src/views/ItemView.svelte` — pane selection and scroll on
  arrival with a `citationRange`.
- `apps/desktop/src/lib/rendered-text-map.ts` and the marking path — a range
  that spans nodes marks every node it covers.

Not in scope: changing how a citation is anchored or stored. The anchor is
correct; only the landing is wrong.

## Acceptance

Clicking a citation opens the text pane, scrolls the fragment into view, and
marks it end to end, including one that spans paragraphs.

## Notes

Not a 1.0.13 blocker: nothing is lost, the citation resolves to the right
asset and the words are there to be found. It degrades a headline feature of
the release, so it is the first thing after it.
