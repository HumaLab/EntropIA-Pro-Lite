# Native OCR PDF Text Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace rasterized OCR PDF exports with semantically laid-out PDFs whose text is selectable, searchable, copyable, and extractable.

**Architecture:** Add a deep `ocr-pdf.ts` module that converts the existing sanitized export HTML into a typed pdfmake document definition, then renders it with bundled offline fonts. Keep `ocr-export.ts` as the orchestration module and preserve its current interfaces plus the Markdown and DOCX paths.

**Tech Stack:** TypeScript 5.9, Vitest 3, happy-dom, pdfmake 0.3.x, `@types/pdfmake`, PDF.js 4, Vite 6.

## Global Constraints

- Every visible textual value in prepared export HTML must become native PDF text.
- Text must be selectable, searchable, copyable, and extractable through PDF.js.
- OCR-region crops remain embedded raster images without duplicate visible alt text.
- Preserve `OcrExportFormat`, `PreparedOcrExport`, `OcrExportGenerators`, and `exportOcrText` interfaces.
- Preserve source normalization, sanitization, crop resolution, fallback wording, save dialog, filenames, and filesystem behavior.
- Preserve existing Markdown and DOCX generation paths.
- PDF generation must remain browser-local, offline, A4 portrait, and use 12 mm-equivalent margins.
- Bundle pdfmake's Roboto VFS; do not fetch fonts or images from the network.
- Do not add a raster fallback or promise pixel-identical CSS or tagged-PDF conformance.

---

### Task 1: Convert sanitized OCR HTML into native PDF content

**Files:**
- Create: `apps/desktop/src/lib/ocr-pdf.ts`
- Create: `apps/desktop/src/lib/ocr-pdf.test.ts`
- Modify: `apps/desktop/package.json`
- Modify: `pnpm-lock.yaml`

**Interfaces:**
- Consumes: full prepared export HTML containing `<div class="ocr-export-document">...</div>`.
- Produces: `buildOcrPdfDefinition(html: string): TDocumentDefinitions` with native headings, inline formatting, links, lists, blockquotes, code, images, and safe wrapper unwrapping. Task 2 adds semantic tables; Task 3 consumes the same function for byte generation.

- [ ] **Step 1: Write failing structural tests**

Create `apps/desktop/src/lib/ocr-pdf.test.ts`:

```typescript
import { describe, expect, it } from 'vitest'
import type { Content, ContentImage, ContentUnorderedList } from 'pdfmake/interfaces'

import { buildOcrPdfDefinition } from './ocr-pdf'

const ONE_PIXEL_PNG =
  'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M/wHwAF/gL+X1S8AAAAAElFTkSuQmCC'

function documentHtml(content: string): string {
  return `<!doctype html><html><body><div class="ocr-export-document">${content}</div></body></html>`
}

function contentArray(content: Content): Content[] {
  return Array.isArray(content) ? content : [content]
}

describe('buildOcrPdfDefinition', () => {
  it('maps headings and inline formatting to native text runs', () => {
    const definition = buildOcrPdfDefinition(
      documentHtml(
        '<h1>Título ñandú</h1>' +
          '<p>Texto <strong>negrita</strong> <em>cursiva</em> ' +
          '<u>subrayado</u><br><a href="https://example.test/">enlace</a>.</p>'
      )
    )
    const content = contentArray(definition.content)

    expect(content[0]).toMatchObject({
      style: 'h1',
      headlineLevel: 1,
      text: [{ text: 'Título ñandú' }],
    })
    expect(content[1]).toMatchObject({
      style: 'paragraph',
      text: [
        { text: 'Texto ' },
        { text: 'negrita', bold: true },
        { text: ' ' },
        { text: 'cursiva', italics: true },
        { text: ' ' },
        { text: 'subrayado', decoration: 'underline' },
        { text: '\n' },
        { text: 'enlace', link: 'https://example.test/' },
        { text: '.' },
      ],
    })
    expect(definition.pageSize).toBe('A4')
    expect(definition.pageOrientation).toBe('portrait')
    expect(definition.pageMargins).toEqual([34.02, 34.02, 34.02, 34.02])
    expect(definition.defaultStyle).toMatchObject({ font: 'Roboto', fontSize: 11 })
  })

  it('maps nested blocks, lists, code, images, and unknown wrappers without losing text', () => {
    const definition = buildOcrPdfDefinition(
      documentHtml(
        '<article><blockquote><p>Cita histórica</p></blockquote>' +
          '<ul><li>Primero<ol><li>Interior</li></ol></li><li>Segundo</li></ul>' +
          '<pre>const x = 1;\n  x++</pre>' +
          `<p>Antes<img src="${ONE_PIXEL_PNG}" alt="crop">Después</p>` +
          '</article>'
      )
    )
    const content = contentArray(definition.content)
    const list = content.find(
      (item): item is ContentUnorderedList =>
        typeof item === 'object' && item !== null && !Array.isArray(item) && 'ul' in item
    )
    const image = content.find(
      (item): item is ContentImage =>
        typeof item === 'object' && item !== null && !Array.isArray(item) && 'image' in item
    )

    expect(content).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          table: expect.objectContaining({
            body: [[expect.objectContaining({ stack: [expect.any(Object)] })]],
          }),
        }),
        expect.objectContaining({
          text: 'const x = 1;\n  x++',
          style: 'codeBlock',
          preserveLeadingSpaces: true,
          preserveTrailingSpaces: true,
        }),
        expect.objectContaining({ text: [{ text: 'Antes' }], style: 'paragraph' }),
        expect.objectContaining({ text: [{ text: 'Después' }], style: 'paragraph' }),
      ])
    )
    expect(list).toMatchObject({
      style: 'list',
      ul: [
        {
          stack: [
            { text: [{ text: 'Primero' }], style: 'listItem' },
            { ol: [{ text: [{ text: 'Interior' }], style: 'listItem' }], style: 'list' },
          ],
        },
        { text: [{ text: 'Segundo' }], style: 'listItem' },
      ],
    })
    expect(image).toMatchObject({ image: ONE_PIXEL_PNG, fit: [527.24, 700] })
  })
})
```

