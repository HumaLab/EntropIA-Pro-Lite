/**
 * Moves a full-window overlay out of the view that rendered it.
 *
 * # Why
 *
 * The app's work panes are CSS size containers (`container-type:
 * inline-size`) so views can reflow with `@container` rules. Size containment
 * makes a pane the containing block of its `position: fixed` descendants, so an
 * overlay left inside it would cover the pane only, not the window.
 *
 * # Where it goes
 *
 * Into the element carrying `data-overlay-root` when the shell provides one,
 * otherwise to the end of <body>. The shell's root keeps the overlays in the
 * same stacking context they had before being moved: the tooltip layer stays
 * above them, and the menus floating in <body> stay above them too.
 *
 * Theme tokens live on <html>, so a moved overlay paints the same; Svelte's
 * scoped classes travel with the node. Put it on the element that is the only
 * root of its block (an `{#if}` branch or a component), because Svelte
 * removes a block by walking its root nodes in place.
 *
 * Usage: `<div class="overlay" {@attach portal}>`.
 */
export const OVERLAY_ROOT_ATTRIBUTE = 'data-overlay-root'

export function portal(node: HTMLElement): () => void {
  const host = document.querySelector(`[${OVERLAY_ROOT_ATTRIBUTE}]`) ?? document.body
  host.appendChild(node)
  return () => node.remove()
}
