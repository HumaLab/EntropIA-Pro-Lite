import { describe, expect, it } from 'vitest'
import { writingSchema } from './document-contract'
import { sectionRange, sectionWeight, siblingSection } from './sections'

/**
 * What a section *is* (plan-editor.md §6.1).
 *
 * A heading plus everything that follows it until the next heading of the same
 * level or higher. That definition is the whole feature: get it wrong and
 * "delete this section" takes the next chapter with it, or leaves its body
 * orphaned under the heading above.
 */

const text = (value: string) => ({ type: 'text', text: value })
const h = (level: number, title: string) => ({
  type: 'heading',
  attrs: { level },
  content: [text(title)],
})
const p = (body: string) => ({ type: 'paragraph', content: [text(body)] })

function docOf(content: unknown[]) {
  return writingSchema().nodeFromJSON({ type: 'doc', content })
}

/** The top-level node types a range covers, which is what the tests assert. */
function covered(doc: ReturnType<typeof docOf>, range: { from: number; to: number }) {
  const names: string[] = []
  doc.forEach((node, offset) => {
    if (offset >= range.from && offset + node.nodeSize <= range.to) {
      names.push(node.type.name === 'heading' ? `h${node.attrs.level}:${node.textContent}` : 'p')
    }
  })
  return names
}

describe('sectionRange', () => {
  it('runs from the heading to just before the next one of the same level', () => {
    const doc = docOf([h(2, 'Uno'), p('cuerpo'), h(2, 'Dos'), p('otro')])

    expect(covered(doc, sectionRange(doc, 0)!)).toEqual(['h2:Uno', 'p'])
  })

  /** An H2 owns the H3s under it: they are its subsections, not its siblings. */
  it('swallows deeper headings', () => {
    const doc = docOf([h(2, 'Uno'), h(3, 'Uno.a'), p('cuerpo'), h(2, 'Dos')])

    expect(covered(doc, sectionRange(doc, 0)!)).toEqual(['h2:Uno', 'h3:Uno.a', 'p'])
  })

  /** An H3 stops at the next H2: a shallower heading is never its child. */
  it('stops at a shallower heading', () => {
    const doc = docOf([h(2, 'Uno'), h(3, 'Uno.a'), p('cuerpo'), h(2, 'Dos'), p('fin')])

    expect(covered(doc, sectionRange(doc, 1)!)).toEqual(['h3:Uno.a', 'p'])
  })

  it('runs to the end of the document for the last section', () => {
    const doc = docOf([h(2, 'Uno'), p('a'), h(2, 'Dos'), p('b'), p('c')])

    expect(covered(doc, sectionRange(doc, 2)!)).toEqual(['h2:Dos', 'p', 'p'])
  })

  /**
   * The footnotes block belongs to the document, not to whichever section
   * happens to be last. Deleting a chapter must not take the notes with it.
   */
  it('never swallows the footnotes block', () => {
    const doc = docOf([
      h(2, 'Uno'),
      p('cuerpo'),
      {
        type: 'footnotes',
        content: [
          {
            type: 'footnote',
            attrs: { id: 'f1', 'data-id': 'f1' },
            content: [p('la nota')],
          },
        ],
      },
    ])

    expect(covered(doc, sectionRange(doc, 0)!)).toEqual(['h2:Uno', 'p'])
  })

  it('has no range for a child that is not a heading', () => {
    const doc = docOf([p('suelto'), h(2, 'Uno')])

    expect(sectionRange(doc, 0)).toBeNull()
  })
})

describe('siblingSection', () => {
  const doc = docOf([h(2, 'Uno'), p('a'), h(3, 'Uno.a'), p('b'), h(2, 'Dos'), p('c')])

  it('finds the next heading of the same level, skipping subsections', () => {
    expect(siblingSection(doc, 0, 1)).toBe(4)
  })

  it('finds the previous one', () => {
    expect(siblingSection(doc, 4, -1)).toBe(0)
  })

  it('has no sibling past either end', () => {
    expect(siblingSection(doc, 0, -1)).toBeNull()
    expect(siblingSection(doc, 4, 1)).toBeNull()
  })

  /** A subsection's siblings are the other subsections, not the chapters. */
  it('does not cross out of its parent to find one', () => {
    expect(siblingSection(doc, 2, 1)).toBeNull()
  })
})

/**
 * What a writer loses by deleting a section.
 *
 * A confirmation that says only "are you sure" is a speed bump. One that says
 * how many words go with the heading is the thing that actually makes someone
 * stop, so the count has to be right — and it has to include the subsections,
 * because those are going too.
 */
describe('sectionWeight', () => {
  it('counts the words in the heading and its body', () => {
    const doc = docOf([h(2, 'Uno dos'), p('tres cuatro cinco'), h(2, 'Otro'), p('no cuenta')])

    expect(sectionWeight(doc, 0)).toEqual({ words: 5, headings: 1 })
  })

  it('counts the subsections it would take with it', () => {
    const doc = docOf([h(2, 'Uno'), h(3, 'Uno a'), p('tres palabras aca'), h(2, 'Dos')])

    expect(sectionWeight(doc, 0)).toEqual({ words: 6, headings: 2 })
  })

  it('is nothing for a child that is not a heading', () => {
    const doc = docOf([p('suelto')])

    expect(sectionWeight(doc, 0)).toEqual({ words: 0, headings: 0 })
  })
})