- [ ] **Step 2: Run the new test and verify RED**

Run from the repository root:

```bash
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-pdf.test.ts
```

Expected: FAIL because `./ocr-pdf` does not exist.

- [ ] **Step 3: Add the typed pdfmake dependencies without removing the current renderer yet**

Run:

```bash
pnpm --filter @entropia-pro/desktop add pdfmake@^0.3.11
pnpm --filter @entropia-pro/desktop add -D @types/pdfmake@^0.3.3
```

Expected: `apps/desktop/package.json` and `pnpm-lock.yaml` add pdfmake and its types while `html2pdf.js` remains until Task 3.

- [ ] **Step 4: Implement the semantic document-definition builder**

Create `apps/desktop/src/lib/ocr-pdf.ts`:

```typescript
import type {
  Alignment,
  Content,
  ContentImage,
  ContentOrderedList,
  ContentStack,
  ContentText,
  ContentUnorderedList,
  StyleDictionary,
  TDocumentDefinitions,
} from 'pdfmake/interfaces'

const PAGE_MARGIN_PT = 34.02
const WRITABLE_PAGE_WIDTH_PT = 527.24
const MAX_IMAGE_HEIGHT_PT = 700
const BLOCK_TAGS = new Set([
  'blockquote',
  'div',
  'h1',
  'h2',
  'h3',
  'h4',
  'h5',
  'h6',
  'img',
  'ol',
  'p',
  'pre',
  'section',
  'table',
  'ul',
])

type InlineStyle = {
  bold?: boolean
  italics?: boolean
  decoration?: 'underline'
  link?: string
  background?: string
}

const DOCUMENT_STYLES: StyleDictionary = {
  paragraph: { margin: [0, 0, 0, 9] },
  listItem: { margin: [0, 0, 0, 3] },
  list: { margin: [0, 0, 0, 9] },
  h1: { fontSize: 22, bold: true, color: '#111827', margin: [0, 0, 0, 10] },
  h2: { fontSize: 19, bold: true, color: '#111827', margin: [0, 0, 0, 9] },
  h3: { fontSize: 16, bold: true, color: '#111827', margin: [0, 0, 0, 8] },
  h4: { fontSize: 14, bold: true, color: '#111827', margin: [0, 0, 0, 7] },
  h5: { fontSize: 12, bold: true, color: '#111827', margin: [0, 0, 0, 6] },
  h6: { fontSize: 11, bold: true, color: '#111827', margin: [0, 0, 0, 5] },
  codeBlock: {
    background: '#f3f4f6',
    color: '#111827',
    fontSize: 9.5,
    margin: [0, 0, 0, 9],
  },
}

function normalizedInlineText(value: string): string {
  return value.replace(/\s+/g, ' ')
}

function inlineStyleFor(element: Element, inherited: InlineStyle): InlineStyle {
  const tag = element.tagName.toLowerCase()
  if (tag === 'strong' || tag === 'b') return { ...inherited, bold: true }
  if (tag === 'em' || tag === 'i') return { ...inherited, italics: true }
  if (tag === 'u') return { ...inherited, decoration: 'underline' }
  if (tag === 'code') return { ...inherited, background: '#f3f4f6' }
  if (tag === 'a') {
    const link = element.getAttribute('href')?.trim()
    return link ? { ...inherited, link } : inherited
  }
  return inherited
}

function trimInlineRuns(runs: ContentText[]): ContentText[] {
  const trimmed = runs.map((run) => ({ ...run, text: String(run.text) }))
  while (trimmed.length > 0 && String(trimmed[0]!.text).trim() === '') trimmed.shift()
  while (trimmed.length > 0 && String(trimmed.at(-1)!.text).trim() === '') trimmed.pop()
  if (trimmed.length === 0) return []

  trimmed[0] = { ...trimmed[0]!, text: String(trimmed[0]!.text).replace(/^\s+/, '') }
  const last = trimmed.length - 1
  trimmed[last] = { ...trimmed[last]!, text: String(trimmed[last]!.text).replace(/\s+$/, '') }
  return trimmed.filter((run) => String(run.text).length > 0)
}

function inlineRuns(nodes: Iterable<Node>, inherited: InlineStyle = {}): ContentText[] {
  const runs: ContentText[] = []

  for (const node of nodes) {
    if (node.nodeType === Node.TEXT_NODE) {
      const text = normalizedInlineText(node.textContent ?? '')
      if (text) runs.push({ text, ...inherited })
      continue
    }
    if (node.nodeType !== Node.ELEMENT_NODE) continue

    const element = node as Element
    if (element.tagName.toLowerCase() === 'br') {
      runs.push({ text: '\n', ...inherited })
      continue
    }
    runs.push(...inlineRuns(element.childNodes, inlineStyleFor(element, inherited)))
  }

  return runs
}

function paragraphFromNodes(nodes: Node[], style: string): ContentText | null {
  const text = trimInlineRuns(inlineRuns(nodes))
  return text.length > 0 ? { text, style } : null
}

function paragraphFromText(value: string, style = 'paragraph'): ContentText | null {
  const text = value.trim()
  return text ? { text, style } : null
}

function alignmentFor(element: Element): Alignment | undefined {
  const align = element.getAttribute('align')?.toLowerCase()
  return align === 'left' || align === 'center' || align === 'right' ? align : undefined
}

function isBlockNode(node: Node): node is Element {
  return node.nodeType === Node.ELEMENT_NODE && BLOCK_TAGS.has((node as Element).tagName.toLowerCase())
}

function convertChildren(element: Element, paragraphStyle = 'paragraph'): Content[] {
  const content: Content[] = []
  let pendingInline: Node[] = []

  const flushInline = () => {
    const paragraph = paragraphFromNodes(pendingInline, paragraphStyle)
    if (paragraph) content.push(paragraph)
    pendingInline = []
  }

  for (const child of Array.from(element.childNodes)) {
    if (isBlockNode(child)) {
      flushInline()
      content.push(...convertElement(child))
    } else {
      pendingInline.push(child)
    }
  }
  flushInline()

  return content
}

function convertListItem(element: Element): Content {
  const content = convertChildren(element, 'listItem')
  if (content.length === 0) return { text: '' }
  return content.length === 1 ? content[0]! : ({ stack: content } satisfies ContentStack)
}

function convertList(element: Element): ContentOrderedList | ContentUnorderedList {
  const items = Array.from(element.children)
    .filter((child) => child.tagName.toLowerCase() === 'li')
    .map(convertListItem)

  return element.tagName.toLowerCase() === 'ol'
    ? { ol: items as ContentOrderedList['ol'], style: 'list' }
    : { ul: items as ContentUnorderedList['ul'], style: 'list' }
}

function convertImage(element: Element): ContentImage[] {
  const source = element.getAttribute('src')?.trim()
  if (!source) return []
  return [
    {
      image: source,
      fit: [WRITABLE_PAGE_WIDTH_PT, MAX_IMAGE_HEIGHT_PT],
      margin: [0, 0, 0, 9],
    },
  ]
}

function convertBlockquote(element: Element): Content[] {
  const stack = convertChildren(element)
  if (stack.length === 0) return []
  return [
    {
      table: {
        widths: ['*'],
        body: [
          [
            {
              stack,
              border: [true, false, false, false],
              borderColor: ['#9ca3af', '#9ca3af', '#9ca3af', '#9ca3af'],
              color: '#4b5563',
              margin: [10, 0, 0, 0],
            },
          ],
        ],
      },
      margin: [0, 0, 0, 9],
    },
  ]
}

function convertElement(element: Element): Content[] {
  const tag = element.tagName.toLowerCase()

  if (/^h[1-6]$/.test(tag)) {
    const heading = paragraphFromNodes(Array.from(element.childNodes), tag)
    if (!heading) return []
    return [{ ...heading, headlineLevel: Number(tag[1]) }]
  }
  if (tag === 'p') return convertChildren(element)
  if (tag === 'div' || tag === 'section') {
    const stack = convertChildren(element)
    const alignment = alignmentFor(element)
    return alignment && stack.length > 0 ? [{ stack, alignment }] : stack
  }
  if (tag === 'ul' || tag === 'ol') return [convertList(element)]
  if (tag === 'blockquote') return convertBlockquote(element)
  if (tag === 'pre') {
    const text = element.textContent ?? ''
    return text
      ? [
          {
            text,
            style: 'codeBlock',
            preserveLeadingSpaces: true,
            preserveTrailingSpaces: true,
          },
        ]
      : []
  }
  if (tag === 'img') return convertImage(element)
  if (tag === 'table') {
    const fallback = paragraphFromText(element.textContent ?? '')
    return fallback ? [fallback] : []
  }

  return convertChildren(element)
}

export function buildOcrPdfDefinition(html: string): TDocumentDefinitions {
  const parsed = new DOMParser().parseFromString(html, 'text/html')
  const root = parsed.querySelector('.ocr-export-document') ?? parsed.body

  return {
    pageSize: 'A4',
    pageOrientation: 'portrait',
    pageMargins: [PAGE_MARGIN_PT, PAGE_MARGIN_PT, PAGE_MARGIN_PT, PAGE_MARGIN_PT],
    compress: true,
    defaultStyle: {
      font: 'Roboto',
      fontSize: 11,
      lineHeight: 1.55,
      color: '#1f2937',
    },
    styles: DOCUMENT_STYLES,
    content: convertChildren(root),
    pageBreakBefore(currentNode, followingNodesOnPage) {
      return Boolean(currentNode.headlineLevel && followingNodesOnPage.length === 0)
    },
  }
}
```

