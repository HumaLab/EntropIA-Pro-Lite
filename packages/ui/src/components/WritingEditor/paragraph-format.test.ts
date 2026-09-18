import { Editor, type JSONContent } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { createWritingExtensions } from './extensions'
import {
  INDENT_MAX,
  LINE_HEIGHTS,
  paragraphFormatOf,
  parseIndent,
  parseLineHeight,
  parseTextAlign,
} from './paragraph-format'

/**
 * Paragraph formatting: alignment, indent and line spacing. Each is a block
 * attribute on paragraphs and headings, acts on every block the selection
 * touches, and is one undo step. Inside a list, indent moves the item instead.
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
const p = (value?: string, attrs?: Record<string, unknown>): JSONContent => ({
  type: 'paragraph',
  ...(attrs ? { attrs } : {}),
  ...(value ? { content: [{ type: 'text', text: value }] } : {}),
})
const h = (level: number, value: string, attrs: Record<string, unknown> = {}): JSONContent => ({
  type: 'heading',
  attrs: { level, ...attrs },
  content: [{ type: 'text', text: value }],
})
const li = (...content: JSONContent[]): JSONContent => ({ type: 'listItem', content })
const ul = (...items: JSONContent[]): JSONContent => ({ type: 'bulletList', content: items })

/** One attribute of every top-level block, in order. */
function attrOf(instance: Editor, name: string): unknown[] {
  return (instance.getJSON().content ?? []).map((block) => block.attrs?.[name] ?? null)
}

/** Positions: "Uno" is 1–4 in the first paragraph, "Dos" 6–9 in the second. */
const TWO = () => doc(p('Uno'), p('Dos'))

describe('the stored values', () => {
  it('has a fixed set of line heights, stored as unitless numbers', () => {
    expect(LINE_HEIGHTS).toEqual(['1', '1.15', '1.5', '2'])
    expect(parseLineHeight('1.5')).toBe('1.5')
    expect(parseLineHeight('3')).toBeNull()
    expect(parseLineHeight('1.5em')).toBeNull()
    expect(parseLineHeight(1.5)).toBeNull()
    expect(parseLineHeight(null)).toBeNull()
  })

  it('has integer indent levels up to a maximum, with 0 as none', () => {
    expect(INDENT_MAX).toBe(8)
    expect(parseIndent(3)).toBe(3)
    expect(parseIndent('2')).toBe(2)
    expect(parseIndent(0)).toBe(0)
    expect(parseIndent(9)).toBe(0)
    expect(parseIndent(1.5)).toBe(0)
    expect(parseIndent('x')).toBe(0)
    expect(parseIndent(null)).toBe(0)
  })

  it('reads left as the default alignment, stored as nothing', () => {
    expect(parseTextAlign('center')).toBe('center')
    expect(parseTextAlign('justify')).toBe('justify')
    expect(parseTextAlign('left')).toBeNull()
    expect(parseTextAlign('start')).toBeNull()
    expect(parseTextAlign(null)).toBeNull()
  })
})

