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
  LevelFormat,
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
import { renderCorpusCitation, renderNoteLink } from './export-citations'
import type { ExportContext, Node } from './export-document'
import {
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

function styleOf(node: Node) {
  const marks = new Set((node.marks ?? []).map((mark) => mark.type))
  return {
    bold: marks.has('bold'),
    italics: marks.has('italic'),
    strike: marks.has('strike'),
    underline: marks.has('underline') ? {} : undefined,
    font: marks.has('code') ? 'Consolas' : undefined,
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

function inline(nodes: Node[], build: Build): ParagraphChild[] {
  return nodes.flatMap((node): ParagraphChild[] => {
    switch (node.type) {
      case 'text': {
        const value = node.text ?? ''
        const link = (node.marks ?? []).find((mark) => mark.type === 'link')
        const href = link ? safeHref(link.attrs?.href) : null
        const run = new TextRun({ text: value, ...styleOf(node) })
        // A refused target keeps its words, exactly as in HTML: dropping them
        // would delete prose the writer wrote.
        return href ? [new ExternalHyperlink({ children: [run], link: href })] : [run]
      }

      case 'hardBreak':
        return [new TextRun({ break: 1 })]

      case 'footnoteReference': {
        const id = typeof node.attrs?.['data-id'] === 'string' ? node.attrs['data-id'] : ''
        return [footnote(build, build.bodies.get(id) ?? [])]
      }

      case 'documentCitation': {
        const rendered = renderCorpusCitation(node.attrs ?? {}, build.context.citations)
        const anchor = rendered.inline
          ? [new TextRun({ text: rendered.inline, italics: true })]
          : []

        if (build.context.citations === 'comment' && rendered.note) {
          // A comment range needs something to span. With no inline text of its
          // own the source label stands in, so the comment has an anchor a
          // reader can click.
          const span =
            anchor.length > 0 ? anchor : [new TextRun({ text: rendered.note, italics: true })]
          return comment(build, rendered.note, span)
        }

        return rendered.note
          ? [...anchor, footnote(build, [new Paragraph(rendered.note)])]
          : anchor
      }

      case 'zoteroCitation':
        // §17.3: styled text carrying the CSL rendering. Live Zotero fields are
        // explicitly out of the MVP's scope.
        return [new TextRun({ text: zoteroTextOf(node, build.context) })]

      case 'noteLink':
        return [new TextRun({ text: renderNoteLink(node.attrs ?? {}), italics: true })]

      default:
        return inline(childrenOf(node), build)
    }
  })
}

function block(node: Node, build: Build, depth = 0, quoted = false): (Paragraph | Table)[] {
  const kids = childrenOf(node)

  switch (node.type) {
    case 'paragraph':
      return [
        new Paragraph({
          children: inline(kids, build),
          ...(quoted
            ? {
                indent: { left: 567 },
                border: { left: { style: BorderStyle.SINGLE, size: 6, space: 8, color: '999999' } },
              }
            : {}),
        }),
      ]

    case 'heading': {
      const level = typeof node.attrs?.level === 'number' ? node.attrs.level : 1
      return [
        new Paragraph({
          heading: HEADINGS[Math.min(Math.max(level, 1), 6) - 1],
          children: inline(kids, build),
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
    body.push(new Paragraph({ heading: HeadingLevel.TITLE, children: [new TextRun(context.title)] }))
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

/** The packed file, ready to be written to disk. */
export async function toDocx(doc: Node, context: ExportContext): Promise<Uint8Array> {
  return new Uint8Array(await Packer.toBuffer(buildDocx(doc, context)))
}
