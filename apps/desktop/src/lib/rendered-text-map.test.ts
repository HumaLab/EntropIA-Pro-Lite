import { describe, expect, it } from 'vitest'
import { renderOcrHtml } from './ocr-rich-text'
import { mapRenderedText } from './rendered-text-map'

/**
 * Each case is rendered by the app's own OCR renderer, so what is tested is the
 * real pair: the text a reader sees and selects, against the raw extraction a
 * citation anchors into. The raw shapes come from a real corpus.
 */
async function visibleTextOf(raw: string): Promise<string> {
  const html = await renderOcrHtml(
    raw,
    { assetUrl: 'x', sourceType: 'image', referenceWidth: 1, referenceHeight: 1 },
    async () => 'data:image/png;base64,iVBORw0KGgo='
  )
  const box = document.createElement('div')
  box.innerHTML = html
  return box.textContent ?? ''
}

/** Selects `fragment` in the visible text and maps it back to the raw one. */
async function cite(raw: string, fragment: string) {
  const visible = await visibleTextOf(raw)
  const start = visible.indexOf(fragment)
  if (start < 0) throw new Error(`"${fragment}" is not in the rendered text: ${visible}`)
  const range = mapRenderedText(visible, raw).toRaw(start, start + fragment.length)
  return range && raw.slice(range.start, range.end)
}

/** Drags from the start of `from` to the end of `to`, as a reader would. */
async function citeSpan(raw: string, from: string, to: string) {
  const visible = await visibleTextOf(raw)
  const start = visible.indexOf(from)
  const end = visible.indexOf(to, start) + to.length
  const range = mapRenderedText(visible, raw).toRaw(start, end)
  return range && raw.slice(range.start, range.end)
}

describe('mapping a selection on rendered OCR back to the raw text', () => {
  it('anchors plain text exactly where it is', async () => {
    expect(await cite('El convenio fue firmado el 10 de marzo.', 'fue firmado')).toBe('fue firmado')
  })

  it('reaches past the HTML and heading marks the reader never sees', async () => {
    const raw =
      '<div align="center">\n\n# Convenio Laboral Para la Rama Filet\n\n</div>\n\nLa aplicación'
    expect(await cite(raw, 'Convenio Laboral')).toBe('Convenio Laboral')
  })

  it('keeps a selection across a heading and the paragraph below it', async () => {
    const raw = '# Convenio Laboral\n\nLa aplicación del convenio'
    expect(await citeSpan(raw, 'Laboral', 'aplicación')).toBe('Laboral\n\nLa aplicación')
  })

  it('includes emphasis marks inside the range, since they sit between the words', async () => {
    expect(await cite('el **dirigente** gremial', 'dirigente gremial')).toBe('dirigente** gremial')
  })

  it('skips the number of an ordered list, which is drawn as a marker, not text', async () => {
    const raw = 'Resuelve:\n\n1. Apoyar las medidas\n2. Adecuar estas medidas'
    expect(await citeSpan(raw, 'Apoyar', 'Adecuar')).toBe('Apoyar las medidas\n2. Adecuar')
  })

  it('reads a code block literally, markup included, as the renderer shows it', async () => {
    const raw = '```markdown\n![](page=0,bbox=[1, 2, 3, 4])\nORGANO DEL COMITE\n```'
    expect(await cite(raw, 'ORGANO DEL COMITE')).toBe('ORGANO DEL COMITE')
  })

  it('treats HTML inside a code block as the text it is shown as', async () => {
    const raw = '```\n<b>Nota</b> del gremio\n```'
    // A range runs from the first letter selected to the last, so the opening
    // `<` stays outside; what matters is that `b` and `Nota` map at all.
    expect(await cite(raw, '<b>Nota</b> del')).toBe('b>Nota</b> del')
  })

  // The renderer names a region inside a code block `ocr-region:region-0`. Its
  // word `region` recurs further on; jumping there on one word alone would
  // strand everything before it. The jump needs the next word to agree too.
  it('does not jump ahead on a single generated word that happens to recur', async () => {
    const raw = '```\n![](page=0,bbox=[1, 2, 3, 4])\n```\n\nInforme de la region sur. Firmado.'
    expect(await cite(raw, 'Informe de la region sur')).toBe('Informe de la region sur')
  })

  it('is not dragged forward by text the renderer generated', async () => {
    const raw =
      "la lucha!'\n\n![](page=0,bbox=[671, 312, 1106, 453])\n\nORGANO DEL COMITE DE ENLACE SINDICAL"
    expect(await cite(raw, 'COMITE DE ENLACE')).toBe('COMITE DE ENLACE')
  })

  it('decodes entities the way the reader sees them', async () => {
    expect(await cite('Luz &amp; Fuerza apoya', 'Luz & Fuerza')).toBe('Luz &amp; Fuerza')
  })

  it('finds the right occurrence of a word that repeats', async () => {
    const raw = 'Crocitto firmó. Luego Crocitto habló.'
    const visible = await visibleTextOf(raw)
    const second = visible.lastIndexOf('Crocitto')
    const range = mapRenderedText(visible, raw).toRaw(second, second + 'Crocitto habló'.length)
    expect(range).toEqual({ start: raw.lastIndexOf('Crocitto'), end: raw.length - 1 })
  })

  it('refuses rather than guess when the selection holds text the raw one does not', async () => {
    // Inside a code block the renderer shows its own name for a region,
    // `ocr-region:region-0`, which the raw text spells as page and bbox.
    const raw = '```\n![](page=0,bbox=[1, 2, 3, 4])\n```\n\ndespués'
    const visible = await visibleTextOf(raw)
    const map = mapRenderedText(visible, raw)
    const generated = visible.indexOf('ocr-region')
    expect(generated).toBeGreaterThanOrEqual(0)

    expect(map.toRaw(generated, generated + 'ocr-region'.length)).toBeNull()
    const after = visible.indexOf('después')
    expect(map.toRaw(after, after + 'después'.length)).toEqual({
      start: raw.indexOf('después'),
      end: raw.length,
    })
  })

  // The alignment would anchor only `después`, silently dropping the letters
  // before it. The letter check is what turns that into a refusal.
  it('refuses a selection that only partly maps, instead of quietly trimming it', async () => {
    const raw = '```\n![](page=0,bbox=[1, 2, 3, 4])\n```\n\ndespués'
    const visible = await visibleTextOf(raw)
    const start = visible.indexOf('region-0')

    expect(mapRenderedText(visible, raw).toRaw(start, visible.length)).toBeNull()
  })

  it('refuses a selection of nothing but blank space or punctuation', async () => {
    const raw = 'uno. — dos'
    const visible = await visibleTextOf(raw)
    const dash = visible.indexOf('—')
    expect(mapRenderedText(visible, raw).toRaw(dash, dash + 1)).toBeNull()
  })
})

