import { Editor, type JSONContent } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { createWritingExtensions } from './extensions'

/**
 * Highlight: a mark of its own, holding a palette name and drawn through the
 * theme's token (writing-colors.ts). It is independent of the text colour and
 * combines with every other mark.
 */

let editor: Editor | undefined

afterEach(() => {
  editor?.destroy()
  editor = undefined
})

function mount(content: JSONContent | string) {
  const element = document.createElement('div')
  document.body.appendChild(element)
  editor = new Editor({ element, extensions: createWritingExtensions(), content })
  return editor
}

const doc = (...content: JSONContent[]): JSONContent => ({ type: 'doc', content })
const p = (...content: JSONContent[]): JSONContent => ({ type: 'paragraph', content })
const text = (value: string, marks?: JSONContent['marks']): JSONContent => ({
  type: 'text',
  text: value,
  ...(marks ? { marks } : {}),
})
const highlight = (color: string) => ({ type: 'highlight', attrs: { color } })

/** A paragraph's runs as [text, highlight | null]. */
function runs(instance: Editor, paragraph = 0): [string, unknown][] {
  const block = instance.getJSON().content?.[paragraph]
  return (block?.content ?? []).map((node) => {
    const mark = node.marks?.find((entry) => entry.type === 'highlight')
    return [node.text ?? '', mark ? mark.attrs?.color : null]
  })
}

describe('highlighting a selection', () => {
  it('highlights one word', () => {
    const instance = mount(doc(p(text('Hola mundo'))))
    instance.chain().setTextSelection({ from: 6, to: 11 }).setHighlight({ color: 'yellow' }).run()

    expect(runs(instance)).toEqual([
      ['Hola ', null],
      ['mundo', 'yellow'],
    ])
  })

  it('highlights across paragraphs, and changes the colour of a highlight', () => {
    const instance = mount(doc(p(text('Uno', [highlight('green')])), p(text('Dos'))))
    instance.chain().selectAll().setHighlight({ color: 'pink' }).run()

    expect(runs(instance, 0)).toEqual([['Uno', 'pink']])
    expect(runs(instance, 1)).toEqual([['Dos', 'pink']])
  })

  it('combines with a text colour and the other marks', () => {
    const instance = mount(doc(p(text('todo', [{ type: 'bold' }]))))
    instance.chain().selectAll().setTextColor('red').setHighlight({ color: 'blue' }).run()

    const marks = instance.getJSON().content?.[0]?.content?.[0]?.marks
    expect(marks).toEqual([
      { type: 'textStyle', attrs: { fontSize: null, color: 'red' } },
      { type: 'bold' },
      { type: 'highlight', attrs: { color: 'blue' } },
    ])
  })

  it('refuses a name that is not in the palette', () => {
    const instance = mount(doc(p(text('Hola'))))
    instance.commands.selectAll()

    expect(instance.commands.setHighlight({ color: '#ffff00' })).toBe(false)
    expect(runs(instance)).toEqual([['Hola', null]])
  })

  it('removes the highlight and nothing else', () => {
    const instance = mount(doc(p(text('marcado', [{ type: 'italic' }, highlight('yellow')]))))
    instance.chain().selectAll().unsetHighlight().run()

    expect(instance.getJSON().content?.[0]?.content?.[0]?.marks).toEqual([{ type: 'italic' }])
  })

  it('is one undo step', () => {
    const instance = mount(doc(p(text('Uno')), p(text('Dos'))))
    instance.chain().selectAll().setHighlight({ color: 'yellow' }).run()
    instance.chain().selectAll().setHighlight({ color: 'green' }).run()
    instance.commands.undo()

    expect(runs(instance, 1)).toEqual([['Dos', 'yellow']])
  })
})

describe('at a caret', () => {
  it('highlights the text typed next, and stops', () => {
    const instance = mount(doc(p(text('Hola'))))
    instance.chain().focus('end').setHighlight({ color: 'orange' }).insertContent(' mundo').run()
    instance.chain().unsetHighlight().insertContent('!').run()

    expect(runs(instance)).toEqual([
      ['Hola', null],
      [' mundo', 'orange'],
      ['!', null],
    ])
  })
})

describe('drawing it', () => {
  it('draws a name through its theme token and leaves the ink to the text', () => {
    const instance = mount(doc(p(text('amarillo', [highlight('yellow')]))))

    const mark = instance.view.dom.querySelector('mark') as HTMLElement
    expect(mark.dataset.highlight).toBe('yellow')
    const style = mark.getAttribute('style') ?? ''
    expect(style).toContain('--writing-highlight: var(--writing-highlight-yellow)')
    // The stylesheet paints it as a band one line tall; a background-color here
    // would fill the whole glyph box and cover the line above at tight spacing.
    expect(style).not.toContain('background-color')
    expect(style).not.toMatch(/(^|;)\s*color:/)
  })

  it('draws a name it does not know as no highlight, and keeps it in the document', () => {
    const instance = mount(doc(p(text('futuro', [highlight('chartreuse')]))))

    const mark = instance.view.dom.querySelector('mark') as HTMLElement
    expect(mark.hasAttribute('data-highlight')).toBe(false)
    expect(mark.hasAttribute('style')).toBe(false)
    expect(runs(instance)).toEqual([['futuro', 'chartreuse']])
  })
})

describe('pasting', () => {
  it('keeps a highlight copied from this editor', () => {
    const source = mount(doc(p(text('verde', [highlight('green')]))))
    const html = source.getHTML()
    source.destroy()

    const target = mount(doc(p()))
    target.commands.setContent(html)

    expect(runs(target)).toEqual([['verde', 'green']])
  })

  /**
   * The stock extension turns `==x==` into a highlight as it is typed or
   * pasted. In a manuscript `a == b == c` is prose or notation, not a request
   * for yellow, so the highlight comes only from the toolbar.
   */
  it('leaves double equals signs as text', () => {
    const instance = mount(doc(p()))
    instance.chain().focus().insertContent('si a ==b== c').run()
    instance.view.pasteText(' y ==d== e')

    expect(runs(instance)).toEqual([['si a ==b== c y ==d== e', null]])
  })

  /** The same rule as the text colour: someone else's colour is not a name. */
  it('ignores a highlight pasted from another program', () => {
    const instance = mount(doc(p()))
    instance.commands.setContent(
      '<p><mark>web</mark> <mark style="background-color: #ffff00">word</mark> <span style="background-color: yellow">otro</span></p>'
    )

    const marks = instance.getJSON().content?.[0]?.content?.flatMap((node) => node.marks ?? [])
    expect(marks).toEqual([])
  })
})