- [ ] **Step 5: Run focused tests and verify GREEN**

Run:

```bash
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-pdf.test.ts
```

Expected: `src/lib/ocr-pdf.test.ts (2 tests)` passes.

- [ ] **Step 6: Run the Lite typecheck**

Run:

```bash
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck
```

Expected: `0 errors and 0 warnings`.

- [ ] **Step 7: Commit the semantic content module**

```bash
git add apps/desktop/src/lib/ocr-pdf.ts apps/desktop/src/lib/ocr-pdf.test.ts apps/desktop/package.json pnpm-lock.yaml
git commit -m "feat(desktop): map OCR HTML to native PDF content"
```

Rollback boundary: this commit adds only the unused semantic document-definition module, its structural tests, and pdfmake dependencies; the active html2pdf export path remains unchanged.

---

### Task 2: Preserve semantic tables and cell spans

**Files:**
- Modify: `apps/desktop/src/lib/ocr-pdf.ts`
- Modify: `apps/desktop/src/lib/ocr-pdf.test.ts`

**Interfaces:**
- Consumes: `buildOcrPdfDefinition(html: string): TDocumentDefinitions` from Task 1.
- Produces: the same interface, now mapping captions, header rows, bordered cells, `rowSpan`, `colSpan`, and required placeholders into rectangular pdfmake tables.