describe('alignment', () => {
  it('aligns the whole paragraph from a word inside it, keeping the selection', () => {
    const instance = mount(doc(p('Hola mundo')))
    instance.chain().setTextSelection({ from: 6, to: 11 }).setTextAlign('center').run()

    expect(attrOf(instance, 'textAlign')).toEqual(['center'])
    expect(instance.state.selection.from).toBe(6)
    expect(instance.state.selection.to).toBe(11)
  })

  it('aligns every block the selection touches, headings included', () => {
    const instance = mount(doc(h(2, 'Título'), p('Uno'), p('Dos')))
    instance.chain().selectAll().setTextAlign('justify').run()

    expect(attrOf(instance, 'textAlign')).toEqual(['justify', 'justify', 'justify'])
  })

  it('aligns the paragraph at an empty caret', () => {
    const instance = mount(TWO())
    instance.chain().setTextSelection(7).setTextAlign('right').run()

    expect(attrOf(instance, 'textAlign')).toEqual([null, 'right'])
  })

  it('stores left as no attribute, taking an alignment off', () => {
    const instance = mount(doc(p('Uno', { textAlign: 'center' })))
    instance.chain().selectAll().setTextAlign('left').run()

    expect(attrOf(instance, 'textAlign')).toEqual([null])
  })

  it('refuses an alignment outside the set', () => {
    const instance = mount(doc(p('Uno')))
    expect(instance.chain().selectAll().setTextAlign('start').run()).toBe(false)
    expect(attrOf(instance, 'textAlign')).toEqual([null])
  })

  it('reports the alignment of the selection, left when none, nothing when mixed', () => {
    const instance = mount(doc(p('Uno', { textAlign: 'center' }), p('Dos')))

    instance.commands.setTextSelection(2)
    expect(paragraphFormatOf(instance.state).alignment).toBe('center')
    instance.commands.setTextSelection(7)
    expect(paragraphFormatOf(instance.state).alignment).toBe('left')
    instance.commands.selectAll()
    expect(paragraphFormatOf(instance.state).alignment).toBeNull()
  })

  it('centres with Mod-Shift-E, the extension’s own shortcut', () => {
    const instance = mount(doc(p('Uno')))
    instance.commands.setTextSelection(2)
    // A real keyboard reports the key code too; ProseMirror reads the
    // unshifted letter from it.
    instance.view.dom.dispatchEvent(
      new KeyboardEvent('keydown', {
        key: 'E',
        keyCode: 69,
        ctrlKey: true,
        shiftKey: true,
        bubbles: true,
      })
    )

    expect(attrOf(instance, 'textAlign')).toEqual(['center'])
  })

  it('is one undo step', () => {
    const instance = mount(TWO())
    instance.chain().selectAll().setTextAlign('center').run()
    instance.commands.undo()

    expect(attrOf(instance, 'textAlign')).toEqual([null, null])
  })
})

describe('indent outside lists', () => {
  it('steps the paragraph in and out by whole levels', () => {
    const instance = mount(doc(p('Uno')))
    instance.commands.setTextSelection(2)

    instance.commands.increaseIndent()
    instance.commands.increaseIndent()
    expect(attrOf(instance, 'indent')).toEqual([2])

    instance.commands.decreaseIndent()
    expect(attrOf(instance, 'indent')).toEqual([1])
    instance.commands.decreaseIndent()
    expect(attrOf(instance, 'indent')).toEqual([null])
  })

  it('stops at 0 and at the maximum', () => {
    const instance = mount(doc(p('Uno'), p('Dos', { indent: INDENT_MAX })))

    instance.commands.setTextSelection(2)
    expect(instance.can().decreaseIndent()).toBe(false)
    expect(instance.can().increaseIndent()).toBe(true)

    instance.commands.setTextSelection(7)
    expect(instance.can().increaseIndent()).toBe(false)
    expect(instance.commands.increaseIndent()).toBe(false)
    expect(attrOf(instance, 'indent')).toEqual([null, INDENT_MAX])
  })

  it('steps each selected block from its own level', () => {
    const instance = mount(doc(h(1, 'Título'), p('Uno', { indent: 2 }), p('Dos')))
    instance.chain().selectAll().increaseIndent().run()
    expect(attrOf(instance, 'indent')).toEqual([1, 3, 1])

    instance.chain().selectAll().decreaseIndent().decreaseIndent().run()
    expect(attrOf(instance, 'indent')).toEqual([null, 1, null])
  })

  it('keeps the selection and is one undo step', () => {
    const instance = mount(TWO())
    instance.chain().setTextSelection({ from: 2, to: 8 }).increaseIndent().run()

    expect(attrOf(instance, 'indent')).toEqual([1, 1])
    expect([instance.state.selection.from, instance.state.selection.to]).toEqual([2, 8])
    instance.commands.undo()
    expect(attrOf(instance, 'indent')).toEqual([null, null])
  })

  it('works on an empty paragraph', () => {
    const instance = mount(doc(p()))
    instance.commands.setTextSelection(1)
    instance.commands.increaseIndent()

    expect(attrOf(instance, 'indent')).toEqual([1])
  })

  it('leaves Tab alone outside a list', () => {
    const instance = mount(doc(p('Uno')))
    instance.commands.setTextSelection(2)
    instance.view.dom.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }))

    expect(attrOf(instance, 'indent')).toEqual([null])
  })
})

