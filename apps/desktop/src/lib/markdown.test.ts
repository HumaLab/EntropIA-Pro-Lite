import { describe, expect, it } from 'vitest'
import { renderMarkdown } from './markdown'

describe('renderMarkdown', () => {
  it('returns empty string for empty input', () => {
    expect(renderMarkdown('')).toBe('')
  })

  it('escapes HTML to prevent XSS', () => {
    expect(renderMarkdown('<script>alert(1)</script>')).toBe(
      '<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>'
    )
  })

  it('escapes an inline HTML injection attempt', () => {
    expect(renderMarkdown('texto <img src=x onerror=alert(1)> fin')).toBe(
      '<p>texto &lt;img src=x onerror=alert(1)&gt; fin</p>'
    )
  })

  it('renders a paragraph', () => {
    expect(renderMarkdown('Hola mundo')).toBe('<p>Hola mundo</p>')
  })

  it('joins consecutive lines into a single paragraph', () => {
    expect(renderMarkdown('línea uno\nlínea dos')).toBe('<p>línea uno línea dos</p>')
  })

  it('renders bold text', () => {
    expect(renderMarkdown('**negrita**')).toBe('<p><strong>negrita</strong></p>')
  })

  it('renders bold with underscores', () => {
    expect(renderMarkdown('__negrita__')).toBe('<p><strong>negrita</strong></p>')
  })

  it('renders italic text', () => {
    expect(renderMarkdown('*cursiva*')).toBe('<p><em>cursiva</em></p>')
  })

  it('renders bold and italic together', () => {
    expect(renderMarkdown('**bold** y *italic*')).toBe(
      '<p><strong>bold</strong> y <em>italic</em></p>'
    )
  })

  it('renders inline code', () => {
    expect(renderMarkdown('usa `console.log`')).toBe('<p>usa <code>console.log</code></p>')
  })

  it('renders headings', () => {
    expect(renderMarkdown('# Título')).toBe('<h1>Título</h1>')
    expect(renderMarkdown('## Subtítulo')).toBe('<h2>Subtítulo</h2>')
    expect(renderMarkdown('### Sección')).toBe('<h3>Sección</h3>')
  })

  it('renders a bulleted list', () => {
    const out = renderMarkdown('- uno\n- dos\n- tres')
    expect(out).toBe('<ul><li>uno</li><li>dos</li><li>tres</li></ul>')
  })

  it('renders a bulleted list with asterisk markers', () => {
    const out = renderMarkdown('* uno\n* dos')
    expect(out).toBe('<ul><li>uno</li><li>dos</li></ul>')
  })

  it('renders an ordered list', () => {
    const out = renderMarkdown('1. primero\n2. segundo')
    expect(out).toBe('<ol><li>primero</li><li>segundo</li></ol>')
  })

  it('keeps literal bracketed citation refs as text', () => {
    const out = renderMarkdown('Los obreros mantuvieron una huelga. [2][5][8]')
    expect(out).toBe('<p>Los obreros mantuvieron una huelga. [2][5][8]</p>')
  })

  it('renders a safe http link', () => {
    const out = renderMarkdown('[sitio](https://example.com)')
    expect(out).toBe(
      '<p><a href="https://example.com" rel="noopener noreferrer" target="_blank">sitio</a></p>'
    )
  })

  it('neutralizes a javascript: link, keeping only the label', () => {
    const out = renderMarkdown('[click](javascript:alert(1))')
    expect(out).toBe('<p>click</p>')
  })

  it('separates a list and a following paragraph', () => {
    const out = renderMarkdown('- item\n\nTexto final')
    expect(out).toBe('<ul><li>item</li></ul><p>Texto final</p>')
  })

  it('renders a realistic multi-block answer', () => {
    const md = [
      'Los fragmentos documentan conflictos:',
      '',
      '- **1934: huelga.** Reclamos de pago. [2][5]',
      '',
      '- **1961: conflicto.** La **Unión Obrera** convocó asamblea. [3]',
    ].join('\n')
    expect(renderMarkdown(md)).toBe(
      '<p>Los fragmentos documentan conflictos:</p>' +
        '<ul>' +
        '<li><strong>1934: huelga.</strong> Reclamos de pago. [2][5]</li>' +
        '<li><strong>1961: conflicto.</strong> La <strong>Unión Obrera</strong> convocó asamblea. [3]</li>' +
        '</ul>'
    )
  })

  it('normalizes CRLF line endings', () => {
    expect(renderMarkdown('uno\r\ndos')).toBe('<p>uno dos</p>')
  })
})
