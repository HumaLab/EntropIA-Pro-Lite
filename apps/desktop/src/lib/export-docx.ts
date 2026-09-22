import {
  AlignmentType,
  BorderStyle,
  CommentRangeEnd,
  CommentRangeStart,
  CommentReference,
  Document,
  ExternalHyperlink,
  FootnoteReferenceRun,
  HeadingLevel,
  ImageRun,
  LevelFormat,
  LineRuleType,
  Packer,
  Paragraph,
  ShadingType,
  Table,
  TableCell,
  TableRow,
  TextRun,
  WidthType,
  type ICommentOptions,
  type ParagraphChild,
} from 'docx'
import { PRINT_COLORS, isLongQuote, parseFontSize, parseWritingColor } from '@entropia/ui'
import {
  aroundFragment,
  quotedPartsOf,
  renderCorpusCitation,
  renderNoteLink,
} from './export-citations'
import { isBlockQuoteParagraph } from './export-markdown'
import type { ExportContext, ExportImage, Node } from './export-document'
import {
  blockFormatOf,
  childrenOf,
  footnoteBodies,
  needsTitleHeading,
  textOf,
  zoteroTextOf,
} from './export-document'
import { safeHref } from './export-html'

/**
 * DOCX export (plan-editor.md §17.1, §17.3).
 *
 * # Why the object model and not HTML in a wrapper
 *
 * Spike S4 opened the incumbent, `html-docx-js`, and found no document model at
 * all: its `word/document.xml` is 2 KB of namespaces wrapping an `altChunk`,
 * and the content sits in `word/afchunk.mht`. What a reader sees is produced by
 * the application opening the file, not by the file — and there is no
 * `footnotes.xml`, which §17.1 requires outright.
 *
 * `docx` 9.7.1 builds the real parts. S4 verified all twelve elements of the
 * matrix in Word, including the one that distinguishes real footnotes from
 * cosmetic markers: deleting the first note promoted the second from ² to ¹.
 *
 * # Why the bar here is higher than for the other two formats
 *
 * §17.4: *"Una advertencia no permite declarar cumplido un elemento obligatorio
 * que DOCX deba conservar."* Markdown may stand something in and say so;
 * §17.1's own "en la medida admitida por cada formato" grants it that. DOCX has
 * no such excuse, because S4 established the format admits all of it.
 */

/** The list definition, referenced by every ordered list in the document. */
const NUMBERING_REFERENCE = 'entropia-ordered'

const HEADINGS = [
  HeadingLevel.HEADING_1,
  HeadingLevel.HEADING_2,
  HeadingLevel.HEADING_3,
  HeadingLevel.HEADING_4,
  HeadingLevel.HEADING_5,
  HeadingLevel.HEADING_6,
]

interface Build {
  context: ExportContext
  /** Footnote bodies by their `data-id`, built once and referenced by number. */
  bodies: Map<string, Paragraph[]>
  /** The footnotes, in the shape `Document` wants them. */
  footnotes: Record<number, { children: Paragraph[] }>
  /** Comments, for the §17.2 representation that uses them. */
  comments: ICommentOptions[]
  next: { footnote: number; comment: number }
}

/**
 * The manuscript's type scale, in half-points: body 12 pt, H1 16, H2 14, H3 13,
 * H4 12 in bold. The package ships no body size (Word then shows 10 pt) and
 * heading styles that reach the body's size by H3, so both are set here. An em
 * in a heading is a proportion of the heading, as on screen. Footnotes keep the
 * package's 10 pt, which is the academic convention.
 */
const BODY_HALF_POINTS = 24
const HEADING_HALF_POINTS = [32, 28, 26, 24]

/**
 * Space around blocks, in twentieths of a point: 6 pt before and after a
 * paragraph, 12 pt before and 6 pt after a heading. A paragraph that sets its
 * own line spacing writes only `w:line`, so it still inherits these.
 */
const PARAGRAPH_SPACING = { before: 120, after: 120 }
const HEADING_SPACING = { before: 240, after: 120 }

/**
 * The heading styles, with the colours the package gives them: it replaces a
 * style's whole `run` with whatever is passed, so the colour has to come along.
 */