describe('indent inside lists', () => {
  /** "Uno" is at 3, "Dos" at 10 inside the list. */
  const LIST = () => doc(ul(li(p('Uno')), li(p('Dos'))))

  const nested = (instance: Editor) =>
    instance.getJSON().content?.[0]?.content?.[0]?.content?.[1]?.type ?? null

  it('sinks the item into a nested list, and lifts it back', () => {
    const instance = mount(LIST())
    instance.commands.setTextSelection(11)

    expect(instance.commands.increaseIndent()).toBe(true)
    expect(nested(instance)).toBe('bulletList')

    expect(instance.commands.decreaseIndent()).toBe(true)
    expect(nested(instance)).toBeNull()
    expect(instance.getJSON().content?.[0]?.content).toHaveLength(2)
  })

  it('cannot sink the first item, which has nothing to go under', () => {
    const instance = mount(LIST())
    instance.commands.setTextSelection(3)

    expect(instance.can().increaseIndent()).toBe(false)
    expect(instance.can().decreaseIndent()).toBe(true)
  })

  it('never puts an indent attribute on a list paragraph', () => {
    const instance = mount(LIST())
    instance.commands.setTextSelection(11)
    instance.commands.increaseIndent()

    expect(JSON.stringify(instance.getJSON())).not.toMatch(/"indent":\d/)
  })

  it('keeps Tab and Shift-Tab moving list items, as before', () => {
    const instance = mount(LIST())
    instance.commands.setTextSelection(11)
    instance.view.dom.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }))
    expect(nested(instance)).toBe('bulletList')

    instance.view.dom.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Tab', shiftKey: true, bubbles: true })
    )
    expect(nested(instance)).toBeNull()
  })
})

describe('line spacing', () => {
  it('sets every selected block, headings included', () => {
    const instance = mount(doc(h(1, 'Título'), p('Uno'), p('Dos')))
    instance.chain().selectAll().setLineHeight('1.5').run()

    expect(attrOf(instance, 'lineHeight')).toEqual(['1.5', '1.5', '1.5'])
  })

  it('sets the paragraph at the caret, and takes it off with null', () => {
    const instance = mount(TWO())
    instance.chain().setTextSelection(7).setLineHeight('2').run()
    expect(attrOf(instance, 'lineHeight')).toEqual([null, '2'])

    instance.chain().setTextSelection(7).setLineHeight(null).run()
    expect(attrOf(instance, 'lineHeight')).toEqual([null, null])
  })

  it('refuses a value outside the set', () => {
    const instance = mount(doc(p('Uno')))
    expect(
      instance
        .chain()
        .selectAll()
        .setLineHeight('3' as never)
        .run()
    ).toBe(false)
    expect(attrOf(instance, 'lineHeight')).toEqual([null])
  })

  it('reports the value of the selection: default when none, nothing when mixed', () => {
    const instance = mount(doc(p('Uno', { lineHeight: '1.15' }), p('Dos')))

    instance.commands.setTextSelection(2)
    expect(paragraphFormatOf(instance.state).lineHeight).toBe('1.15')
    instance.commands.setTextSelection(7)
    expect(paragraphFormatOf(instance.state).lineHeight).toBe('default')
    instance.commands.selectAll()
    expect(paragraphFormatOf(instance.state).lineHeight).toBeNull()
  })

  it('is one undo step', () => {
    const instance = mount(TWO())
    instance.chain().selectAll().setLineHeight('1.15').run()
    instance.commands.undo()

    expect(attrOf(instance, 'lineHeight')).toEqual([null, null])
  })
})

