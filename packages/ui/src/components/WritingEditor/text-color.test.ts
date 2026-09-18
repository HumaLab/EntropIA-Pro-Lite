import { Editor, type JSONContent } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { createWritingExtensions } from './extensions'

/**
 * Text colour: a palette name on the `textStyle` mark, drawn through the
 * theme's own token so a colour chosen on one theme stays readable on the
 * others (writing-colors.ts).
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
const style = (attrs: Record<string, unknown>) => ({ type: 'textStyle', attrs })

/** A paragraph's runs as [text, colour | null]. */
function runs(instance: Editor, paragraph = 0): [string, unknown][] {
  const block = instance.getJSON().content?.[paragraph]
  return (block?.content ?? []).map((node) => {
    const mark = node.marks?.find((entry) => entry.type === 'textStyle')
    return [node.text ?? '', mark?.attrs?.color ?? null]
  })
}

describe('colouring a selection', () => {
  it('colours one word and leaves the rest of the line alone', () => {
    const instance = mount(doc(p(text('Hola mundo'))))
    instance.chain().setTextSelection({ from: 6, to: 11 }).setTextColor('red').run()

    expect(runs(instance)).toEqual([
      ['Hola ', null],
      ['mundo', 'red'],
    ])
  })

  it('colours a phrase across paragraphs', () => {
    const instance = mount(doc(p(text('Uno')), p(text('Dos'))))
    instance.chain().selectAll().setTextColor('blue').run()

    expect(runs(instance, 0)).toEqual([['Uno', 'blue']])
    expect(runs(instance, 1)).toEqual([['Dos', 'blue']])
  })

  it('recolours text that already had a colour', () => {
    const instance = mount(doc(p(text('rojo', [style({ color: 'red' })]))))
    instance.chain().selectAll().setTextColor('green').run()

    expect(runs(instance)).toEqual([['rojo', 'green']])
  })

  it('keeps a font size and the other marks on the text it colours', () => {
    const instance = mount(doc(p(text('grande', [style({ fontSize: '1.5em' }), { type: 'bold' }]))))
    instance.chain().selectAll().setTextColor('purple').run()

    const node = instance.getJSON().content?.[0]?.content?.[0]
    expect(node?.marks).toEqual([
      { type: 'textStyle', attrs: { fontSize: '1.5em', color: 'purple' } },
      { type: 'bold' },
    ])
  })

  it('keeps the colour when the size changes', () => {
    const instance = mount(doc(p(text('azul', [style({ color: 'blue' })]))))
    instance.chain().selectAll().increaseFontSize().run()

    expect(instance.getJSON().content?.[0]?.content?.[0]?.marks).toEqual([
      { type: 'textStyle', attrs: { fontSize: '1.125em', color: 'blue' } },
    ])
  })

  it('refuses a name that is not in the palette', () => {
    const instance = mount(doc(p(text('Hola'))))
    instance.commands.selectAll()

    expect(instance.commands.setTextColor('#ff0000' as never)).toBe(false)
    expect(runs(instance)).toEqual([['Hola', null]])
  })

  it('is one undo step', () => {
    const instance = mount(doc(p(text('Uno')), p(text('Dos'))))
    instance.chain().selectAll().setTextColor('red').run()
    instance.chain().selectAll().setTextColor('blue').run()
    instance.commands.undo()

    expect(runs(instance, 0)).toEqual([['Uno', 'red']])
    expect(runs(instance, 1)).toEqual([['Dos', 'red']])
  })
})

describe('removing the colour', () => {
  it('drops the mark altogether when nothing else was on it', () => {
    const instance = mount(doc(p(text('rojo', [style({ color: 'red' }), { type: 'italic' }]))))
    instance.chain().selectAll().unsetTextColor().run()

    expect(instance.getJSON().content?.[0]?.content?.[0]?.marks).toEqual([{ type: 'italic' }])
  })

  it('keeps the size when there is one', () => {
    const instance = mount(doc(p(text('rojo', [style({ color: 'red', fontSize: '2em' })]))))
    instance.chain().selectAll().unsetTextColor().run()

    expect(instance.getJSON().content?.[0]?.content?.[0]?.marks).toEqual([
      { type: 'textStyle', attrs: { fontSize: '2em', color: null } },
    ])
  })
})

describe('at a caret', () => {
  it('colours the text typed next', () => {
    const instance = mount(doc(p(text('Hola'))))
    instance.chain().focus('end').setTextColor('orange').insertContent(' mundo').run()

    expect(runs(instance)).toEqual([
      ['Hola', null],
      [' mundo', 'orange'],
    ])
  })

  it('stops colouring the text typed next', () => {
    const instance = mount(doc(p(text('Hola', [style({ color: 'red' })]))))
    instance.chain().focus('end').unsetTextColor().insertContent(' mundo').run()

    expect(runs(instance)).toEqual([
      ['Hola', 'red'],
      [' mundo', null],
    ])
    // No empty textStyle waits behind: the new run carries no mark at all.
    expect(instance.getJSON().content?.[0]?.content?.[1]?.marks).toBeUndefined()
  })
})

describe('drawing it', () => {
  it('draws a name through its theme token', () => {
    const instance = mount(doc(p(text('rojo', [style({ color: 'red' })]))))

    const span = instance.view.dom.querySelector('span[data-text-color]') as HTMLElement
    expect(span.dataset.textColor).toBe('red')
    expect(span.getAttribute('style')).toContain('color: var(--writing-text-red)')
  })

  it('draws a name it does not know as no colour, and keeps it in the document', () => {
    const instance = mount(doc(p(text('futuro', [style({ color: 'chartreuse' })]))))

    expect(instance.view.dom.querySelector('[data-text-color]')).toBeNull()
    expect(instance.view.dom.innerHTML).not.toContain('chartreuse')
    expect(runs(instance)).toEqual([['futuro', 'chartreuse']])
  })
})

describe('pasting', () => {
  it('keeps a colour copied from this editor', () => {
    const source = mount(doc(p(text('rojo', [style({ color: 'red', fontSize: '1.5em' })]))))
    const html = source.getHTML()
    source.destroy()

    const target = mount(doc(p()))
    target.commands.setContent(html)

    expect(target.getJSON().content?.[0]?.content?.[0]?.marks).toEqual([
      { type: 'textStyle', attrs: { fontSize: '1.5em', color: 'red' } },
    ])
  })

  it('reads the token alone as well as the data attribute', () => {
    const instance = mount(doc(p()))
    instance.commands.setContent(
      '<p><span style="color: var(--writing-text-green)">verde</span></p>'
    )

    expect(runs(instance)).toEqual([['verde', 'green']])
  })

  /**
   * A colour from another program is a hex picked on someone else's page.
   * Snapping it to the nearest name would guess at intent and could put a
   * near-invisible colour in the manuscript, so it is dropped instead.
   */
  it('ignores a colour pasted from another program', () => {
    const instance = mount(doc(p()))
    instance.commands.setContent(
      '<p><span style="color: #ff0000">rojo</span> <span style="color: red">otro</span></p>'
    )

    const marks = instance.getJSON().content?.[0]?.content?.flatMap((node) => node.marks ?? [])
    expect(marks).toEqual([])
  })
})