- [ ] **Step 1: Add failing table tests**

Append inside `describe('buildOcrPdfDefinition', ...)` in `apps/desktop/src/lib/ocr-pdf.test.ts`:

```typescript
  it('maps captions, headers, row spans, and column spans into a rectangular table', () => {
    const definition = buildOcrPdfDefinition(
      documentHtml(
        '<table><caption>Resumen anual</caption><thead>' +
          '<tr><th rowspan="2">Nombre</th><th colspan="2">Valores</th></tr>' +
          '<tr><th>Uno</th><th>Dos</th></tr></thead><tbody>' +
          '<tr><td>Registro</td><td>10</td><td>20</td></tr>' +
          '</tbody></table>'
      )
    )
    const [tableStack] = contentArray(definition.content)

    expect(tableStack).toMatchObject({
      stack: [
        { text: 'Resumen anual', style: 'tableCaption' },
        {
          table: {
            headerRows: 2,
            widths: ['*', '*', '*'],
            body: [
              [
                expect.objectContaining({ rowSpan: 2, style: 'tableHeader' }),
                expect.objectContaining({ colSpan: 2, style: 'tableHeader' }),
                {},
              ],
              [{}, expect.objectContaining({ style: 'tableHeader' }), expect.any(Object)],
              [
                expect.objectContaining({ style: 'tableCell' }),
                expect.any(Object),
                expect.any(Object),
              ],
            ],
          },
        },
      ],
    })
  })

  it('degrades a malformed table to readable native text', () => {
    const definition = buildOcrPdfDefinition(
      documentHtml('<table><caption>Tabla incompleta</caption></table>')
    )

    expect(contentArray(definition.content)).toEqual([
      { text: 'Tabla incompleta', style: 'paragraph' },
    ])
  })
```