const HEADING_STYLES = {
  heading1: {
    run: { size: HEADING_HALF_POINTS[0], color: '2E74B5' },
    paragraph: { spacing: HEADING_SPACING },
  },
  heading2: {
    run: { size: HEADING_HALF_POINTS[1], color: '2E74B5' },
    paragraph: { spacing: HEADING_SPACING },
  },
  heading3: {
    run: { size: HEADING_HALF_POINTS[2], color: '1F4D78' },
    paragraph: { spacing: HEADING_SPACING },
  },
  heading4: {
    run: { size: HEADING_HALF_POINTS[3], color: '2E74B5', bold: true },
    paragraph: { spacing: HEADING_SPACING },
  },
}

/**
 * The package's footnote style, restated because passing any `paragraph`
 * replaces its own: single spacing and nothing after, and nothing before
 * either, which it would otherwise inherit from the body's 6 pt.
 */
const FOOTNOTE_TEXT = {
  paragraph: {
    // 240 is single spacing (SINGLE_LINE, declared further down).
    spacing: { before: 0, after: 0, line: 240, lineRule: LineRuleType.AUTO },
  },
  run: { size: 20 },
}

function baseOfHeading(level: number): number {
  return HEADING_HALF_POINTS[level - 1] ?? BODY_HALF_POINTS
}

/**
 * A palette name as the hex OOXML wants (no `#`), from the print table the
 * other exporters use. A highlight goes out as shading, not as `w:highlight`:
 * Word's named highlights are a fixed set of sixteen saturated colours, and
 * snapping to the nearest would print something the writer never picked.
 */
function printHex(value: string): string {
  return value.replace('#', '').toUpperCase()
}

function colorsOf(node: Node) {
  const style = (node.marks ?? []).find((mark) => mark.type === 'textStyle')
  const marker = (node.marks ?? []).find((mark) => mark.type === 'highlight')
  const ink = parseWritingColor(style?.attrs?.color)
  const paper = marker
    ? parseWritingColor(marker.attrs?.color === undefined ? 'yellow' : marker.attrs.color)
    : null
  return {
    color: ink === null ? undefined : printHex(PRINT_COLORS[ink].text),
    shading:
      paper === null
        ? undefined
        : { type: ShadingType.CLEAR, color: 'auto', fill: printHex(PRINT_COLORS[paper].highlight) },
  }
}

function styleOf(
  node: Node,
  base = BODY_HALF_POINTS,
  { forceItalics = false, sizeFallback }: { forceItalics?: boolean; sizeFallback?: number } = {}
) {
  const marks = new Set((node.marks ?? []).map((mark) => mark.type))
  const style = (node.marks ?? []).find((mark) => mark.type === 'textStyle')
  const size = parseFontSize(style?.attrs?.fontSize)
  return {
    ...colorsOf(node),
    bold: marks.has('bold'),
    italics: forceItalics || marks.has('italic'),
    strike: marks.has('strike'),
    underline: marks.has('underline') ? {} : undefined,
    font: marks.has('code') ? 'Consolas' : undefined,
    subScript: marks.has('subscript') || undefined,
    superScript: marks.has('superscript') || undefined,
    size: size === null ? sizeFallback : Math.round(base * size),
  }
}

/** One indent level: half an inch, Word's own step for its indent buttons. */
const INDENT_STEP_TWIPS = 720
/** A blockquote's own left indent, which an indent level adds to. */
const QUOTE_INDENT_TWIPS = 567
/** Single line spacing, in the 240ths of a line `w:line` counts in with the auto rule. */
const SINGLE_LINE = 240

const ALIGNMENT = {
  center: AlignmentType.CENTER,
  right: AlignmentType.RIGHT,
  // OOXML's `both`: justified on both sides, the last line left.
  justify: AlignmentType.JUSTIFIED,
} as const

/** A manuscript image's own alignment (I7) — `left` included, unlike
 *  `ALIGNMENT` above: a paragraph's unset alignment already reads as left in
 *  Word, but a writingImage's `align` defaults to `'center'` (schema), so
 *  `left` has to be written explicitly rather than left implicit. */
