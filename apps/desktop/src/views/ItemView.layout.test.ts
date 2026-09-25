import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * The right panel (Notas/Texto/Análisis/Mapa/Búsquedas/Layout/Metadatos) used
 * to scroll sideways at a 320-400px pane width: `sidebarWidth` (min 20%,
 * default 33%) of an already-narrow pane left only ~65-100px of real content
 * width, which is less than several right-panel rows need even after they
 * reflow. Two things fixed it, verified empirically in real Chromium at pane
 * widths 320/400/560/720/900/1200/full: a floor on the right column's grid
 * track so it always gets a usable minimum, and per-row reflow so nothing
 * inside that column still forces its own horizontal scroll.
 */
const ITEM_VIEW_SOURCE = readFileSync(resolve(import.meta.dirname, 'ItemView.svelte'), 'utf-8')
const NOTES_SOURCE = readFileSync(resolve(import.meta.dirname, 'ItemNotesPanel.svelte'), 'utf-8')
const METADATA_SOURCE = readFileSync(
  resolve(import.meta.dirname, 'ItemMetadataPanel.svelte'),
  'utf-8'
)
const LAYOUT_SOURCE = readFileSync(resolve(import.meta.dirname, 'ItemLayoutPanel.svelte'), 'utf-8')
const TEXT_SOURCE = readFileSync(resolve(import.meta.dirname, 'ItemTextPanel.svelte'), 'utf-8')
const METADATA_EDITOR_SOURCE = readFileSync(
  resolve(
    import.meta.dirname,
    '../../../../packages/ui/src/components/MetadataEditor/MetadataEditor.svelte'
  ),
  'utf-8'
)

describe('ItemView right panel does not scroll sideways at a narrow pane width', () => {
  it('gives the right column a usable minimum instead of a bare percentage of the pane', () => {
    // A pure `${sidebarWidth}%` track can resolve under 100px in a 320px
    // pane; `minmax()` keeps today's wide-pane ratio (the floor never wins
    // once the percentage already clears it) while flooring the narrow case.
    expect(ITEM_VIEW_SOURCE).toMatch(/minmax\(190px,\s*\$\{sidebarWidth\}%\)/)
  })

  it('reflows a note row to a second line instead of squeezing the date and actions off-screen', () => {
    expect(NOTES_SOURCE).toMatch(/\.note-card\s*\{[^}]*container-type:\s*inline-size;/)
    expect(NOTES_SOURCE).toMatch(
      /@container \(max-width: 200px\) \{\s*\.note-row \{[^}]*grid-template-areas:\s*\n\s*'preview preview'\s*\n\s*'date actions';/
    )
  })

  it('lets the layout tab header wrap instead of pushing the overlay toggle past the pane edge', () => {
    expect(LAYOUT_SOURCE).toMatch(/\.layout-section-header\s*\{[^}]*flex-wrap:\s*wrap;/)
  })

  it('lets the OCR unavailable hint break a long word instead of overflowing a narrow pane', () => {
    expect(TEXT_SOURCE).toMatch(/\.ocr-llm-hint\s*\{[^}]*overflow-wrap:\s*break-word;/)
  })

  it('stacks the metadata label/value rows instead of squeezing both columns to nothing', () => {
    expect(METADATA_SOURCE).toMatch(/\.section\s*\{[^}]*container-type:\s*inline-size;/)
    expect(METADATA_SOURCE).toMatch(
      /@container \(max-width: 200px\) \{\s*\.metadata-list__row \{[^}]*grid-template-columns:\s*1fr;/
    )
  })

  it('stacks the custom-metadata editor rows at a narrow container width', () => {
    expect(METADATA_EDITOR_SOURCE).toMatch(/\.metadata-editor\s*\{[^}]*container-type:\s*inline-size;/)
    expect(METADATA_EDITOR_SOURCE).toMatch(
      /@container \(max-width: 220px\) \{\s*\.metadata-editor__header \{\s*display:\s*none;/
    )
  })
})
