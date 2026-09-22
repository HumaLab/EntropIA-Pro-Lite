import { describe, expect, it } from 'vitest'
import { shouldIgnoreWritingImageMutation, shouldStopWritingImageEvent } from './writing-image-node-view'

function buildFigure() {
  const figure = document.createElement('figure')
  const img = document.createElement('img')
  const chrome = document.createElement('div')
  const handle = document.createElement('button')
  const alignButton = document.createElement('button')
  chrome.append(alignButton, handle)
  const figcaption = document.createElement('figcaption')
  const captionText = document.createTextNode('a caption')
  figcaption.append(captionText)
  figure.append(img, chrome, figcaption)
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

describe('shouldStopWritingImageEvent', () => {
  it('stops an event targeting the resize handle', () => {
    const { chrome, handle } = buildFigure()
    const event = new Event('pointerdown')
    Object.defineProperty(event, 'target', { value: handle })
    expect(shouldStopWritingImageEvent(chrome, event)).toBe(true)
  })

  it('stops an event targeting an alignment button', () => {
    const { chrome, alignButton } = buildFigure()
    const event = new Event('click')
    Object.defineProperty(event, 'target', { value: alignButton })
    expect(shouldStopWritingImageEvent(chrome, event)).toBe(true)
  })

  it('does not stop an event targeting the image — ProseMirror still needs it for node selection', () => {
    const { chrome, img } = buildFigure()
    const event = new Event('mousedown')
    Object.defineProperty(event, 'target', { value: img })
    expect(shouldStopWritingImageEvent(chrome, event)).toBe(false)
  })

  it('does not stop an event targeting the caption', () => {
    const { chrome, figcaption } = buildFigure()
    const event = new Event('click')
    Object.defineProperty(event, 'target', { value: figcaption })
    expect(shouldStopWritingImageEvent(chrome, event)).toBe(false)
  })
})