const WRITING_IMAGE_ALIGNMENT: Record<string, (typeof AlignmentType)[keyof typeof AlignmentType]> = {
  left: AlignmentType.LEFT,
  center: AlignmentType.CENTER,
  right: AlignmentType.RIGHT,
}

/**
 * A paragraph's or heading's formatting as paragraph properties: alignment,
 * left indent and line spacing. `quoted` adds the blockquote's own indent, so
 * an indented quotation steps from the quotation, not from the margin.
 */
function paragraphFormat(node: Node, { quoted = false, listed = false } = {}) {
  const { alignment, indent, lineHeight } = blockFormatOf(node)
  const level = listed ? 0 : indent
  const left = (quoted ? QUOTE_INDENT_TWIPS : 0) + level * INDENT_STEP_TWIPS
  return {
    ...(alignment === null ? {} : { alignment: ALIGNMENT[alignment] }),
    ...(left === 0 ? {} : { indent: { left } }),
    ...(lineHeight === null
      ? {}
      : {
          spacing: {
            line: Math.round(Number(lineHeight) * SINGLE_LINE),
            lineRule: LineRuleType.AUTO,
          },
        }),
  }
}

/** Adds a footnote and returns the run that points at it. */
function footnote(build: Build, body: Paragraph[]): FootnoteReferenceRun {
  const id = build.next.footnote++
  build.footnotes[id] = { children: body.length > 0 ? body : [new Paragraph('')] }
  return new FootnoteReferenceRun(id)
}

/**
 * Adds a real Office Open XML comment around a piece of text.
 *
 * S4 confirmed `docx` emits `comments.xml`, which is what makes §17.2's
 * "comentario, cuando el formato y la biblioteca lo permitan" reachable at all.
 * The range has to be opened and closed around something, so it wraps the
 * citation's own text rather than nothing.
 */
function comment(build: Build, body: string, anchor: ParagraphChild[]): ParagraphChild[] {
  const id = build.next.comment++
  build.comments.push({
    id,
    author: 'EntropIA',
    date: new Date(),
    children: [new Paragraph(body)],
  })
  return [
    new CommentRangeStart(id),
    ...anchor,
    new CommentRangeEnd(id),
    new TextRun({ children: [new CommentReference(id)] }),
  ]
}

