import { Editor, type JSONContent } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { createWritingExtensions } from './extensions'
import { FONT_SIZE_SCALE, parseFontSize, stepFontSize } from './font-size'

/**
 * Relative font size: A+ and A− move text one step along a fixed scale of em
 * values, so a manuscript follows whichever typography preset is on and never
 * pins an absolute size the preset cannot move.
 */

let editor: Editor | undefined

afterEach(() => {
  editor?.destroy()
  editor = undefined
})

function mount(content: JSONContent) {
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
const size = (fontSize: string) => ({ type: 'textStyle', attrs: { fontSize } })

/** The first paragraph's runs as [text, fontSize | null]. */
function runs(instance: Editor, paragraph = 0): [string, string | null][] {
  const block = instance.getJSON().content?.[paragraph]
  return (block?.content ?? []).map((node) => {
    const style = node.marks?.find((mark) => mark.type === 'textStyle')
    return [node.text ?? '', (style?.attrs?.fontSize as string | undefined) ?? null]
  })
}

describe('the scale', () => {
  it('is a fixed set of em steps with 1 as the unmarked size', () => {
    expect(FONT_SIZE_SCALE).toEqual([0.75, 0.875, 1, 1.125, 1.25, 1.5, 1.75, 2])
  })

  it('reads em values, and snaps one off the scale to its nearest step', () => {
    expect(parseFontSize('1.25em')).toBe(1.25)
    expect(parseFontSize('1.3em')).toBe(1.25)
    expect(parseFontSize('9em')).toBe(2)
  })

  it('reads the unmarked size, absolute units and rubbish as no size', () => {
    expect(parseFontSize('1em')).toBeNull()
    expect(parseFontSize('12pt')).toBeNull()
    expect(parseFontSize('16px')).toBeNull()
    expect(parseFontSize('1em;background:red')).toBeNull()
    expect(parseFontSize(null)).toBeNull()
  })

  it('steps up and down, and stops at the ends', () => {
    expect(stepFontSize(1, 1)).toBe(1.125)
    expect(stepFontSize(1, -1)).toBe(0.875)
    expect(stepFontSize(2, 1)).toBe(2)
    expect(stepFontSize(0.75, -1)).toBe(0.75)
  })
})

describe('A+ and A− on a selection', () => {
  it('enlarges one word and leaves the rest of the line alone', () => {
    const instance = mount(doc(p(text('Hola mundo'))))
    instance.chain().setTextSelection({ from: 6, to: 11 }).increaseFontSize().run()

    expect(runs(instance)).toEqual([
      ['Hola ', null],
      ['mundo', '1.125em'],
    ])
  })

  it('shrinks a phrase', () => {
    const instance = mount(doc(p(text('Una frase entera aquí'))))
    instance.chain().setTextSelection({ from: 1, to: 10 }).decreaseFontSize().run()

    expect(runs(instance)).toEqual([
      ['Una frase', '0.875em'],
      [' entera aquí', null],
    ])
  })

  it('reaches across paragraphs', () => {
    const instance = mount(doc(p(text('Uno')), p(text('Dos'))))
    instance.chain().selectAll().increaseFontSize().run()

    expect(runs(instance, 0)).toEqual([['Uno', '1.125em']])
    expect(runs(instance, 1)).toEqual([['Dos', '1.125em']])
  })

  it('moves each size in a mixed selection one step of its own', () => {
    const instance = mount(doc(p(text('a'), text('b', [size('1.25em')]), text('c', [size('2em')]))))
    instance.chain().selectAll().increaseFontSize().run()

    expect(runs(instance)).toEqual([
      ['a', '1.125em'],
      ['b', '1.5em'],
      ['c', '2em'],
    ])
  })

  it('drops the mark altogether when a step lands back on the unmarked size', () => {
    const instance = mount(doc(p(text('grande', [size('1.125em'), { type: 'bold' }]))))
    instance.chain().selectAll().decreaseFontSize().run()

    const node = instance.getJSON().content?.[0]?.content?.[0]
    expect(node?.marks).toEqual([{ type: 'bold' }])
  })

  it('keeps the other marks on the text it resizes', () => {
    const instance = mount(doc(p(text('negrita', [{ type: 'bold' }, { type: 'italic' }]))))
    instance.chain().selectAll().increaseFontSize().run()

    const marks = instance.getJSON().content?.[0]?.content?.[0]?.marks?.map((mark) => mark.type)
    expect(marks).toEqual(expect.arrayContaining(['bold', 'italic', 'textStyle']))
  })

  it('draws the size relative to the surrounding text', () => {
    const instance = mount(doc(p(text('Hola'))))
    instance.chain().selectAll().increaseFontSize().run()

    const span = instance.view.dom.querySelector('span[style]') as HTMLElement
    expect(span.style.fontSize).toBe('1.125em')
  })

  it('is one undo step', () => {
    const instance = mount(doc(p(text('Uno')), p(text('Dos'))))
    instance.chain().selectAll().increaseFontSize().run()
    instance.chain().selectAll().increaseFontSize().run()
    instance.commands.undo()

    expect(runs(instance, 1)).toEqual([['Dos', '1.125em']])
  })
})

describe('A+ and A− at a caret', () => {
  it('sizes the text typed next', () => {
    const instance = mount(doc(p(text('Hola'))))
    instance.chain().focus().setTextSelection(5).increaseFontSize().insertContent(' mundo').run()

    expect(runs(instance)).toEqual([
      ['Hola', null],
      [' mundo', '1.125em'],
    ])
  })
})

describe('the ends of the scale', () => {
  it('cannot grow text already at the largest step', () => {
    const instance = mount(doc(p(text('máximo', [size('2em')]))))
    instance.commands.selectAll()

    expect(instance.can().increaseFontSize()).toBe(false)
    expect(instance.can().decreaseFontSize()).toBe(true)
  })

  it('cannot shrink text already at the smallest step', () => {
    const instance = mount(doc(p(text('mínimo', [size('0.75em')]))))
    instance.commands.selectAll()

    expect(instance.can().decreaseFontSize()).toBe(false)
  })

  it('can still grow a selection where only part is at the top', () => {
    const instance = mount(doc(p(text('a', [size('2em')]), text('b'))))
    instance.commands.selectAll()

    expect(instance.can().increaseFontSize()).toBe(true)
  })
})

describe('the keyboard', () => {
  /** Word's own shortcuts: Ctrl+Shift+. grows, Ctrl+Shift+, shrinks. */
  function press(instance: Editor, key: string, keyCode: number) {
    const event = new KeyboardEvent('keydown', {
      key,
      keyCode,
      ctrlKey: true,
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    })
    instance.view.dom.dispatchEvent(event)
  }

  it('grows with Mod-Shift-. and shrinks with Mod-Shift-,', () => {
    const instance = mount(doc(p(text('Hola'))))
    instance.commands.selectAll()

    press(instance, '>', 190)
    expect(runs(instance)).toEqual([['Hola', '1.125em']])
    // Not also superscript, whose shortcut is the same key without Shift.
    expect(JSON.stringify(instance.getJSON())).not.toContain('superscript')

    press(instance, '<', 188)
    press(instance, '<', 188)
    expect(runs(instance)).toEqual([['Hola', '0.875em']])
  })
})

describe('pasting', () => {
  it('keeps an em size from pasted HTML', () => {
    const instance = mount(doc(p()))
    instance.commands.setContent('<p><span style="font-size: 1.5em">grande</span> normal</p>')

    expect(runs(instance)).toEqual([
      ['grande', '1.5em'],
      [' normal', null],
    ])
  })

  it('leaves an absolute size and a style it does not know without any mark', () => {
    const instance = mount(doc(p()))
    instance.commands.setContent(
      '<p><span style="font-size: 12pt">doce</span><span style="letter-spacing: 2px">ancho</span></p>'
    )

    const marks = instance.getJSON().content?.[0]?.content?.flatMap((node) => node.marks ?? [])
    expect(marks).toEqual([])
  })
})
