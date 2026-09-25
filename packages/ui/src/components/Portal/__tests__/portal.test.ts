import { afterEach, describe, expect, it } from 'vitest'
import { OVERLAY_ROOT_ATTRIBUTE, portal } from '../portal'

afterEach(() => {
  document.body.innerHTML = ''
})

function paneWith(node: HTMLElement): HTMLElement {
  const pane = document.createElement('div')
  pane.className = 'work-pane'
  pane.appendChild(node)
  document.body.appendChild(pane)
  return pane
}

describe('portal', () => {
  it('moves the node out of its pane to the end of <body> when no overlay root exists', () => {
    const overlay = document.createElement('div')
    const pane = paneWith(overlay)

    portal(overlay)

    expect(pane.contains(overlay)).toBe(false)
    expect(overlay.parentElement).toBe(document.body)
    expect(document.body.lastElementChild).toBe(overlay)
  })

  it('prefers the overlay root the shell provides', () => {
    const root = document.createElement('div')
    root.setAttribute(OVERLAY_ROOT_ATTRIBUTE, '')
    document.body.appendChild(root)
    const overlay = document.createElement('div')
    const pane = paneWith(overlay)

    portal(overlay)

    expect(pane.contains(overlay)).toBe(false)
    expect(overlay.parentElement).toBe(root)
  })

  it('removes the node on teardown', () => {
    const overlay = document.createElement('div')
    paneWith(overlay)

    const teardown = portal(overlay)
    teardown()

    expect(overlay.isConnected).toBe(false)
  })
})