- [ ] **Step 2: Run the table tests and verify RED**

Run:

```bash
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-pdf.test.ts
```

Expected: the new semantic table test fails because Task 1 emits only flattened table text.

- [ ] **Step 3: Add table types and styles**

Extend the type import in `apps/desktop/src/lib/ocr-pdf.ts` with:

```typescript
  ContentTable,
  TableCell,
```

Add these entries to `DOCUMENT_STYLES`:

```typescript
  tableCaption: { bold: true, color: '#4b5563', margin: [0, 0, 0, 5] },
  tableHeader: { bold: true, fillColor: '#f3f4f6', margin: [4, 3, 4, 3] },
  tableCell: { margin: [4, 3, 4, 3] },
```

- [ ] **Step 4: Implement span-aware table conversion**

Insert before `convertElement`:

```typescript
function boundedSpan(element: Element, name: 'rowspan' | 'colspan', maximum: number): number {
  const parsed = Number(element.getAttribute(name) ?? '1')
  return Number.isInteger(parsed) && parsed > 0 ? Math.min(parsed, maximum) : 1
}

function directTableRows(table: Element): Element[] {
  const rows: Element[] = []
  for (const child of Array.from(table.children)) {
    const tag = child.tagName.toLowerCase()
    if (tag === 'tr') rows.push(child)
    if (tag === 'thead' || tag === 'tbody' || tag === 'tfoot') {
      rows.push(...Array.from(child.children).filter((row) => row.tagName.toLowerCase() === 'tr'))
    }
  }
  return rows
}

function headerRowCount(table: Element): number {
  const head = Array.from(table.children).find((child) => child.tagName.toLowerCase() === 'thead')
  return head
    ? Array.from(head.children).filter((child) => child.tagName.toLowerCase() === 'tr').length
    : 0
}

function tableCellContent(element: Element, rowSpan: number, colSpan: number): TableCell {
  const stack = convertChildren(element)
  const isHeader = element.tagName.toLowerCase() === 'th'
  return {
    stack: stack.length > 0 ? stack : [{ text: '' }],
    style: isHeader ? 'tableHeader' : 'tableCell',
    rowSpan,
    colSpan,
    verticalAlignment: 'top',
  }
}

function convertTable(element: Element): Content[] {
  const rows = directTableRows(element)
  if (rows.length === 0) {
    const fallback = paragraphFromText(element.textContent ?? '')
    return fallback ? [fallback] : []
  }

  const occupied = new Set<string>()
  const body: TableCell[][] = []
  let columnCount = 0

  rows.forEach((row, rowIndex) => {
    const output: TableCell[] = []
    let columnIndex = 0
    const cells = Array.from(row.children).filter((cell) => {
      const tag = cell.tagName.toLowerCase()
      return tag === 'th' || tag === 'td'
    })

    for (const cell of cells) {
      while (occupied.has(`${rowIndex}:${columnIndex}`)) {
        output[columnIndex] = {}
        columnIndex += 1
      }

      const rowSpan = boundedSpan(cell, 'rowspan', rows.length - rowIndex)
      const colSpan = boundedSpan(cell, 'colspan', 100)
      output[columnIndex] = tableCellContent(cell, rowSpan, colSpan)

      for (let columnOffset = 1; columnOffset < colSpan; columnOffset += 1) {
        output[columnIndex + columnOffset] = {}
      }
      for (let rowOffset = 1; rowOffset < rowSpan; rowOffset += 1) {
        for (let columnOffset = 0; columnOffset < colSpan; columnOffset += 1) {
          occupied.add(`${rowIndex + rowOffset}:${columnIndex + columnOffset}`)
        }
      }
      columnIndex += colSpan
    }

    const occupiedColumns = Array.from(occupied)
      .filter((key) => key.startsWith(`${rowIndex}:`))
      .map((key) => Number(key.split(':')[1]))
    const currentWidth = Math.max(output.length, ...occupiedColumns.map((index) => index + 1), 0)
    for (let index = 0; index < currentWidth; index += 1) {
      if (!output[index]) output[index] = {}
    }
    columnCount = Math.max(columnCount, currentWidth)
    body.push(output)
  })

  for (const row of body) {
    while (row.length < columnCount) row.push({})
  }

  const table: ContentTable = {
    table: {
      headerRows: Math.min(headerRowCount(element), body.length),
      widths: Array.from({ length: columnCount }, () => '*'),
      body,
    },
    margin: [0, 0, 0, 9],
  }
  const caption = Array.from(element.children).find(
    (child) => child.tagName.toLowerCase() === 'caption'
  )
  const captionText = caption ? paragraphFromText(caption.textContent ?? '', 'tableCaption') : null

  return captionText ? [{ stack: [captionText, table] }] : [table]
}
```

