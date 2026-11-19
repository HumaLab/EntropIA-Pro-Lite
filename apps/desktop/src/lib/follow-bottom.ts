/**
 * Keeps a scroll container showing its end while the reader stays there.
 *
 * The container is "pinned" while its bottom edge is within
 * `FOLLOW_THRESHOLD_PX` of the end. Any growth of its content (new nodes,
 * changed text) or of its own box (window resize) scrolls it back to the end
 * when pinned. Scrolling up unpins it, so reading older messages is never
 * interrupted; scrolling back down re-pins it. Changing the `pin` parameter
 * (for example when the user sends a question) re-pins and jumps to the end.
 */
export const FOLLOW_THRESHOLD_PX = 32

export interface FollowBottomOptions {
  /** Any value; each change re-pins the container and scrolls to the end. */
  pin?: unknown
}

function distanceFromEnd(node: HTMLElement): number {
  return node.scrollHeight - node.scrollTop - node.clientHeight
}

function scrollToEnd(node: HTMLElement): void {
  node.scrollTop = node.scrollHeight
}

export function followBottom(node: HTMLElement, options: FollowBottomOptions = {}) {
  let pinned = true
  let pinKey = options.pin

  const onScroll = () => {
    pinned = distanceFromEnd(node) <= FOLLOW_THRESHOLD_PX
  }

  const follow = () => {
    if (pinned) scrollToEnd(node)
  }

  node.addEventListener('scroll', onScroll, { passive: true })

  const mutations = new MutationObserver(follow)
  mutations.observe(node, { childList: true, subtree: true, characterData: true })

  const resizes = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(follow)
  resizes?.observe(node)

  scrollToEnd(node)

  return {
    update(next: FollowBottomOptions = {}) {
      if (Object.is(next.pin, pinKey)) return
      pinKey = next.pin
      pinned = true
      scrollToEnd(node)
    },
    destroy() {
      node.removeEventListener('scroll', onScroll)
      mutations.disconnect()
      resizes?.disconnect()
    },
  }
}