/** `base` is the paragraph's own size, in half-points, for relative sizes. */
function inline(
  nodes: Node[],
  build: Build,
  base = BODY_HALF_POINTS,
  runStyle: { forceItalics?: boolean; sizeFallback?: number } = {}
): ParagraphChild[] {
  return nodes.flatMap((node): ParagraphChild[] => {
    switch (node.type) {
      case 'text': {
        const value = node.text ?? ''
        const link = (node.marks ?? []).find((mark) => mark.type === 'link')
        const href = link ? safeHref(link.attrs?.href) : null
        // A refused target keeps its words, exactly as in HTML: dropping them
        // would delete prose the writer wrote.
        if (!href) return [new TextRun({ text: value, ...styleOf(node, base, runStyle) })]

        // `ExternalHyperlink` alone emits a live link in a plain run, so the
        // reader gets something that works and looks like body text — nobody
        // clicks what they cannot see is clickable. The `Hyperlink` character
        // style is already in the package's `styles.xml` (blue, underlined);
        // this is what references it.
        return [
          new ExternalHyperlink({
            children: [
              new TextRun({ text: value, style: 'Hyperlink', ...styleOf(node, base, runStyle) }),
            ],
            link: href,
          }),
        ]
      }

      case 'hardBreak':
        return [new TextRun({ break: 1 })]

      case 'footnoteReference': {
        const id = typeof node.attrs?.['data-id'] === 'string' ? node.attrs['data-id'] : ''
        return [footnote(build, build.bodies.get(id) ?? [])]
      }

      case 'documentCitation': {
        const rendered = renderCorpusCitation(node.attrs ?? {}, build.context.citations)
        // A quote that took in an image draws it where it stood. The bytes go
        // into the package, as Word requires: a .docx is a zip with its own
        // media, not a document that reaches back into the archive.
        // A long quotation stands in the body as a block, and reads a point
        // smaller there. In a footnote it is already at the footnote's size,
        // so nothing is set: the note would end up smaller than its own text.
        const smaller = isLongQuote(
          typeof node.attrs?.quotedText === 'string' ? node.attrs.quotedText : ''
        )
        const say = (value: string, size?: number): ParagraphChild[] => {
          const around = aroundFragment(value, rendered.fragment)
          const parts = quotedPartsOf(node.attrs ?? {})
          const drawable =
            parts !== null &&
            parts.every(
              (part) => part.kind === 'text' || drawnImage(build.context.images?.[part.source])
            )
          if (!around || !parts || !drawable) return said(value, size)

          return [
            ...said(`${around[0]}«`, size),
            ...parts.flatMap((part, index): ParagraphChild[] => {
              if (part.kind === 'text') return said(part.text, size)
              const drawn = drawnImage(build.context.images?.[part.source])
              if (!drawn) return []
              // Word lays an image out inside the line. Without a break on
              // each side the words that surround it sit alongside the
              // picture, which is not how the page read.
              return [
                ...(index > 0 ? [new TextRun({ break: 1 })] : []),
                drawn,
                ...(index < parts.length - 1 ? [new TextRun({ break: 1 })] : []),
              ]
            }),
            ...said(`»${around[1]}`, size),
          ]
        }
        const anchor = rendered.inline
          ? say(rendered.inline, smaller ? QUOTE_HALF_POINTS : undefined)
          : []

        if (build.context.citations === 'comment' && rendered.note) {
          // A comment range needs something to span. With no inline text of its
          // own the source label stands in, so the comment has an anchor a
          // reader can click.
          const span = anchor.length > 0 ? anchor : say(rendered.note)
          return comment(build, rendered.note, span)
        }

        return rendered.note
          ? [...anchor, footnote(build, [new Paragraph({ children: say(rendered.note) })])]
          : anchor
      }

      case 'zoteroCitation':
        // §17.3: styled text carrying the CSL rendering. Live Zotero fields are
        // explicitly out of the MVP's scope.
        return [new TextRun({ text: zoteroTextOf(node, build.context) })]

      case 'noteLink':
        return [new TextRun({ text: renderNoteLink(node.attrs ?? {}), italics: true })]

      default:
        return inline(childrenOf(node), build, base, runStyle)
    }
  })
}

/**
 * A long quotation is set a point below the body — 11 pt against the body's
 * 12 — which is what tells the eye it is quoted before it reads a word of it.
 * A short one stays inside the sentence, at the size of the sentence.
 */
const QUOTE_HALF_POINTS = BODY_HALF_POINTS - 2

/**
 * A quote keeps the page's line breaks; DOCX writes each one as a break run,
 * the same run a hard break in the manuscript produces.
 */
function said(value: string, size?: number): TextRun[] {
  return value.split('\n').map(
    (line, index) =>
      new TextRun({
        text: line,
        italics: true,
        ...(size === undefined ? {} : { size }),
        ...(index > 0 ? { break: 1 } : {}),
      })
  )
}

/**
 * How wide a column of the page is, in pixels at 96 dpi: a Letter page less its
 * margins. A crop of a scan is far wider than that, and Word does not scale an
 * oversized image down — it runs it off the page.
 */
const COLUMN_WIDTH_PX = 540

/** What Word calls the formats we may have stored a crop in. */
const DOCX_IMAGE_TYPES: Record<string, 'png' | 'jpg' | 'gif' | 'bmp'> = {
  'image/png': 'png',
  'image/jpeg': 'jpg',
  'image/gif': 'gif',
  'image/bmp': 'bmp',
}

/**
 * The size a quoted image is drawn at: its own, or the column's, keeping its
 * proportions. Null when the file never declared a size — a shape guessed for
 * a scan would be worse than the words alone.
 */
export function quotedImageSize(image: ExportImage): { width: number; height: number } | null {
  if (image.width <= 0 || image.height <= 0) return null
  if (image.width <= COLUMN_WIDTH_PX) return { width: image.width, height: image.height }
  return {
    width: COLUMN_WIDTH_PX,
    height: Math.round(image.height * (COLUMN_WIDTH_PX / image.width)),
  }
}