/**
 * A footnote is written out as one run of prose in every format, so a block
 * attribute inside one would be lost on export. The tools leave it alone.
 */
describe('inside a footnote', () => {
  /** "Uno" at 1–4; the note's paragraph text starts at 8. */
  const NOTED = () =>
    doc(p('Uno'), {
      type: 'footnotes',
      content: [{ type: 'footnote', attrs: { 'data-id': 'f1' }, content: [p('nota')] }],
    })

  const noteAttrs = (instance: Editor) =>
    instance.getJSON().content?.[1]?.content?.[0]?.content?.[0]?.attrs

  it('neither aligns, indents nor spaces a footnote paragraph', () => {
    const instance = mount(NOTED())
    instance.commands.setTextSelection(9)
    expect(instance.state.selection.$from.parent.textContent).toBe('nota')

    expect(instance.commands.setTextAlign('center')).toBe(false)
    expect(instance.commands.setLineHeight('2')).toBe(false)
    expect(instance.can().increaseIndent()).toBe(false)
    expect(noteAttrs(instance)).toEqual({ textAlign: null, indent: null, lineHeight: null })
    expect(paragraphFormatOf(instance.state).applicable).toBe(false)
  })

  it('still formats the body of a document that has notes', () => {
    const instance = mount(NOTED())
    instance.chain().setTextSelection(2).setTextAlign('right').run()

    expect(attrOf(instance, 'textAlign')[0]).toBe('right')
    expect(noteAttrs(instance)?.textAlign).toBeNull()
    expect(paragraphFormatOf(instance.state).applicable).toBe(true)
  })
})

describe('rendering and pasting', () => {
  it('draws each attribute on the block, and nothing for the defaults', () => {
    const instance = mount(
      doc(p('Uno', { textAlign: 'justify', indent: 2, lineHeight: '1.5' }), p('Dos'))
    )
    const [first, second] = [...instance.view.dom.querySelectorAll('p')] as HTMLElement[]

    expect(first!.style.textAlign).toBe('justify')
    expect(first!.style.lineHeight).toBe('1.5')
    expect(first!.dataset.indent).toBe('2')
    expect(first!.style.getPropertyValue('--writing-indent')).toBe('2')
    expect(second!.hasAttribute('style')).toBe(false)
    expect(second!.hasAttribute('data-indent')).toBe(false)
  })

  it('draws a value it does not know as the default, and keeps it', () => {
    const instance = mount(doc(p('Uno', { lineHeight: '3', indent: 12, textAlign: 'start' })))
    const block = instance.view.dom.querySelector('p') as HTMLElement

    expect(block.hasAttribute('style')).toBe(false)
    expect(block.hasAttribute('data-indent')).toBe(false)
    expect(instance.getJSON().content?.[0]?.attrs).toEqual({
      textAlign: 'start',
      indent: 12,
      lineHeight: '3',
    })
  })

  it('keeps all three through a copy from this editor', () => {
    const source = mount(
      doc(
        h(2, 'Título', { textAlign: 'center', lineHeight: '2', indent: 1 }),
        p('Uno', { textAlign: 'right', indent: 3, lineHeight: '1.15' })
      )
    )
    const html = source.getHTML()
    const before = source.getJSON()
    source.destroy()

    const target = mount(doc(p()))
    target.commands.setContent(html)

    expect(target.getJSON()).toEqual(before)
  })

  it('ignores values from another program it has no step for', () => {
    const instance = mount(doc(p()))
    instance.commands.setContent(
      '<p style="text-align: left; line-height: 3" data-indent="12">a</p>' +
        '<p style="text-align: start; line-height: 24px">b</p>'
    )

    expect(instance.getJSON().content?.map((block) => block.attrs)).toEqual([
      { textAlign: null, indent: null, lineHeight: null },
      { textAlign: null, indent: null, lineHeight: null },
    ])
  })
})
