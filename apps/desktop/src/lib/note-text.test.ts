import { describe, expect, it } from 'vitest'
import { plainTextOf, previewTextOf } from './note-text'

/**
 * Reading a note's words out of its markup (plan-editor.md §13).
 *
 * The defect this exists for was visible in the manuscript: linking a note
 * inserted `«<p>Esto es una carta enviada a la Unión Obrera Local.</p>»` into
 * the article, tags and all, and the results list showed the same.
 *
 * So what is asserted is that the markup goes and the *text* survives intact —
 * including the parts a regular expression over HTML gets wrong, which is why
 * this parses instead.
 */

describe('reading a note', () => {
  it('gives the words without the markup', () => {
    expect(plainTextOf('<p>Esto es una carta enviada a la Unión Obrera Local.</p>')).toBe(
      'Esto es una carta enviada a la Unión Obrera Local.'
    )
  })

  it('keeps the words that were emphasised', () => {
    expect(plainTextOf('<p>Nota sobre el <strong>Convenio Laboral</strong> de 1965.</p>')).toBe(
      'Nota sobre el Convenio Laboral de 1965.'
    )
  })

  /**
   * Two paragraphs are two paragraphs. Joining them would run the last word of
   * one into the first of the next — `local.Esto` — which reads as a typo the
   * writer did not make.
   */
  it('keeps paragraphs apart', () => {
    expect(plainTextOf('<p>Primero.</p><p>Segundo.</p>')).toBe('Primero.\n\nSegundo.')
  })

  it('keeps a line break as a line break', () => {
    expect(plainTextOf('<p>Una línea<br>y la siguiente</p>')).toBe('Una línea\ny la siguiente')
  })

  it('keeps the items of a list apart', () => {
    expect(plainTextOf('<ul><li>los metalúrgicos</li><li>los gráficos</li></ul>')).toBe(
      'los metalúrgicos\n\nlos gráficos'
    )
  })

  /**
   * Entities are characters. `&amp;` printed literally in an article is a
   * defect, not a note.
   *
   * Asserted with the entities jsdom decodes — the named basics and the numeric
   * forms — rather than with `&mdash;`, which jsdom leaves alone and the app's
   * Chromium webview resolves. Testing the difference between two HTML parsers
   * would be testing the test environment.
   */
  it('resolves entities to the characters they stand for', () => {
    expect(plainTextOf('<p>Arruzza &amp; Bhattacharya &#8212; 2020</p>')).toBe(
      'Arruzza & Bhattacharya — 2020'
    )
    expect(plainTextOf('<p>a &lt; b</p>')).toBe('a < b')
  })

  /**
   * The case that settles why this parses rather than stripping with a pattern:
   * a `>` inside an attribute value ends the tag as far as a regular expression
   * is concerned, and the rest of the tag lands in the manuscript.
   */
  it('is not fooled by a bracket inside an attribute', () => {
    expect(plainTextOf('<p title="a>b">el texto</p>')).toBe('el texto')
  })

  /** A note that is already plain text is already the answer. */
  it('leaves text that was never markup alone', () => {
    expect(plainTextOf('Esto es una nota sin etiquetas.')).toBe('Esto es una nota sin etiquetas.')
  })

  it('has nothing to say about nothing', () => {
    expect(plainTextOf('')).toBe('')
    expect(plainTextOf('<p></p>')).toBe('')
  })

  /**
   * A script tag reaches this as text, never as behaviour: `DOMParser` on
   * `text/html` builds an inert document. What matters for the caller is that
   * nothing executes and no tag survives into the article.
   */
  it('takes a script tag apart rather than running it', () => {
    const out = plainTextOf('<p>antes</p><script>alert(1)</script><p>después</p>')

    expect(out).not.toContain('<script>')
    expect(out).toContain('antes')
    expect(out).toContain('después')
  })
})

describe('a note on one line', () => {
  /** A result row is one line tall, so a break in it stops the list lining up. */
  it('folds the paragraphs into one line', () => {
    expect(previewTextOf('<p>Primero.</p><p>Segundo.</p>')).toBe('Primero. Segundo.')
  })

  it('collapses the run of spaces that leaves behind', () => {
    expect(previewTextOf('<p>Una   nota</p>\n\n<p>partida</p>')).toBe('Una nota partida')
  })
})