Replace the existing `tag === 'table'` branch in `convertElement` with:

```typescript
  if (tag === 'table') return convertTable(element)
```

- [ ] **Step 5: Run all converter tests and verify GREEN**

Run:

```bash
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-pdf.test.ts
```

Expected: `src/lib/ocr-pdf.test.ts (4 tests)` passes.

- [ ] **Step 6: Run the Lite typecheck**

Run:

```bash
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck
```

Expected: `0 errors and 0 warnings`.

- [ ] **Step 7: Commit semantic table support**

```bash
git add apps/desktop/src/lib/ocr-pdf.ts apps/desktop/src/lib/ocr-pdf.test.ts
git commit -m "feat(desktop): preserve OCR tables in native PDFs"
```

Rollback boundary: reverting this commit returns table content to the Task 1 readable-text fallback without affecting other native content conversion.

---

### Task 3: Replace html2pdf with real native PDF byte generation

**Files:**
- Modify: `apps/desktop/src/lib/ocr-pdf.ts`
- Modify: `apps/desktop/src/lib/ocr-pdf.test.ts`
- Modify: `apps/desktop/src/lib/ocr-export.ts:1-150,269-285`
- Modify: `apps/desktop/src/lib/ocr-export.test.ts:1-69,290-333`
- Modify: `apps/desktop/src/types/ocr-export-libraries.d.ts:1-14`
- Modify: `apps/desktop/package.json`
- Modify: `pnpm-lock.yaml`

**Interfaces:**
- Consumes: `buildOcrPdfDefinition(html: string): TDocumentDefinitions` from Tasks 1–2 and pdfmake's browser VFS.
- Produces: `generateNativeOcrPdfBytes(html: string): Promise<Uint8Array>`; `generateOcrExportBytes('pdf', ...)` delegates to it without changing any exported interface.