/**
 * The size a manuscript image (writingImage) is drawn at (I7): the author's
 * own chosen width (`node.attrs.width`, spec — "the author's chosen width in
 * CSS pixels"), scaled down to the column if it would overrun the page, and
 * never scaled *up* past the column either. Before the author has ever
 * resized the image, `attrs.width` is null (C1) and this falls back to
 * `quotedImageSize`'s own intrinsic-or-column rule — the same policy a quote
 * crop already gets, since there is no chosen width to honour yet.
 */
export function writingImageSize(
  image: ExportImage,
  attrsWidth: number | null
): { width: number; height: number } | null {
  if (image.width <= 0 || image.height <= 0) return null
  if (typeof attrsWidth !== 'number' || attrsWidth <= 0) return quotedImageSize(image)
  const aspect = image.height / image.width
  const width = Math.min(attrsWidth, COLUMN_WIDTH_PX)
  return { width: Math.round(width), height: Math.round(width * aspect) }
}

/** The rule that runs down the side of a quotation set off as a block. */
const QUOTE_BORDER = {
  left: { style: BorderStyle.SINGLE, size: 6, space: 8, color: '999999' },
}

/** The run that draws a quoted image, or null when it cannot be drawn.
 *  `attrsWidth` is the manuscript image's own chosen width (I7); omitted for
 *  a quote crop, which has no such attribute and keeps `quotedImageSize`'s
 *  intrinsic-or-column sizing exactly as before. */
function drawnImage(image: ExportImage | undefined, attrsWidth?: number | null): ImageRun | null {
  if (!image) return null
  const type = DOCX_IMAGE_TYPES[image.mediaType]
  const size = attrsWidth === undefined ? quotedImageSize(image) : writingImageSize(image, attrsWidth)
  if (!type || !size) return null
  return new ImageRun({ data: image.bytes, type, transformation: size })
}

/**
 * A quotation that took in an image, as several paragraphs.
 *
 * Word has no picture *inside* a paragraph that a writer can move on its own:
 * alignment, spacing and indentation are the paragraph's, so with the image in
 * the quotation's paragraph, centring the image centres the words with it. The
 * quotation therefore becomes words, image, words — paragraphs that share the
 * quotation's border and indent, which is what keeps Word drawing one rule
 * beside the three of them instead of three.
 *
 * Null when this paragraph is not that: the caller then writes the single
 * paragraph it always wrote.
 */
function quotedBlock(paragraph: Node, build: Build): Paragraph[] | null {
  const citation = childrenOf(paragraph)[0]
  if (childrenOf(paragraph).length !== 1 || citation?.type !== 'documentCitation') return null

  const rendered = renderCorpusCitation(citation.attrs ?? {}, build.context.citations)
  const around = aroundFragment(rendered.inline, rendered.fragment)
  if (!around) return null

  const quotedText = typeof citation.attrs?.quotedText === 'string' ? citation.attrs.quotedText : ''

  // What the quotation is made of. With no images that is its text — which
  // still becomes several paragraphs when it holds a blank line.
  const pieces: ({ kind: 'text'; text: string } | ImageRun)[] = []
  for (const part of quotedPartsOf(citation.attrs ?? {}) ?? [{ kind: 'text', text: quotedText }]) {
    if (part.kind === 'text') {
      pieces.push(part)
      continue
    }
    const image = drawnImage(build.context.images?.[part.source])
    // An image that could not be read: back to the single paragraph, where the
    // quoted text is written whole and says what the picture cannot.
    if (!image) return null
    pieces.push(image)
  }

  const size = isLongQuote(quotedText) ? QUOTE_HALF_POINTS : undefined
  const shared = { border: QUOTE_BORDER, ...paragraphFormat(paragraph, { quoted: true }) }

  const paragraphs: Paragraph[] = []
  let runs: ParagraphChild[] = said(`${around[0]}«`, size)
  for (const part of pieces) {
    if ('kind' in part) {
      // A blank line was a paragraph break on the page, and a break run gives
      // no space between paragraphs — which is what it is for.
      const blocks = part.text.split(/\n{2,}/)
      blocks.forEach((text, index) => {
        if (index > 0) {
          paragraphs.push(new Paragraph({ children: runs, ...shared }))
          runs = []
        }
        runs.push(...said(text, size))
      })
      continue
    }
    paragraphs.push(new Paragraph({ children: runs, ...shared }))
    runs = []
    // Centred, where a picture in an academic quotation belongs — and now a
    // paragraph, so the writer can align it differently without moving the
    // words with it.
    paragraphs.push(new Paragraph({ children: [part], ...shared, alignment: AlignmentType.CENTER }))
  }

  runs.push(...said(`»${around[1]}`, size))
  if (rendered.note) {
    runs.push(footnote(build, [new Paragraph({ children: said(rendered.note) })]))
  }
  paragraphs.push(new Paragraph({ children: runs, ...shared }))

  return paragraphs
}

