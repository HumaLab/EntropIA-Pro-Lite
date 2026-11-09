import { describe, expect, it } from 'vitest'
import type { CanonicalDocument } from '../document-contract'
import {
  documentSections,
  findSection,
  restoreSection,
  sectionChanges,
  type SectionKey,
} from '../section-history'

const heading = (level: number, text: string) => ({
  type: 'heading',
  attrs: { level },
  content: [{ type: 'text', text }],
})
const paragraph = (text: string) => ({ type: 'paragraph', content: [{ type: 'text', text }] })
const doc = (...content: object[]): CanonicalDocument => ({
  schemaVersion: 1,
  doc: { type: 'doc', content },
})

const base = doc(
  paragraph('Antes del primer título'),
  heading(1, 'Introducción'),
  paragraph('El tema.'),
  heading(2, 'Fuentes'),
  paragraph('Archivos.'),
  heading(1, 'Desarrollo'),
  paragraph('El cuerpo.'),
  { type: 'footnotes', content: [] }
)

const key = (title: string, level = 1, ordinal = 0): SectionKey => ({
  title,
  level,
  occurrence: 0,
  ordinal,
})

describe('documentSections', () => {
  it('derives sections from headings, nesting subsections and leaving notes out', () => {
    const sections = documentSections(base)
    expect(sections.map((s) => [s.title, s.level, s.start, s.end])).toEqual([
      ['Introducción', 1, 1, 5],
      ['Fuentes', 2, 3, 5],
      ['Desarrollo', 1, 5, 7],
    ])
    expect(sections.map((s) => s.ordinal)).toEqual([0, 0, 1])
  })

  it('hashes only the section, so an edit elsewhere keeps its hash', () => {
    const edited = structuredClone(base)
    edited.doc.content![6] = paragraph('Otro cuerpo.')
    const [intro, , dev] = documentSections(base)
    const [intro2, , dev2] = documentSections(edited)
    expect(intro2!.hash).toBe(intro!.hash)
    expect(dev2!.hash).not.toBe(dev!.hash)
  })
})

describe('findSection', () => {
  it('finds a moved section by its title', () => {
    const moved = doc(
      heading(1, 'Desarrollo'),
      paragraph('El cuerpo.'),
      heading(1, 'Introducción'),
      paragraph('El tema.')
    )
    expect(findSection(documentSections(moved), key('Introducción'))?.start).toBe(2)
  })

  it('finds a renamed section by its position among headings of its level', () => {
    const renamed = doc(heading(1, 'Presentación'), paragraph('El tema.'), heading(1, 'Desarrollo'))
    expect(findSection(documentSections(renamed), key('Introducción', 1, 0))?.title).toBe(
      'Presentación'
    )
  })

  it('does not take another existing section for a renamed one', () => {
    // "Introducción" is gone; ordinal 0 is now "Desarrollo", which exists under
    // its own title in the base version, so it is not the renamed section.
    const removed = doc(heading(1, 'Desarrollo'), paragraph('El cuerpo.'))
    expect(findSection(documentSections(removed), key('Introducción', 1, 0), base)).toBeUndefined()
  })
})

describe('sectionChanges', () => {
  it('lists the versions where the section first appears or changes', () => {
    const v1 = base
    const v2 = structuredClone(base)
    v2.doc.content![6] = paragraph('Otro cuerpo.') // only "Desarrollo" changes
    const v3 = structuredClone(v2)
    v3.doc.content![2] = paragraph('Un tema nuevo.') // "Introducción" changes
    const versions = [
      { id: 'v1', document: v1 },
      { id: 'v2', document: v2 },
      { id: 'v3', document: v3 },
    ]
    expect(sectionChanges(versions, key('Introducción'))).toEqual(['v1', 'v3'])
    expect(sectionChanges(versions, key('Desarrollo', 1, 1))).toEqual(['v1', 'v2'])
  })
})

describe('restoreSection', () => {
  it('replaces only that section of the current document with its old content', () => {
    const current = structuredClone(base)
    current.doc.content![2] = paragraph('Tema reescrito.')
    current.doc.content![6] = paragraph('Cuerpo reescrito.')
    const restored = restoreSection(current, base, key('Introducción'))!
    expect(restored.doc.content![2]).toEqual(paragraph('El tema.'))
    expect(restored.doc.content![6]).toEqual(paragraph('Cuerpo reescrito.'))
    expect(restored.doc.content![0]).toEqual(paragraph('Antes del primer título'))
    expect(restored.doc.content!.at(-1)).toEqual({ type: 'footnotes', content: [] })
    // The current document is not mutated.
    expect(current.doc.content![2]).toEqual(paragraph('Tema reescrito.'))
  })

  it('restores a section from a version where it had another title', () => {
    // Position 0 among H1s was "Antiguo", a title the current text lacks: the
    // same section before it was renamed.
    const older = doc(heading(1, 'Antiguo'), paragraph('Viejo tema.'), heading(1, 'Desarrollo'))
    const restored = restoreSection(base, older, key('Introducción'))!
    expect(restored.doc.content![1]).toEqual(heading(1, 'Antiguo'))
    expect(restored.doc.content![2]).toEqual(paragraph('Viejo tema.'))
  })

  it('refuses when the section is missing from either document', () => {
    // No level-1 heading at all: neither by title nor by position.
    const without = doc(heading(2, 'Otro'), paragraph('x'))
    expect(restoreSection(base, without, key('Introducción'))).toBeNull()
    expect(restoreSection(without, base, key('Inexistente', 1, 5))).toBeNull()
  })
})