- [ ] **Step 1: Add the failing real-PDF text-layer contract test**

Extend the import in `apps/desktop/src/lib/ocr-pdf.test.ts`:

```typescript
import { buildOcrPdfDefinition, generateNativeOcrPdfBytes } from './ocr-pdf'
```

Append inside the existing `describe` block:

```typescript
  it('generates native extractable text while preserving an embedded OCR image', async () => {
    const bytes = await generateNativeOcrPdfBytes(
      documentHtml(
        '<h1>Radiografía nativa</h1>' +
          '<p>Texto español con <strong>énfasis</strong> y ' +
          '<a href="https://example.test/">enlace verificable</a>.</p>' +
          '<ul><li>Elemento seleccionable</li></ul>' +
          '<table><thead><tr><th>Columna</th></tr></thead>' +
          '<tbody><tr><td>Celda extraíble</td></tr></tbody></table>' +
          `<img src="${ONE_PIXEL_PNG}" alt="OCR region from page 1">`
      )
    )

    expect(Array.from(bytes.slice(0, 5))).toEqual([37, 80, 68, 70, 45])

    const pdfjs = await import('pdfjs-dist/legacy/build/pdf.mjs')
    pdfjs.GlobalWorkerOptions.workerSrc = new URL(
      'pdfjs-dist/legacy/build/pdf.worker.min.mjs',
      import.meta.url
    ).href
    const loadingTask = pdfjs.getDocument({ data: bytes.slice() })
    const pdf = await loadingTask.promise
    const page = await pdf.getPage(1)
    const textContent = await page.getTextContent()
    const extracted = textContent.items
      .map((item) => ('str' in item ? item.str : ''))
      .join(' ')
      .replace(/\s+/g, ' ')
      .replace(/\s+([.,;:!?])/g, '$1')
      .trim()

    expect(extracted).toContain('Radiografía nativa')
    expect(extracted).toContain('Texto español con énfasis y enlace verificable.')
    expect(extracted).toContain('Elemento seleccionable')
    expect(extracted).toContain('Columna')
    expect(extracted).toContain('Celda extraíble')
    expect(extracted).not.toContain('OCR region from page 1')

    await pdf.destroy()
  })
```

- [ ] **Step 2: Run the contract test and verify RED**

Run:

```bash
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-pdf.test.ts
```

Expected: FAIL because `generateNativeOcrPdfBytes` is not yet exported.

- [ ] **Step 3: Implement one-time offline pdfmake loading and byte generation**

Extend the type import in `apps/desktop/src/lib/ocr-pdf.ts` with:

```typescript
  TCreatedPdf,
  TVirtualFileSystem,
```

Add after the constants:

```typescript
interface PdfMakeBrowserApi {
  addVirtualFileSystem(vfs: TVirtualFileSystem): void
  createPdf(definition: TDocumentDefinitions): Pick<TCreatedPdf, 'getBlob'>
}

let pdfMakePromise: Promise<PdfMakeBrowserApi> | null = null

function moduleDefault<T>(module: unknown): T {
  if (typeof module === 'object' && module !== null && 'default' in module) {
    return (module as { default: T }).default
  }
  return module as T
}

async function loadPdfMake(): Promise<PdfMakeBrowserApi> {
  if (!pdfMakePromise) {
    pdfMakePromise = Promise.all([
      import('pdfmake/build/pdfmake'),
      import('pdfmake/build/vfs_fonts'),
    ])
      .then(([pdfMakeModule, fontsModule]) => {
        const pdfMake = moduleDefault<PdfMakeBrowserApi>(pdfMakeModule)
        const fonts = moduleDefault<TVirtualFileSystem>(fontsModule)
        pdfMake.addVirtualFileSystem(fonts)
        return pdfMake
      })
      .catch((error) => {
        pdfMakePromise = null
        throw error
      })
  }
  return pdfMakePromise
}
```

Append after `buildOcrPdfDefinition`:

```typescript
export async function generateNativeOcrPdfBytes(html: string): Promise<Uint8Array> {
  const pdfMake = await loadPdfMake()
  const blob = await pdfMake.createPdf(buildOcrPdfDefinition(html)).getBlob()
  return new Uint8Array(await blob.arrayBuffer())
}
```