describe('mapping raw OCR ranges into rendered text', () => {
  it('spans all visible words across Markdown emphasis and paragraphs', async () => {
    const raw = 'El **dirigente** gremial firmó.\n\nEl segundo párrafo confirma el acuerdo.'
    const visible = await visibleTextOf(raw)
    const rawStart = raw.indexOf('dirigente')
    const rawEnd = raw.indexOf('confirma') + 'confirma'.length

    expect(mapRenderedText(visible, raw).toVisible(rawStart, rawEnd)).toEqual({
      start: visible.indexOf('dirigente'),
      end: visible.indexOf('confirma') + 'confirma'.length,
    })
  })

  it('resolves repeated wording from the selected raw occurrence', async () => {
    const raw = 'Crocitto firmó el acuerdo. Luego Crocitto firmó el acuerdo.'
    const visible = await visibleTextOf(raw)
    const rawStart = raw.lastIndexOf('Crocitto')
    const rawEnd = rawStart + 'Crocitto firmó'.length
    const visibleStart = visible.lastIndexOf('Crocitto')

    expect(mapRenderedText(visible, raw).toVisible(rawStart, rawEnd)).toEqual({
      start: visibleStart,
      end: visibleStart + 'Crocitto firmó'.length,
    })
  })

  it('refuses invalid, empty, and unmapped raw ranges', async () => {
    const raw = 'uno **dos** tres'
    const visible = await visibleTextOf(raw)
    const map = mapRenderedText(visible, raw)
    const openingEmphasis = raw.indexOf('**')

    expect(map.toVisible(-1, 3)).toBeNull()
    expect(map.toVisible(4, 4)).toBeNull()
    expect(map.toVisible(openingEmphasis, openingEmphasis + 2)).toBeNull()
  })
})