function block(node: Node, build: Build, depth = 0, quoted = false): (Paragraph | Table)[] {
  const kids = childrenOf(node)

  switch (node.type) {
    case 'paragraph': {
      // A paragraph that holds nothing but a long quote is that quote's block,
      // as the manuscript shows it (export-markdown.ts, isBlockQuoteParagraph)
      // — the same indented, bordered paragraph a blockquote gets.
      const set = quoted || isBlockQuoteParagraph(node, build.context)
      const split = set ? quotedBlock(node, build) : null
      if (split) return split
      return [
        new Paragraph({
          children: inline(kids, build),
          ...(set ? { border: QUOTE_BORDER } : {}),
          ...paragraphFormat(node, { quoted: set }),
        }),
      ]
    }

    case 'heading': {
      const level = Math.min(
        Math.max(typeof node.attrs?.level === 'number' ? node.attrs.level : 1, 1),
        6
      )
      return [
        new Paragraph({
          heading: HEADINGS[level - 1],
          children: inline(kids, build, baseOfHeading(level)),
          ...paragraphFormat(node),
        }),
      ]
    }

    case 'bulletList':
    case 'orderedList': {
      const ordered = node.type === 'orderedList'
      return kids.flatMap((item) =>
        childrenOf(item).flatMap((child, index) => {
          const paragraphs = block(child, build, depth + 1, quoted)
          // Only the item's first paragraph carries the bullet: a second one
          // inside the same item would otherwise start a new item, and a
          // nested list carries its own bullets already.
          if (index === 0 && child.type === 'paragraph') {
            return [
              new Paragraph({
                children: inline(childrenOf(child), build),
                // Alignment and spacing; a list item carries no indent level,
                // since a list is indented by nesting it.
                ...paragraphFormat(child, { listed: true }),
                ...(ordered
                  ? { numbering: { reference: NUMBERING_REFERENCE, level: Math.min(depth, 2) } }
                  : { bullet: { level: Math.min(depth, 2) } }),
              }),
              ...paragraphs.slice(1),
            ]
          }
          return paragraphs
        })
      )
    }

    case 'blockquote':
      return kids.flatMap((child) => block(child, build, depth, true))

    case 'codeBlock':
      return [
        new Paragraph({
          children: [new TextRun({ text: textOf(node), font: 'Consolas' })],
          shading: { type: ShadingType.CLEAR, fill: 'F2F2F2' },
        }),
      ]

    case 'horizontalRule':
      return [
        new Paragraph({
          border: { bottom: { style: BorderStyle.SINGLE, size: 6, space: 1, color: '999999' } },
        }),
      ]

    case 'table':
      return [
        new Table({
          width: { size: 100, type: WidthType.PERCENTAGE },
          rows: kids.map(
            (row) =>
              new TableRow({
                children: childrenOf(row).map(
                  (cell) =>
                    new TableCell({
                      // A header cell is shaded, which is how S4 verified the
                      // header row renders as one in Word.
                      ...(cell.type === 'tableHeader'
                        ? { shading: { type: ShadingType.CLEAR, fill: 'EEEEEE' } }
                        : {}),
                      children: childrenOf(cell).flatMap(
                        (child) => block(child, build, depth, quoted) as Paragraph[]
                      ),
                    })
                ),
              })
          ),
        }),
      ]

    case 'footnotes':
      // Written into `footnotes.xml` as each reference is met, never into the
      // body: that block sits at the end of the manuscript but its notes belong
      // wherever their markers are.
      return []

    case 'writingImage': {
      const src = typeof node.attrs?.src === 'string' ? node.attrs.src : ''
      const attrsWidth = typeof node.attrs?.width === 'number' ? node.attrs.width : null
      const drawn = drawnImage(build.context.images?.[src], attrsWidth)
      // I7: italic and a step below body size, the same step the exporter
      // already uses for a long quotation (QUOTE_HALF_POINTS) — matching the
      // editor's figcaption CSS (`font-style: italic`, `font-size: --xs`).
      const captionRuns = inline(kids, build, QUOTE_HALF_POINTS, {
        forceItalics: true,
        sizeFallback: QUOTE_HALF_POINTS,
      })
      // I7: the author's own alignment (spec's Node Shape `align`), which the
      // caption follows too — the editor's `[data-align] figcaption` rules
      // keep the caption aligned with its image, never a hardcoded center.
      const align = WRITING_IMAGE_ALIGNMENT[node.attrs?.align as string] ?? AlignmentType.CENTER
      const paragraphs: Paragraph[] = []
      if (drawn) paragraphs.push(new Paragraph({ children: [drawn], alignment: align }))
      if (captionRuns.length > 0) {
        paragraphs.push(new Paragraph({ children: captionRuns, alignment: align }))
      }
      return paragraphs
    }

    default:
      return kids.flatMap((child) => block(child, build, depth, quoted))
  }
}

