import { Editor } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { createWritingExtensions } from './extensions'
import { goToMatch, readSearch, replaceAll, replaceCurrent, setSearch } from './search-highlight'

/**
 * The search as the editor experiences it.
 *
 * `search.ts` is tested on its own; what is asserted here is everything that
 * only shows up against a live editor — that looking for a word does not count
 * as editing it, that a replacement lands on the right range, and that
 * replacing every match does not walk over its own positions.
 */

let editor: Editor | undefined

afterEach(() => {
  editor?.destroy()
  editor = undefined
})

function mount(text: string) {
  const element = document.createElement('div')
  document.body.appendChild(element)
  editor = new Editor({
    element,
    extensions: createWritingExtensions(),
    content: {
      type: 'doc',
      content: [{ type: 'paragraph', content: [{ type: 'text', text }] }],
    },
  })
  return editor
}

describe('searching does not edit', () => {
  /**
   * The autosave loop watches for a changed document. If a search advanced the
   * document, every query typed would earn a revision and a journal entry —
   * which is how a reader ends up with a history of having looked for things.
   */
  it('leaves the document untouched', () => {
    const instance = mount('el molino de viento')
    const before = JSON.stringify(instance.getJSON())

    setSearch(instance, { query: 'molino' })
    goToMatch(instance, 0)

    expect(JSON.stringify(instance.getJSON())).toBe(before)
  })

  it('finds the matches and makes one of them current', () => {
    const instance = mount('sal y mas sal')

    const state = setSearch(instance, { query: 'sal' })
    expect(state.matches).toHaveLength(2)

    const moved = goToMatch(instance, 1)
    expect(moved.current).toBe(1)
  })

  it('re-runs the search when the document changes underneath it', () => {
    const instance = mount('sal')
    setSearch(instance, { query: 'sal' })

    instance.chain().focus('end').insertContent(' y mas sal').run()

    expect(readSearch(instance).matches).toHaveLength(2)
  })
})

describe('replacing', () => {
  it('replaces the current match and nothing else', () => {
    const instance = mount('sal y mas sal')
    setSearch(instance, { query: 'sal' })
    goToMatch(instance, 0)

    replaceCurrent(instance, 'azucar')

    expect(instance.getText()).toBe('azucar y mas sal')
  })

  it('does nothing when no match is current', () => {
    const instance = mount('sal y mas sal')
    setSearch(instance, { query: 'sal' })

    replaceCurrent(instance, 'azucar')

    expect(instance.getText()).toBe('sal y mas sal')
  })

  /**
   * Replacing forwards would apply each range after the earlier replacements
   * had already shifted it. With a longer replacement than the match, that puts
   * the text in visibly the wrong place.
   */
  it('replaces every match without walking over its own positions', () => {
    const instance = mount('sal y mas sal y aun mas sal')

    replaceAll(instance, 'azucar')
    expect(instance.getText()).toBe('sal y mas sal y aun mas sal')

    setSearch(instance, { query: 'sal' })
    replaceAll(instance, 'azucar')

    expect(instance.getText()).toBe('azucar y mas azucar y aun mas azucar')
  })

  it('is a single undo', () => {
    const instance = mount('sal y mas sal')
    setSearch(instance, { query: 'sal' })
    replaceAll(instance, 'azucar')

    instance.commands.undo()

    expect(instance.getText()).toBe('sal y mas sal')
  })

  it('keeps the index in range when a replacement removes the last match', () => {
    const instance = mount('sal y mas sal')
    setSearch(instance, { query: 'sal' })
    goToMatch(instance, 1)

    const after = replaceCurrent(instance, 'azucar')

    expect(after.matches).toHaveLength(1)
    expect(after.current).toBeLessThan(after.matches.length)
  })
})
