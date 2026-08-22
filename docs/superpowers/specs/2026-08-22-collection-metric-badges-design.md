# Collection Metric Badges Design

## Goal

Replace the collection header's pipe-separated status line with compact, independently readable badges for the collection-wide metrics: items, assets, OCR coverage, embedding coverage, NER coverage, and triplet coverage.

## Existing pattern

`packages/ui/src/components/StatusBadge/StatusBadge.svelte` already provides the visual language required by this change:

- compact `sm` sizing;
- rounded control radius;
- thin themed border;
- subtle themed background;
- small typography and internal spacing;
- theme tokens that work in light and dark modes.

`apps/desktop/src/layout/SyncStatusIndicator.svelte` uses this component for “Sincronización al día”. Collection metrics will reuse the same component rather than duplicating its CSS.

## Design

`apps/desktop/src/views/CollectionView.svelte` will:

1. Keep the existing `CollectionStats` loading, refresh, and translation logic.
2. Replace the derived pipe-separated `collectionStatsLabel` string with a derived array of translated metric labels.
3. Render one `StatusBadge` per label, using `variant="neutral"` and `size="sm"` for every metric.
4. Place the badges in a semantic wrapper below the collection subtitle.
5. Use wrapper-level layout only (`display: flex`, wrapping, and token-based gap) so the badges remain compact and responsive without introducing a second visual style.

The labels and counts remain unchanged. Distinction comes from the translated metric text and count, not from color variation, because these are informational values rather than success, warning, or error states.

## Accessibility and responsive behavior

- Each badge remains visible text, preserving the current translated labels and counts.
- The wrapper may use a non-semantic grouping element because the badges are informational and not interactive.
- `flex-wrap` prevents overflow when the header narrows or the locale produces longer labels.
- Existing header semantics and reading order remain unchanged.

## Testing

Update `apps/desktop/src/views/CollectionView.test.ts` so the collection stats test verifies:

- each of the six metric labels is rendered independently;
- the six metric values are present;
- the old single text node containing `|` is no longer rendered.

No data-layer or translation changes are required. Existing synchronization badge tests remain the reference for shared `StatusBadge` behavior.

## Scope and non-goals

In scope:

- collection header metric markup;
- reuse of the shared `StatusBadge` component;
- layout styling for the badge group;
- focused view test updates.

Out of scope:

- changing metric calculations or refresh behavior;
- changing translation strings;
- adding new badge variants or design tokens;
- changing the synchronization indicator;
- creating a new shared component for this one consumer.