- [ ] **Step 4: Run the real PDF contract test and verify GREEN**

Run:

```bash
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-pdf.test.ts
```

Expected: `src/lib/ocr-pdf.test.ts (5 tests)` passes and PDF.js extracts all visible text, including accents.

- [ ] **Step 5: Wire the native adapter and delete the canvas renderer**

Add to `apps/desktop/src/lib/ocr-export.ts` imports:

```typescript
import { generateNativeOcrPdfBytes } from './ocr-pdf'
```

Delete the complete `generatePdfBytes` function. Replace the PDF branch with:

```typescript
  if (format === 'pdf') {
    return (generators.pdf ?? generateNativeOcrPdfBytes)(html)
  }
```

In `apps/desktop/src/lib/ocr-export.test.ts`:

- Replace the hoisted mock with:

```typescript
const { htmlDocxAsBlobMock } = vi.hoisted(() => ({
  htmlDocxAsBlobMock: vi.fn(),
}))
```

- Delete `vi.mock('html2pdf.js', ...)`.
- Delete every `html2pdfWorker` setup/reset line from `beforeEach` and `afterEach`.
- Delete the three obsolete tests beginning with:
  - `keeps the html2pdf render target in normal flow while staging it off-screen`;
  - `removes the temporary PDF DOM after a successful render`;
  - `removes the temporary PDF DOM after a failed render`.

The existing shared-HTML, Markdown, DOCX, save, and cancellation tests remain unchanged.

- [ ] **Step 6: Remove obsolete library declarations and dependency**

Delete lines 1–14 containing the `html2pdf.js` declaration from `apps/desktop/src/types/ocr-export-libraries.d.ts`. Retain every DOCX declaration.

Run:

```bash
pnpm --filter @entropia-pro/desktop remove html2pdf.js
```

Expected: `html2pdf.js` disappears from `apps/desktop/package.json` and its unused dependency graph disappears from `pnpm-lock.yaml`; `pdfmake` and `@types/pdfmake` remain.

- [ ] **Step 7: Run focused PDF and export tests**

Run:

```bash
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-pdf.test.ts src/lib/ocr-export.test.ts
```

Expected: both files pass; `ocr-pdf.test.ts` has 5 tests and `ocr-export.test.ts` retains 10 non-html2pdf tests.

- [ ] **Step 8: Run the Lite frontend typecheck**

Run:

```bash
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck
```

Expected: `0 errors and 0 warnings`.

- [ ] **Step 9: Verify the actual surface in Chromium**

Start Vite from the worktree:

```bash
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop exec vite --port 1421
```

In Chromium at `http://localhost:1421/`, import the real module and generate a PDF:

```typescript
const { generateOcrExportBytes } = await import('/src/lib/ocr-export.ts')
const bytes = await generateOcrExportBytes('pdf', {
  markdown: '# Radiografía nativa',
  html:
    '<h1>Radiografía nativa</h1>' +
    '<p>Texto español seleccionable con ñandú.</p>' +
    '<table><thead><tr><th>Columna</th></tr></thead>' +
    '<tbody><tr><td>Celda extraíble</td></tr></tbody></table>',
})
const url = URL.createObjectURL(new Blob([bytes], { type: 'application/pdf' }))
location.href = url
```

Confirm visually that the heading, paragraph, and table render on an A4 page. Parse the same bytes with `pdfjs-dist` and confirm `getTextContent()` contains `Radiografía nativa`, `Texto español seleccionable con ñandú.`, `Columna`, and `Celda extraíble`. The output must contain more than zero text items and must not be a full-page raster image.

- [ ] **Step 10: Commit the native PDF cutover**

```bash
git add apps/desktop/src/lib/ocr-pdf.ts apps/desktop/src/lib/ocr-pdf.test.ts apps/desktop/src/lib/ocr-export.ts apps/desktop/src/lib/ocr-export.test.ts apps/desktop/src/types/ocr-export-libraries.d.ts apps/desktop/package.json pnpm-lock.yaml
git commit -m "fix(desktop): export OCR PDFs with native text"
```

Rollback boundary: reverting this commit restores html2pdf as the active adapter while leaving the independently tested semantic converter commits available; Markdown, DOCX, OCR preparation, and UI code are outside the rollback.