/** The document model, before it is packed. Separated so a test can read it. */
export function buildDocx(doc: Node, context: ExportContext): Document {
  const build: Build = {
    context,
    bodies: new Map(),
    footnotes: {},
    comments: [],
    next: { footnote: 1, comment: 0 },
  }

  // The bodies first, so a reference met in the body already has its text.
  // Rendered against a build of their own: a note inside a note is not a thing,
  // and letting one register itself would recurse.
  const inner: Build = { ...build, next: { footnote: 1, comment: 0 }, comments: [] }
  build.bodies = footnoteBodies(doc, (nodes) => [new Paragraph({ children: inline(nodes, inner) })])

  const children = childrenOf(doc).flatMap((node) => block(node, build))

  const body: (Paragraph | Table)[] = []
  if (needsTitleHeading(doc, context.title)) {
    body.push(
      new Paragraph({ heading: HeadingLevel.TITLE, children: [new TextRun(context.title)] })
    )
  }
  body.push(...children)

  if (context.bibliography.length > 0) {
    body.push(
      new Paragraph({
        heading: HeadingLevel.HEADING_1,
        children: [new TextRun(context.bibliographyHeading)],
      })
    )
    for (const entry of context.bibliography) {
      body.push(
        new Paragraph({
          children: [new TextRun(entry)],
          // §11.6's hanging indent: the first line flush, the rest indented, so
          // the author is what the eye finds running down the page.
          indent: { left: 567, hanging: 567 },
          alignment: AlignmentType.LEFT,
        })
      )
    }
  }

  return new Document({
    styles: {
      default: {
        document: {
          run: { size: BODY_HALF_POINTS },
          paragraph: { spacing: PARAGRAPH_SPACING },
        },
        ...HEADING_STYLES,
        footnoteText: FOOTNOTE_TEXT,
      },
    },
    footnotes: build.footnotes,
    comments: { children: build.comments },
    numbering: {
      config: [
        {
          reference: NUMBERING_REFERENCE,
          levels: [0, 1, 2].map((level) => ({
            level,
            format: LevelFormat.DECIMAL,
            text: `%${level + 1}.`,
            alignment: AlignmentType.START,
            style: { paragraph: { indent: { left: 720 * (level + 1), hanging: 360 } } },
          })),
        },
      ],
    },
    sections: [{ children: body }],
  })
}

/**
 * The packed file, ready to be written to disk.
 *
 * `toArrayBuffer`, never `toBuffer`: the app runs in a WebView, which has no
 * Node `Buffer`, and there JSZip refuses the `nodebuffer` output outright.
 */
export async function toDocx(doc: Node, context: ExportContext): Promise<Uint8Array> {
  return new Uint8Array(await Packer.toArrayBuffer(buildDocx(doc, context)))
}
