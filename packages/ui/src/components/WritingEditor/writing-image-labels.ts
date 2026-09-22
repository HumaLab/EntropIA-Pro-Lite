/**
 * Every string the writingImage node view's own chrome shows (I2 fix round):
 * the alignment buttons, the alt/title fields, the resize handle's accessible
 * name, and the missing-file placeholder. Everything else in this editor
 * already flows through `WritingEditorLabels` → `writing-editor-labels.ts` →
 * `i18n.ts` (extensions.ts's own `insertImage` toolbar label among them); this
 * node view is a plain ProseMirror DOM view built once at construction, not a
 * reactive Svelte component, so it takes its strings as a resolved options
 * object rather than a live translation function — the same shape
 * `resolveImage` and `importImage` already take.
 *
 * The defaults below are never meant to reach a user: the app always passes
 * its own translated labels (WritingEditor.svelte). They exist so a test that
 * mounts `createWritingExtensions()` with no options still gets a node view
 * whose chrome has real accessible names instead of `undefined`.
 */
export interface WritingImageLabels {
  alignLeft: string
  alignCenter: string
  alignRight: string
  altLabel: string
  titleLabel: string
  resizeHandle: string
  missingImage: string
}

export const DEFAULT_WRITING_IMAGE_LABELS: WritingImageLabels = {
  alignLeft: 'Align left',
  alignCenter: 'Center',
  alignRight: 'Align right',
  altLabel: 'Alt text',
  titleLabel: 'Title',
  resizeHandle: 'Resize image',
  missingImage: 'Image unavailable',
}
