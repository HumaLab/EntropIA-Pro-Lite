import { afterEach, describe, expect, it } from 'vitest'
import { followBottom } from './follow-bottom'

// happy-dom does no layout, so the box is simulated: the content height
// grows with each appended line and scrollTop is clamped like a browser.
const LINE_PX = 100
const VIEW_PX = 300

function makeContainer() {
  const node = document.createElement('div')
  let top = 0
  const contentHeight = () => node.childNodes.length * LINE_PX
  Object.defineProperty(node, 'clientHeight', { get: () => VIEW_PX })
  Object.defineProperty(node, 'scrollHeight', { get: () => Math.max(contentHeight(), VIEW_PX) })
  Object.defineProperty(node, 'scrollTop', {
    get: () => top,
    set: (value: number) => {
      top = Math.max(0, Math.min(value, Math.max(contentHeight(), VIEW_PX) - VIEW_PX))
    },
  })
  document.body.append(node)
  return node
}

function addLine(node: HTMLElement) {
  const line = document.createElement('p')
  line.textContent = `line ${node.childNodes.length}`
  node.append(line)
}

function userScrollTo(node: HTMLElement, top: number) {
  node.scrollTop = top
  node.dispatchEvent(new Event('scroll'))
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0))
const atEnd = (node: HTMLElement) => node.scrollHeight - node.scrollTop - node.clientHeight

afterEach(() => {
  document.body.innerHTML = ''
})

describe('followBottom', () => {
  it('scrolls to the end when new content arrives while pinned', async () => {
    const node = makeContainer()
    const action = followBottom(node)
    for (let i = 0; i < 6; i++) addLine(node)
    await flush()

    expect(atEnd(node)).toBe(0)
    action.destroy()
  })

  it('keeps following the end through content that keeps growing', async () => {
    const node = makeContainer()
    const action = followBottom(node)
    for (let i = 0; i < 4; i++) {
      addLine(node)
      await flush()
      node.dispatchEvent(new Event('scroll'))
    }
    addLine(node)
    await flush()

    expect(atEnd(node)).toBe(0)
    action.destroy()
  })

  it('leaves the position alone while the reader looks at older messages', async () => {
    const node = makeContainer()
    for (let i = 0; i < 6; i++) addLine(node)
    const action = followBottom(node)
    userScrollTo(node, 0)
    addLine(node)
    await flush()

    expect(node.scrollTop).toBe(0)
    action.destroy()
  })

  it('follows again once the reader scrolls back to the end', async () => {
    const node = makeContainer()
    for (let i = 0; i < 6; i++) addLine(node)
    const action = followBottom(node)
    userScrollTo(node, 0)
    userScrollTo(node, node.scrollHeight)
    addLine(node)
    await flush()

    expect(atEnd(node)).toBe(0)
    action.destroy()
  })

  it('re-pins and jumps to the end when the pin key changes', async () => {
    const node = makeContainer()
    for (let i = 0; i < 6; i++) addLine(node)
    const action = followBottom(node, { pin: 0 })
    userScrollTo(node, 0)
    action.update({ pin: 1 })
    expect(atEnd(node)).toBe(0)

    addLine(node)
    await flush()
    expect(atEnd(node)).toBe(0)
    action.destroy()
  })

  it('does not move when the pin key is unchanged', () => {
    const node = makeContainer()
    for (let i = 0; i < 6; i++) addLine(node)
    const action = followBottom(node, { pin: 0 })
    userScrollTo(node, 0)
    action.update({ pin: 0 })

    expect(node.scrollTop).toBe(0)
    action.destroy()
  })

  it('stops following after destroy', async () => {
    const node = makeContainer()
    const action = followBottom(node)
    action.destroy()
    for (let i = 0; i < 6; i++) addLine(node)
    await flush()

    expect(node.scrollTop).toBe(0)
  })
})
