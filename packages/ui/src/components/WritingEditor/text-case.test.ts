import { Editor, type JSONContent } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { createWritingExtensions } from './extensions'
import { changeCase } from './text-case'

/**
 * Change case: UPPER, lower, Sentence case, Capitalize Each Word.
 *
 * The words change and nothing else does: every run keeps its own marks, the
 * atoms in the line (citations, footnote markers, note links) are not text and
 * are not touched, and the selection still covers what it covered.
 */

describe('changeCase — the rules', () => {
  it('writes everything in capitals', () => {
    expect(changeCase(['hola ', 'Mundo'], 'upper')).toEqual(['HOLA ', 'MUNDO'])
  })

  it('writes everything in lower case', () => {
    expect(changeCase(['HOLA ', 'Mundo'], 'lower')).toEqual(['hola ', 'mundo'])
  })

  it('capitalizes each sentence, including after an opening question mark', () => {
    expect(changeCase(['HOLA MUNDO. ¿QUÉ TAL? bien'], 'sentence')).toEqual([
      'Hola mundo. ¿Qué tal? Bien',
    ])
  })

  it('does not take a decimal point for the end of a sentence', () => {
    expect(changeCase(['MIDE 3.5 METROS'], 'sentence')).toEqual(['Mide 3.5 metros'])
  })

  it('capitalizes each word', () => {
    expect(changeCase(['EL puerto de mar del plata'], 'words')).toEqual([
      'El Puerto De Mar Del Plata',
    ])
  })

  it('treats a word split across two runs as one word', () => {
    expect(changeCase(['bue', 'nos aires'], 'words')).toEqual(['Bue', 'nos Aires'])
  })

  it('does not capitalize the rest of a word the selection starts inside', () => {
    expect(changeCase(['nos aires'], 'words', { afterWord: true })).toEqual(['nos Aires'])
  })

  it('passes over an atom and keeps its place', () => {
    expect(changeCase(['según ', null, ' dice'], 'words')).toEqual(['Según ', null, ' Dice'])
  })

  it('follows the locale', () => {
    expect(changeCase(['istanbul'], 'upper', { locale: 'tr' })).toEqual(['İSTANBUL'])
  })

  it('lets a capital be longer than its letter', () => {
    expect(changeCase(['straße'], 'upper', { locale: 'de' })).toEqual(['STRASSE'])
  })
})

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

describe('setTextCase — in the editor', () => {
  it('changes one word and leaves the rest of the line', () => {
    const instance = mount(doc(p(text('hola mundo'))))
    instance.chain().setTextSelection({ from: 6, to: 11 }).setTextCase('upper').run()

    expect(instance.getText()).toBe('hola MUNDO')
  })

  it('keeps every run’s own marks', () => {
    const instance = mount(
      doc(p(text('hola ', [{ type: 'bold' }]), text('mundo', [{ type: 'italic' }])))
    )
    instance.chain().selectAll().setTextCase('upper').run()

    expect(instance.getJSON().content?.[0]?.content).toEqual([
      text('HOLA ', [{ type: 'bold' }]),
      text('MUNDO', [{ type: 'italic' }]),
    ])
  })

  it('starts a sentence at every paragraph', () => {
    const instance = mount(doc(p(text('PRIMERO.')), p(text('SEGUNDO.'))))
    instance.chain().selectAll().setTextCase('sentence').run()

    expect(instance.getText({ blockSeparator: '|' })).toBe('Primero.|Segundo.')
  })

  it('leaves a citation, a footnote marker and a note link exactly as they were', () => {
    const citation = {
      type: 'zoteroCitation',
      attrs: { citationNodeId: 'z-1', renderedText: '(acha, 2015)', items: [] },
    }
    const noteLink = {
      type: 'noteLink',
      attrs: { noteLinkNodeId: 'n-1', contentSnapshot: 'una nota' },
    }
    const instance = mount(doc(p(text('según '), citation, text(' y '), noteLink, text(' fin'))))
    instance.chain().focus().setTextSelection(3).addFootnote().run()
    const before = JSON.stringify(instance.getJSON())
    const atoms = (json: string) =>
      [
        ...json.matchAll(/"type":"(zoteroCitation|noteLink|footnoteReference)","attrs":\{[^}]*\}/g),
      ].map(([match]) => match)

    instance.commands.setTextSelection({ from: 1, to: instance.state.doc.child(0).nodeSize - 1 })
    instance.commands.setTextCase('upper')

    const after = JSON.stringify(instance.getJSON())
    expect(atoms(after)).toEqual(atoms(before))
    expect(instance.state.doc.child(0).textContent).toContain('SEGÚN')
    expect(instance.state.doc.child(0).textContent).toContain(' FIN')
  })

  it('keeps the selection over the same words, even when they grow', () => {
    const instance = mount(doc(p(text('la straße larga'))))
    instance.chain().setTextSelection({ from: 4, to: 10 }).setTextCase('upper', 'de').run()

    const { from, to } = instance.state.selection
    expect(instance.state.doc.textBetween(from, to)).toBe('STRASSE')
  })

  it('is one undo step across paragraphs', () => {
    const instance = mount(doc(p(text('uno')), p(text('dos'))))
    instance.chain().selectAll().setTextCase('upper').run()
    expect(instance.getText({ blockSeparator: '|' })).toBe('UNO|DOS')

    instance.commands.undo()
    expect(instance.getText({ blockSeparator: '|' })).toBe('uno|dos')
  })

  it('does nothing at a caret', () => {
    const instance = mount(doc(p(text('hola'))))
    instance.commands.setTextSelection(3)

    expect(instance.can().setTextCase('upper')).toBe(false)
    expect(instance.commands.setTextCase('upper')).toBe(false)
    expect(instance.getText()).toBe('hola')
  })
})
