import { describe, expect, it } from 'vitest'
import { shouldIgnoreWritingImageMutation, shouldStopWritingImageEvent } from './writing-image-node-view'

function buildFigure() {
  const figure = document.createElement('figure')
  const img = document.createElement('img')
  const chrome = document.createElement('div')
  const handle = document.createElement('button')
  const alignButton = document.createElement('button')
  chrome.append(alignButton)
  const figcaption = document.createElement('figcaption')
  const captionText = document.createTextNode('a caption')
  figcaption.append(captionText)
  figure.append(img, chrome, handle, figcaption)
  return { figure, img, chrome, handle, alignButton, figcaption, captionText }
}

describe('shouldIgnoreWritingImageMutation', () => {
  it('ignores an attribute mutation on the image (outside the caption)', () => {
    const { img, figcaption } = buildFigure()
    expect(
      shouldIgnoreWritingImageMutation(figcaption, { type: 'attributes', target: img })
    ).toBe(true)
  })

  it('ignores a childList mutation on the chrome (outside the caption)', () => {
    const { chrome, figcaption } = buildFigure()
    expect(
      shouldIgnoreWritingImageMutation(figcaption, { type: 'childList', target: chrome })
    ).toBe(true)
  })

  it('does not ignore a characterData mutation inside the caption — a real edit', () => {
    const { captionText, figcaption } = buildFigure()
    expect(
      shouldIgnoreWritingImageMutation(figcaption, { type: 'characterData', target: captionText })
    ).toBe(false)
  })

  it('does not ignore a mutation whose target is the caption element itself', () => {
    const { figcaption } = buildFigure()
    expect(
      shouldIgnoreWritingImageMutation(figcaption, { type: 'childList', target: figcaption })
    ).toBe(false)
  })

  it('never ignores a selection mutation, even when its target sits outside the caption', () => {
    const { img, figcaption } = buildFigure()
    expect(
      shouldIgnoreWritingImageMutation(figcaption, { type: 'selection', target: img })
    ).toBe(false)
  })
})

// shouldStopWritingImageEvent takes a *list* of interactive roots (I1 fix
// round, defect 3): the resize handle no longer lives inside the same
// container as the alignment/alt toolbar — it moves into the image's own
// shrink-wrapped frame so it can be positioned on the image's corner instead
// of the far edge of the full-width figure — so the predicate has to check
// membership across two independent containers, not one.
describe('shouldStopWritingImageEvent', () => {
  it('stops an event targeting the resize handle, even when the handle lives outside the toolbar chrome', () => {
    const { chrome, handle } = buildFigure()
    const event = new Event('pointerdown')
    Object.defineProperty(event, 'target', { value: handle })
    expect(shouldStopWritingImageEvent([chrome, handle], event)).toBe(true)
  })

  it('does not stop the handle when it is not passed as one of the roots', () => {
    const { chrome, handle } = buildFigure()
    const event = new Event('pointerdown')
    Object.defineProperty(event, 'target', { value: handle })
    expect(shouldStopWritingImageEvent([chrome], event)).toBe(false)
  })

  it('stops an event targeting an alignment button', () => {
    const { chrome, alignButton, handle } = buildFigure()
    const event = new Event('click')
    Object.defineProperty(event, 'target', { value: alignButton })
    expect(shouldStopWritingImageEvent([chrome, handle], event)).toBe(true)
  })

  it('does not stop an event targeting the image — ProseMirror still needs it for node selection', () => {
    const { chrome, img, handle } = buildFigure()
    const event = new Event('mousedown')
    Object.defineProperty(event, 'target', { value: img })
    expect(shouldStopWritingImageEvent([chrome, handle], event)).toBe(false)
  })

  it('does not stop an event targeting the caption', () => {
    const { chrome, figcaption, handle } = buildFigure()
    const event = new Event('click')
    Object.defineProperty(event, 'target', { value: figcaption })
    expect(shouldStopWritingImageEvent([chrome, handle], event)).toBe(false)
  })

  it('stops nothing when given an empty list of roots', () => {
    const { handle } = buildFigure()
    const event = new Event('pointerdown')
    Object.defineProperty(event, 'target', { value: handle })
    expect(shouldStopWritingImageEvent([], event)).toBe(false)
  })
})
