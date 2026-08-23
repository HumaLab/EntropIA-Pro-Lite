# OCR Extracted Text Actions and Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add scoped Copy and Markdown/PDF/DOCX export actions to the left-panel `Texto extraído` header while preserving the OCR source and every other viewer surface.

**Architecture:** Keep the existing OCR parser/crop resolver as the single source for rendered content. Add a focused `ocr-export.ts` module that prepares one Markdown representation and one sanitized visual HTML representation, embeds valid page/bbox crops in memory, and feeds the same prepared HTML to PDF and DOCX adapters. Mount the actions only in `ItemAssetPanel.svelte`; use the existing Tauri save/write APIs and existing UI icons/components.

**Tech Stack:** Svelte 5 runes, TypeScript, Vitest, Testing Library Svelte, `markdown-it@^14.1.1`, existing `pdfjs-dist@^4`, `html2pdf.js`, `html-docx-js`, HTML canvas, Tauri dialog/filesystem plugins.

## Global Constraints

- Target only `apps/desktop/src/views/ItemAssetPanel.svelte` non-audio `left-panel-text` content.
- Do not modify `apps/desktop/src/lib/markdown.ts`, `DocumentViewer`, `ItemTextPanel.svelte`, OCR state/persistence, Rust OCR code, or database schemas.
- Preserve `ocrEditedText` exactly for Copy; generated Markdown/HTML/crops are derived in memory only.
- Interpret `page` as zero-based and `bbox` as `[left, top, right, bottom]`.
- Embed valid page/bbox crops in all derived exports; use the readable `Imagen OCR no disponible` marker for failed references.
- Do not fetch arbitrary remote image URLs; only generated source-asset crops may become export images.
- Use `VITE_LOCAL_ML=0` for Lite typecheck and avoid full Tauri release builds.
- Keep all technical artifacts and code comments in English; localize only user-facing strings through `apps/desktop/src/lib/i18n.ts`.

---

### Task 1: Add the shared OCR export preparation boundary

**Files:**
- Create: `apps/desktop/src/lib/ocr-export.ts`
- Create: `apps/desktop/src/lib/ocr-export.test.ts`
- Modify: `apps/desktop/src/lib/ocr-rich-text.ts`
- Modify: `apps/desktop/src/lib/ocr-rich-text.test.ts`

**Interfaces:**

```ts
import type {
  OcrRegionReference,
  OcrRegionResolver,
  OcrRenderContext,
  OcrSourceType,
} from './ocr-rich-text'

export type OcrExportFormat = 'markdown' | 'pdf' | 'docx'

export interface OcrExportInput extends OcrRenderContext {
  source: string
}

export interface PreparedOcrExport {
  markdown: string
  html: string
}

export function replaceOcrRegionReferences(
  source: string,
  replacer: (reference: OcrRegionReference) => string
): string

export async function prepareOcrExport(
  input: OcrExportInput,
  resolveRegion?: OcrRegionResolver
): Promise<PreparedOcrExport>
```

- [ ] **Step 1: Add the failing source-replacement and export-preparation tests**

Create `apps/desktop/src/lib/ocr-export.test.ts` with deterministic resolver injection. The first test must prove that valid references are replaced independently, including duplicate source literals, while all other Markdown/HTML remains unchanged:

```ts
import { describe, expect, it } from 'vitest'
import { prepareOcrExport } from './ocr-export'

const input = {
  source: '# Título\r\n\r\n<div align="center">HTML</div>\r\n\r\n![](page=0,bbox=[1,2,3,4])\r\n\r\n![](page=0,bbox=[1,2,3,4])',
  assetUrl: 'asset://source',
  sourceType: 'image' as const,
  referenceWidth: 100,
  referenceHeight: 100,
}

it('preserves source Markdown/HTML and embeds every valid OCR region', async () => {
  const result = await prepareOcrExport(input, async (reference) => {
    return reference.token === 'region-0'
      ? 'data:image/png;base64,AAAA'
      : 'data:image/png;base64,BBBB'
  })

  expect(result.markdown).toContain('# Título')
  expect(result.markdown).toContain('<div align="center">HTML</div>')
  expect(result.markdown).toContain('![OCR region from page 1](data:image/png;base64,AAAA)')
  expect(result.markdown).toContain('![OCR region from page 1](data:image/png;base64,BBBB)')
  expect(result.markdown).not.toContain('\\r')
  expect(result.html).toContain('<h1>Título</h1>')
  expect(result.html).toContain('<img src="data:image/png;base64,AAAA"')
  expect(result.html).toContain('<img src="data:image/png;base64,BBBB"')
})

it('uses a readable marker for a rejected region without dropping surrounding content', async () => {
  const result = await prepareOcrExport(
    { ...input, source: 'antes ![](page=4,bbox=[1,2,3,4]) después' },
    async () => {
      throw new Error('source unavailable')
    }
  )

  expect(result.markdown).toContain('*[Imagen OCR no disponible]*')
  expect(result.html).toContain('Imagen OCR no disponible')
  expect(result.html).toContain('antes')
  expect(result.html).toContain('después')
})
```

Add the corresponding helper test to `apps/desktop/src/lib/ocr-rich-text.test.ts`:

```ts
import { replaceOcrRegionReferences } from './ocr-rich-text'

it('assigns distinct tokens to repeated valid OCR references', () => {
  const tokens: string[] = []
  const source = '![](page=0,bbox=[1,2,3,4]) ![](page=0,bbox=[1,2,3,4])'

  const replaced = replaceOcrRegionReferences(source, (reference) => {
    tokens.push(reference.token)
    return `<${reference.token}>`
  })

  expect(tokens).toEqual(['region-0', 'region-1'])
  expect(replaced).toBe('<region-0> <region-1>')
})
```

Run the tests before implementation:

```powershell
pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-rich-text.test.ts src/lib/ocr-export.test.ts
```

Expected: FAIL because `replaceOcrRegionReferences`, `ocr-export.ts`, and `prepareOcrExport` are not implemented.

- [ ] **Step 2: Implement the shared region replacement helper**

In `apps/desktop/src/lib/ocr-rich-text.ts`, export a helper using the existing private OCR-region regex and parser so the exporter does not duplicate tokenization rules:

```ts
export function replaceOcrRegionReferences(
  source: string,
  replacer: (reference: OcrRegionReference) => string
): string {
  let nextToken = 0

  return source.replace(OCR_REGION_MARKDOWN, (whole, region: string) => {
    const parsed = parseOcrRegionReference(region)
    if (!parsed) return whole

    return replacer({
      ...parsed,
      source: whole,
      token: `region-${nextToken++}`,
    })
  })
}
```

Refactor `protectOcrRegionReferences` to use the same token numbering and preserve its current malformed-`page=` escaping behavior. Do not change `renderOcrHtml`'s existing discreet visual fallback; the new export module owns the readable export fallback.

- [ ] **Step 3: Implement `prepareOcrExport` without mutating the source**

Implement `apps/desktop/src/lib/ocr-export.ts` with these exact rules:

```ts
import {
  escapeHtml,
  renderOcrMarkup,
  replaceOcrRegionPlaceholders,
  replaceOcrRegionReferences,
  resolveOcrRegion,
  sanitizeOcrHtml,
  type OcrRegionResolver,
  type OcrRenderContext,
} from './ocr-rich-text'

const OCR_EXPORT_FALLBACK_MARKDOWN = '*[Imagen OCR no disponible]*'
const OCR_EXPORT_FALLBACK_HTML = '<span>Imagen OCR no disponible</span>'

export async function prepareOcrExport(
  input: OcrExportInput,
  resolveRegion?: OcrRegionResolver
): Promise<PreparedOcrExport> {
  const normalizedSource = input.source.replace(/\r\n?/g, '\n')
  const markup = renderOcrMarkup(normalizedSource)
  const resolver = resolveRegion ?? resolveOcrRegion
  const htmlReplacements = new Map<string, string>()
  const markdownReplacements = new Map<string, string>()

  await Promise.all(
    markup.references.map(async (reference) => {
      try {
        const dataUrl = await resolver(reference, input)
        if (!/^data:image\/[a-z0-9.+-]+;base64,[a-z0-9+/]+=*$/i.test(dataUrl)) {
          throw new Error('OCR export resolver returned an unsafe image')
        }

        const alt = `OCR region from page ${reference.page + 1}`
        htmlReplacements.set(
          reference.token,
          `<img src="${escapeHtml(dataUrl)}" alt="${escapeHtml(alt)}" />`
        )
        markdownReplacements.set(reference.token, `![${alt}](${dataUrl})`)
      } catch {
        htmlReplacements.set(reference.token, OCR_EXPORT_FALLBACK_HTML)
        markdownReplacements.set(reference.token, OCR_EXPORT_FALLBACK_MARKDOWN)
      }
    })
  )

  const markdown = replaceOcrRegionReferences(
    normalizedSource,
    (reference) => markdownReplacements.get(reference.token) ?? OCR_EXPORT_FALLBACK_MARKDOWN
  )
  const html = sanitizeOcrHtml(
    replaceOcrRegionPlaceholders(
      markup.html,
      new Map(
        markup.references.map((reference) => [
          reference.token,
          htmlReplacements.get(reference.token) ?? OCR_EXPORT_FALLBACK_HTML,
        ])
      )
    )
  )

  return { markdown, html }
}
```

Use the existing exported `escapeHtml` helper rather than adding a second HTML escaping implementation. Import `resolveOcrRegion` from `ocr-rich-text.ts` as the default resolver. The `input.source` value must not be assigned back to component state or passed through any mutating helper.

- [ ] **Step 4: Run the preparation tests and commit the pure export boundary**

```powershell
pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-rich-text.test.ts src/lib/ocr-export.test.ts
```

Expected: PASS for parser regressions, repeated references, Markdown/HTML preservation, embedded data URIs, and readable fallback output.

```powershell
git add apps/desktop/src/lib/ocr-rich-text.ts apps/desktop/src/lib/ocr-rich-text.test.ts apps/desktop/src/lib/ocr-export.ts apps/desktop/src/lib/ocr-export.test.ts
git commit -m "feat(desktop): prepare OCR exports from rendered content"
```

---

### Task 2: Add PDF/DOCX adapters and native file saving

**Files:**
- Modify: `apps/desktop/package.json`
- Modify: `pnpm-lock.yaml`
- Create: `apps/desktop/src/types/ocr-export-libraries.d.ts`
- Modify: `apps/desktop/src/lib/ocr-export.ts`
- Modify: `apps/desktop/src/lib/ocr-export.test.ts`

**Interfaces:**

```ts
export interface OcrExportGenerators {
  pdf: (html: string) => Promise<Uint8Array>
  docx: (html: string) => Promise<Uint8Array>
}

export interface OcrExportRuntime {
  resolveRegion?: OcrRegionResolver
  generators?: Partial<OcrExportGenerators>
}

export function generateOcrExportBytes(
  format: OcrExportFormat,
  document: PreparedOcrExport,
  generators?: Partial<OcrExportGenerators>
): Promise<Uint8Array>

export function exportOcrText(
  input: OcrExportInput,
  format: OcrExportFormat,
  defaultName: string,
  runtime?: OcrExportRuntime
): Promise<string | null>
```

- [ ] **Step 1: Add the browser export dependencies and declarations**

Run the workspace package-manager command from the repository root:

```powershell
pnpm add --filter @entropia-pro/desktop html2pdf.js html-docx-js
```

Expected: `apps/desktop/package.json` has direct dependencies for both generators and `pnpm-lock.yaml` records them under the desktop importer. Do not add a Rust crate or a second PDF engine.

Create `apps/desktop/src/types/ocr-export-libraries.d.ts` so the adapters use explicit browser types even if either package does not ship declarations:

```ts
declare module 'html2pdf.js' {
  interface Html2PdfWorker {
    set(options: Record<string, unknown>): Html2PdfWorker
    from(element: HTMLElement): Html2PdfWorker
    outputPdf(type: 'arraybuffer'): Promise<ArrayBuffer>
  }

  interface Html2PdfFactory {
    (): Html2PdfWorker
  }

  const html2pdf: Html2PdfFactory
  export default html2pdf
}

declare module 'html-docx-js' {
  export function asBlob(html: string, options?: Record<string, unknown>): Blob
}
```

- [ ] **Step 2: Write failing adapter and save-flow tests**

Add injected generator tests to `apps/desktop/src/lib/ocr-export.test.ts`:

```ts
import { afterEach, describe, expect, it, vi } from 'vitest'
import { save } from '@tauri-apps/plugin-dialog'
import { writeFile } from '@tauri-apps/plugin-fs'
import { exportOcrText, generateOcrExportBytes } from './ocr-export'

const prepared = {
  markdown: '# Título\n',
  html: '<h1>Título</h1><img src="data:image/png;base64,AAAA" alt="crop" />',
}

afterEach(() => {
  vi.mocked(save).mockReset()
  vi.mocked(writeFile).mockReset()
})

it('routes PDF and DOCX through the same prepared HTML', async () => {
  const pdf = vi.fn(async (html: string) => {
    expect(html).toContain('<h1>Título</h1>')
    expect(html).toContain('data:image/png;base64,AAAA')
    return Uint8Array.from([1, 2])
  })
  const docx = vi.fn(async (html: string) => {
    expect(html).toContain('<h1>Título</h1>')
    expect(html).toContain('data:image/png;base64,AAAA')
    return Uint8Array.from([3, 4])
  })

  await expect(generateOcrExportBytes('pdf', prepared, { pdf, docx })).resolves.toEqual(
    Uint8Array.from([1, 2])
  )
  await expect(generateOcrExportBytes('docx', prepared, { pdf, docx })).resolves.toEqual(
    Uint8Array.from([3, 4])
  )
  expect(pdf).toHaveBeenCalledTimes(1)
  expect(docx).toHaveBeenCalledTimes(1)
})

it('writes Markdown only after the user chooses a save path', async () => {
  vi.mocked(save).mockResolvedValue('/exports/scan-ocr.md')
  vi.mocked(writeFile).mockResolvedValue(undefined)

  await expect(
    exportOcrText(
      {
        source: '# Título',
        assetUrl: 'asset://source',
        sourceType: 'image',
        referenceWidth: 100,
        referenceHeight: 100,
      },
      'markdown',
      'scan-ocr.md',
      { generators: { pdf: vi.fn(), docx: vi.fn() } }
    )
  ).resolves.toBe('/exports/scan-ocr.md')

  expect(writeFile).toHaveBeenCalledWith('/exports/scan-ocr.md', expect.any(Uint8Array))
  const bytes = vi.mocked(writeFile).mock.calls[0]![1] as Uint8Array
  expect(new TextDecoder().decode(bytes)).toContain('# Título')
})

it('does not write when the save dialog is cancelled', async () => {
  vi.mocked(save).mockResolvedValue(null)

  await expect(
    exportOcrText(
      {
        source: '# Título',
        assetUrl: 'asset://source',
        sourceType: 'image',
        referenceWidth: 100,
        referenceHeight: 100,
      },
      'markdown',
      'scan-ocr.md',
      { generators: { pdf: vi.fn(), docx: vi.fn() } }
    )
  ).resolves.toBeNull()

  expect(writeFile).not.toHaveBeenCalled()
})
```

Run the test before implementation:

```powershell
pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-export.test.ts
```

Expected: FAIL because the adapter functions, dependencies, and save flow are not implemented.

- [ ] **Step 3: Implement the shared export stylesheet and PDF adapter**

In `ocr-export.ts`, define one export stylesheet and wrap the prepared HTML in a complete document string. Keep it independent from the dark application theme:

```ts
const OCR_EXPORT_STYLES = `
  :root { color-scheme: light; }
  body { margin: 0; color: #1f2937; background: #ffffff; font: 11pt/1.55 system-ui, sans-serif; }
  h1, h2, h3, h4, h5, h6 { color: #111827; line-height: 1.2; break-after: avoid; }
  p, ul, ol, blockquote, pre, table { margin: 0 0 12pt; }
  ul, ol { padding-inline-start: 24pt; }
  blockquote { border-inline-start: 2pt solid #9ca3af; padding-inline-start: 10pt; color: #4b5563; }
  table { width: 100%; border-collapse: collapse; break-inside: avoid; }
  th, td { border: 0.5pt solid #9ca3af; padding: 5pt; vertical-align: top; }
  pre { padding: 8pt; background: #f3f4f6; white-space: pre-wrap; break-inside: avoid; }
  img { display: block; max-width: 100%; height: auto; break-inside: avoid; }
`

function buildExportHtml(document: PreparedOcrExport): string {
  return `<!doctype html><html><head><meta charset="utf-8"><style>${OCR_EXPORT_STYLES}</style></head><body>${document.html}</body></html>`
}
```

Implement `generatePdfBytes` with a visible-to-layout-but-offscreen temporary element. Do not use `display:none`, because html2canvas cannot measure hidden content:

```ts
async function generatePdfBytes(html: string): Promise<Uint8Array> {
  const { default: html2pdf } = await import('html2pdf.js')
  const container = document.createElement('div')
  container.style.position = 'fixed'
  container.style.insetInlineStart = '-100000px'
  container.style.insetBlockStart = '0'
  container.style.width = '180mm'
  container.innerHTML = html
  document.body.append(container)

  try {
    const output = await html2pdf()
      .set({
        margin: [12, 12, 12, 12],
        image: { type: 'png', quality: 1 },
        pagebreak: { mode: ['css', 'legacy'], avoid: ['img', 'tr', 'pre'] },
        html2canvas: { scale: 2, useCORS: false, backgroundColor: '#ffffff' },
        jsPDF: { unit: 'mm', format: 'a4', orientation: 'portrait', compressPDF: true },
      })
      .from(container)
      .outputPdf('arraybuffer')

    return new Uint8Array(output)
  } finally {
    container.remove()
  }
}
```

Pass `buildExportHtml(document)` to both the PDF and DOCX adapters so they share all content and inline images.

- [ ] **Step 4: Implement the DOCX adapter and native save flow**

Implement the browser DOCX adapter and format routing:

```ts
async function generateDocxBytes(html: string): Promise<Uint8Array> {
  const { asBlob } = await import('html-docx-js')
  const blob = asBlob(html, { orientation: 'portrait', margins: { top: 720, right: 720, bottom: 720, left: 720 } })
  return new Uint8Array(await blob.arrayBuffer())
}

export async function generateOcrExportBytes(
  format: OcrExportFormat,
  document: PreparedOcrExport,
  generators: Partial<OcrExportGenerators> = {}
): Promise<Uint8Array> {
  if (format === 'markdown') return new TextEncoder().encode(document.markdown)

  const html = buildExportHtml(document)
  if (format === 'pdf') return (generators.pdf ?? generatePdfBytes)(html)
  return (generators.docx ?? generateDocxBytes)(html)
}
```

Use the existing `save`/`writeFile` pattern from `apps/desktop/src/lib/export.ts`:

```ts
const EXPORT_OPTIONS = {
  markdown: { name: 'Markdown', extension: 'md' },
  pdf: { name: 'PDF', extension: 'pdf' },
  docx: { name: 'Microsoft Word', extension: 'docx' },
} as const

export async function exportOcrText(
  input: OcrExportInput,
  format: OcrExportFormat,
  defaultName: string,
  runtime: OcrExportRuntime = {}
): Promise<string | null> {
  const document = await prepareOcrExport(input, runtime.resolveRegion)
  const option = EXPORT_OPTIONS[format]
  const filePath = await save({
    defaultPath: defaultName,
    filters: [{ name: option.name, extensions: [option.extension] }],
  })
  if (!filePath) return null

  const bytes = await generateOcrExportBytes(format, document, runtime.generators)
  await writeFile(filePath, bytes)
  return filePath
}
```

Generate the bytes after a path is selected so canceling the dialog does not spend time resolving crops or rendering a document. The UI must pass a default name with the correct extension, but the native filter remains authoritative.

- [ ] **Step 5: Run adapter tests and commit the format boundary**

```powershell
pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-export.test.ts
```

Expected: PASS for shared HTML input, Markdown UTF-8 bytes, PDF/DOCX injected generators, save cancellation, and native writes.

```powershell
git add apps/desktop/package.json pnpm-lock.yaml apps/desktop/src/types/ocr-export-libraries.d.ts apps/desktop/src/lib/ocr-export.ts apps/desktop/src/lib/ocr-export.test.ts
git commit -m "feat(desktop): add OCR Markdown PDF and DOCX adapters"
```

---

### Task 3: Add component regression coverage for scope and state transitions

**Files:**
- Create: `apps/desktop/src/views/ItemAssetPanel.test.ts`
- Create: `apps/desktop/src/views/__mocks__/MockDocumentViewer.svelte`
- Create: `apps/desktop/src/views/__mocks__/MockOcrRichText.svelte`
- Modify: `apps/desktop/src/lib/i18n.test.ts`

**Interfaces:**

The test must render the real `ItemAssetPanel.svelte` while mocking only the heavy document/rich-content children and the export module. The component receives the existing full prop surface; the test factory must provide every callback required by the current props type.

- [ ] **Step 1: Add deterministic child mocks**

Create `MockDocumentViewer.svelte`:

```svelte
<script lang="ts">
  let { path }: { path: string } = $props()
</script>

<div data-testid="mock-document-viewer">{path}</div>
```

Create `MockOcrRichText.svelte`:

```svelte
<script lang="ts">
  let { text }: { text: string } = $props()
</script>

<div data-testid="mock-ocr-rich-text">{text}</div>
```

- [ ] **Step 2: Add the component test harness and failing behavior tests**

Mock `$lib/ocr-export` with a controllable promise and mock only `DocumentViewer`/`OcrRichText` while preserving the real `IconButton`, `ActionIcon`, `TabButton`, and `TabList` exports:

```ts
import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { Asset } from '@entropia/store'
import ItemAssetPanel from './ItemAssetPanel.svelte'

const { clipboardWriteTextMock, exportOcrTextMock } = vi.hoisted(() => ({
  clipboardWriteTextMock: vi.fn<(value: string) => Promise<void>>(),
  exportOcrTextMock: vi.fn(),
}))

vi.mock('$lib/ocr-export', () => ({
  exportOcrText: exportOcrTextMock,
}))

vi.mock('../components/OcrRichText.svelte', async () => ({
  default: (await import('./__mocks__/MockOcrRichText.svelte')).default,
}))

vi.mock('@entropia/ui', async (importOriginal) => {
  const actual = await importOriginal<typeof import('@entropia/ui')>()
  return {
    ...actual,
    DocumentViewer: (await import('./__mocks__/MockDocumentViewer.svelte')).default,
  }
})

const source = '# Fuente\n\n<div>HTML</div>\n\n![](page=0,bbox=[1,2,3,4])\n'

function makeProps() {
  return {
    selectedAsset: {
      id: 'asset-1',
      type: 'image',
      path: 'C:/assets/scan.png',
      filename: 'scan.png',
    } as Asset,
    viewerSrc: 'asset://scan',
    viewerType: 'image' as const,
    annotations: [],
    layoutRegions: [],
    showLayoutOverlay: false,
    hoveredLayoutRegionId: null,
    selectedLayoutRegionId: null,
    layoutReferenceWidth: 100,
    layoutReferenceHeight: 100,
    selectedAnnotationId: null,
    annotationTool: 'select' as const,
    annotationColor: '#ff0000',
    editTool: 'none' as const,
    canUndo: false,
    canRedo: false,
    viewerPage: 0,
    annotationSaveError: null,
    ocrState: { status: 'done', progress: 100, method: 'glm_ocr' },
    ocrEditedText: source,
    transcriptionState: null,
    transcriptionEditedText: '',
    documentViewerLabels: {} as never,
    annotationToolbarLabels: {} as never,
    translate: (key: string, params?: Record<string, string | number>) =>
      params?.count === undefined ? key : `${key}:${params.count}`,
    onAnnotationsChange: vi.fn(),
    onSelectedAnnotationIdChange: vi.fn(),
    onAnnotationToolChange: vi.fn(),
    onAnnotationColorChange: vi.fn(),
    onLayoutRegionHoverChange: vi.fn(),
    onLayoutRegionSelect: vi.fn(),
    onEditSelect: vi.fn(),
    onEditToolChange: vi.fn(),
    onRotateLeft: vi.fn(),
    onRotateRight: vi.fn(),
    onFineRotateCommit: vi.fn(),
    onUndo: vi.fn(),
    onRedo: vi.fn(),
    onDuplicateAsset: vi.fn(),
    duplicateAssetDisabled: false,
    onPageChange: vi.fn(),
    onDimensionsChange: vi.fn(),
  }
}

beforeEach(() => {
  Object.defineProperty(navigator, 'clipboard', {
    configurable: true,
    value: { writeText: clipboardWriteTextMock },
  })
  clipboardWriteTextMock.mockReset().mockResolvedValue(undefined)
  exportOcrTextMock.mockReset().mockResolvedValue('/exports/scan-texto-extraido.md')
})

afterEach(() => {
  vi.clearAllMocks()
})
```

Add tests for the observable contract:

```ts
it('keeps actions hidden with the document tab and shows them only in extracted text', async () => {
  render(ItemAssetPanel, makeProps())

  const copy = screen.getByRole('button', { name: 'item.copyExtractedTextAria' })
  expect(copy).not.toBeVisible()

  await fireEvent.click(screen.getByRole('tab', { name: 'item.extractedTextTab' }))
  expect(copy).toBeVisible()
  expect(screen.getByRole('button', { name: 'item.downloadExtractedTextAria' })).toBeVisible()
})

it('copies the exact OCR source string without trimming or rendering it', async () => {
  render(ItemAssetPanel, makeProps())
  await fireEvent.click(screen.getByRole('tab', { name: 'item.extractedTextTab' }))
  await fireEvent.click(screen.getByRole('button', { name: 'item.copyExtractedTextAria' }))

  expect(clipboardWriteTextMock).toHaveBeenCalledWith(source)
})

it('opens exactly the three download formats and routes the selected format', async () => {
  render(ItemAssetPanel, makeProps())
  await fireEvent.click(screen.getByRole('tab', { name: 'item.extractedTextTab' }))
  await fireEvent.click(screen.getByRole('button', { name: 'item.downloadExtractedTextAria' }))

  expect(screen.getByRole('menu', { name: 'item.downloadExtractedTextMenu' })).toBeInTheDocument()
  expect(screen.getByRole('menuitem', { name: 'item.exportExtractedTextMarkdown' })).toBeVisible()
  expect(screen.getByRole('menuitem', { name: 'item.exportExtractedTextPdf' })).toBeVisible()
  expect(screen.getByRole('menuitem', { name: 'item.exportExtractedTextDocx' })).toBeVisible()
  expect(screen.getAllByRole('menuitem')).toHaveLength(3)

  await fireEvent.click(screen.getByRole('menuitem', { name: 'item.exportExtractedTextDocx' }))

  expect(exportOcrTextMock).toHaveBeenCalledWith(
    expect.objectContaining({ source, assetUrl: 'asset://scan', sourceType: 'image' }),
    'docx',
    'scan-texto-extraido.docx'
  )
})

it('disables both actions while an export is pending and closes on Escape', async () => {
  let resolveExport!: (path: string | null) => void
  exportOcrTextMock.mockReturnValueOnce(new Promise<string | null>((resolve) => { resolveExport = resolve }))

  render(ItemAssetPanel, makeProps())
  await fireEvent.click(screen.getByRole('tab', { name: 'item.extractedTextTab' }))
  const download = screen.getByRole('button', { name: 'item.downloadExtractedTextAria' })
  await fireEvent.click(download)
  await fireEvent.click(screen.getByRole('menuitem', { name: 'item.exportExtractedTextPdf' }))

  expect(screen.getByRole('button', { name: 'item.copyExtractedTextAria' })).toBeDisabled()
  expect(download).toBeDisabled()

  resolveExport('/exports/scan-texto-extraido.pdf')
  await waitFor(() => expect(download).not.toBeDisabled())

  await fireEvent.click(download)
  expect(screen.getByRole('menu')).toBeInTheDocument()
  await fireEvent.keyDown(window, { key: 'Escape' })
  expect(screen.queryByRole('menu')).not.toBeInTheDocument()
  expect(document.activeElement).toBe(download)
})
```

Run before implementation:

```powershell
pnpm --filter @entropia-pro/desktop test -- src/views/ItemAssetPanel.test.ts
```

Expected: FAIL until the UI, i18n keys, and export handler are implemented.

- [ ] **Step 3: Add locale assertions**

Extend `apps/desktop/src/lib/i18n.test.ts` to assert the new keys exist in both locales and contain the required labels. Keep the existing locale switch pattern:

```ts
it('translates extracted-text actions and export formats in Spanish and English', () => {
  locale.set('es')
  expect(t('item.copyExtractedText')).toBe('Copiar')
  expect(t('item.exportExtractedTextMarkdown')).toContain('.md')
  expect(t('item.exportExtractedTextPdf')).toContain('.pdf')
  expect(t('item.exportExtractedTextDocx')).toContain('.docx')

  locale.set('en')
  expect(t('item.copyExtractedText')).toBe('Copy')
  expect(t('item.exportExtractedTextDocx')).toContain('.docx')
})
```

- [ ] **Step 4: Run the red regression set and commit the component tests**

```powershell
pnpm --filter @entropia-pro/desktop test -- src/views/ItemAssetPanel.test.ts src/lib/i18n.test.ts src/lib/ocr-export.test.ts src/lib/ocr-rich-text.test.ts
```

Expected: OCR export/rich-text tests pass; the new component and locale assertions fail because Task 4 has not added the UI/i18n implementation yet.

```powershell
git add apps/desktop/src/views/ItemAssetPanel.test.ts apps/desktop/src/views/__mocks__/MockDocumentViewer.svelte apps/desktop/src/views/__mocks__/MockOcrRichText.svelte apps/desktop/src/lib/i18n.test.ts
git commit -m "test(desktop): cover OCR extraction actions"
```

---
### Task 4: Add localized labels and the extracted-text actions

**Files:**
- Modify: `apps/desktop/src/lib/i18n.ts`
- Modify: `apps/desktop/src/views/ItemAssetPanel.svelte`

**Interfaces:**

Add these keys to both `es` and `en` objects in `apps/desktop/src/lib/i18n.ts`; adding them to `es` automatically includes them in `I18nKey`:

```ts
'item.copyExtractedText': 'Copiar'
'item.copyExtractedTextAria': 'Copiar texto extraído'
'item.copyExtractedTextSuccess': 'Texto extraído copiado'
'item.copyExtractedTextError': 'No se pudo copiar el texto extraído'
'item.downloadExtractedText': 'Descargar'
'item.downloadExtractedTextAria': 'Descargar texto extraído'
'item.downloadExtractedTextMenu': 'Formatos de descarga del texto extraído'
'item.exportExtractedTextMarkdown': 'Documento Markdown (.md)'
'item.exportExtractedTextPdf': 'Documento PDF (.pdf)'
'item.exportExtractedTextDocx': 'Microsoft Word (.docx)'
'item.exportExtractedTextWorking': 'Generando exportación…'
'item.exportExtractedTextError': 'No se pudo generar la exportación'
'item.ocrImageUnavailable': 'Imagen OCR no disponible'
```

Use equivalent English values in the `en` object:

```ts
'item.copyExtractedText': 'Copy'
'item.copyExtractedTextAria': 'Copy extracted text'
'item.copyExtractedTextSuccess': 'Extracted text copied'
'item.copyExtractedTextError': 'Could not copy extracted text'
'item.downloadExtractedText': 'Download'
'item.downloadExtractedTextAria': 'Download extracted text'
'item.downloadExtractedTextMenu': 'Extracted text download formats'
'item.exportExtractedTextMarkdown': 'Markdown document (.md)'
'item.exportExtractedTextPdf': 'PDF document (.pdf)'
'item.exportExtractedTextDocx': 'Microsoft Word (.docx)'
'item.exportExtractedTextWorking': 'Generating export…'
'item.exportExtractedTextError': 'Could not generate export'
'item.ocrImageUnavailable': 'OCR image unavailable'
```

- [ ] **Step 1: Run the failing component interaction contract**

The component regression from Task 3 is the red contract for this UI unit. Run it before editing `ItemAssetPanel.svelte`:

```powershell
pnpm --filter @entropia-pro/desktop test -- src/views/ItemAssetPanel.test.ts
```

Expected: FAIL because the action buttons, menu, and handlers do not exist.

- [ ] **Step 2: Add imports and local Svelte state**

Extend the existing UI import with `ActionIcon` and `IconButton`, import `onDestroy`, and import the export API:

```ts
import { onDestroy } from 'svelte'
import {
  ActionIcon,
  DocumentViewer,
  IconButton,
  TabButton,
  TabList,
  // existing types
} from '@entropia/ui'
import {
  exportOcrText,
  type OcrExportFormat,
} from '$lib/ocr-export'
```

Add state next to `leftPanelTab`/`currentAssetId`:

```ts
let downloadMenuOpen = $state(false)
let copyFeedback = $state<'idle' | 'success' | 'error'>('idle')
let exportingFormat = $state<OcrExportFormat | null>(null)
let exportError = $state(false)
let downloadContainerEl = $state<HTMLElement | null>(null)
let downloadTriggerEl = $state<HTMLButtonElement | null>(null)
let feedbackTimer: ReturnType<typeof setTimeout> | undefined
let exportGeneration = 0

const exportMenuId = 'left-panel-extracted-text-export-menu'
```

Reset menu and stale statuses in the existing selected-asset effect. Increment `exportGeneration` when the asset changes so a previous export cannot update the next asset's feedback. Clear `feedbackTimer` in `onDestroy`.

- [ ] **Step 3: Implement exact clipboard and export handlers**

Add these functions below `loadAudioFallbackBlob`:

```ts
function showCopyFeedback(next: 'success' | 'error') {
  copyFeedback = next
  if (feedbackTimer) clearTimeout(feedbackTimer)
  feedbackTimer = setTimeout(() => {
    copyFeedback = 'idle'
    feedbackTimer = undefined
  }, 2200)
}

async function handleCopyExtractedText() {
  try {
    await navigator.clipboard.writeText(ocrEditedText)
    showCopyFeedback('success')
  } catch {
    showCopyFeedback('error')
  }
}

function closeDownloadMenu(restoreFocus = false) {
  downloadMenuOpen = false
  if (restoreFocus) downloadTriggerEl?.focus()
}

function handleWindowKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && downloadMenuOpen) {
    event.preventDefault()
    closeDownloadMenu(true)
  }
}

function handleWindowPointerdown(event: PointerEvent) {
  const target = event.target
  if (downloadMenuOpen && target instanceof Node && !downloadContainerEl?.contains(target)) {
    closeDownloadMenu()
  }
}

function buildExportDefaultName(format: OcrExportFormat): string {
  const filename = selectedAsset?.filename || 'texto-extraido'
  const stem = filename.replace(/\.[^.]+$/, '') || 'texto-extraido'
  return `${stem}-texto-extraido.${format === 'markdown' ? 'md' : format}`
}

async function handleExport(format: OcrExportFormat) {
  const asset = selectedAsset
  const source = ocrEditedText
  if (!asset || !source.trim() || exportingFormat) return

  const generation = ++exportGeneration
  exportingFormat = format
  exportError = false
  closeDownloadMenu()

  try {
    await exportOcrText(
      {
        source,
        assetUrl: viewerSrc,
        sourceType: viewerType === 'pdf' ? 'pdf' : 'image',
        referenceWidth: layoutReferenceWidth,
        referenceHeight: layoutReferenceHeight,
      },
      format,
      buildExportDefaultName(format)
    )
  } catch {
    if (generation === exportGeneration) exportError = true
  } finally {
    if (generation === exportGeneration) exportingFormat = null
  }
}

onDestroy(() => {
  if (feedbackTimer) clearTimeout(feedbackTimer)
})
```

Use the captured `source`, `asset`, and current viewer context for the operation. Do not call `ocrEditedText.trim()` as the value passed to Copy or export.

- [ ] **Step 4: Add the header actions and accessible download menu**

Replace only the OCR metadata block at lines 215–223 with this structure:

```svelte
<div class="left-text-panel-meta">
  <div class="left-text-panel-meta__details">
    <span>{translate('item.extractedText')}</span>
    <span class="ocr-meta">
      via {ocrState?.method ?? translate('item.ocrMethodUnknown')} · {translate(
        'item.characters',
        { count: ocrEditedText.length }
      )}
    </span>
  </div>

  <div
    bind:this={downloadContainerEl}
    class="left-text-panel-actions"
    aria-live="polite"
  >
    <IconButton
      size="sm"
      variant="ghost"
      label={translate('item.copyExtractedTextAria')}
      title={translate('item.copyExtractedText')}
      disabled={exportingFormat !== null}
      onclick={() => void handleCopyExtractedText()}
    >
      <ActionIcon name="copy" size={14} />
    </IconButton>

    <IconButton
      bind:this={downloadTriggerEl}
      size="sm"
      variant="ghost"
      label={translate('item.downloadExtractedTextAria')}
      title={translate('item.downloadExtractedText')}
      active={downloadMenuOpen}
      disabled={exportingFormat !== null}
      aria-expanded={downloadMenuOpen ? 'true' : 'false'}
      aria-controls={downloadMenuOpen ? exportMenuId : undefined}
      onclick={() => {
        downloadMenuOpen = !downloadMenuOpen
      }}
    >
      <ActionIcon name="download" size={14} />
    </IconButton>

    {#if downloadMenuOpen}
      <div
        id={exportMenuId}
        class="left-text-panel-export-menu"
        role="menu"
        aria-label={translate('item.downloadExtractedTextMenu')}
      >
        <button
          type="button"
          role="menuitem"
          disabled={exportingFormat !== null}
          onclick={() => void handleExport('markdown')}
        >{translate('item.exportExtractedTextMarkdown')}</button>
        <button
          type="button"
          role="menuitem"
          disabled={exportingFormat !== null}
          onclick={() => void handleExport('pdf')}
        >{translate('item.exportExtractedTextPdf')}</button>
        <button
          type="button"
          role="menuitem"
          disabled={exportingFormat !== null}
          onclick={() => void handleExport('docx')}
        >{translate('item.exportExtractedTextDocx')}</button>
      </div>
    {/if}

    {#if copyFeedback !== 'idle'}
      <span class="sr-only" role="status">
        {translate(copyFeedback === 'success'
          ? 'item.copyExtractedTextSuccess'
          : 'item.copyExtractedTextError')}
      </span>
    {/if}
    {#if exportError}
      <span class="sr-only" role="alert">{translate('item.exportExtractedTextError')}</span>
    {/if}
  </div>
</div>
```

Add window handlers near the top-level markup:

```svelte
<svelte:window onkeydown={handleWindowKeydown} onpointerdown={handleWindowPointerdown} />
```

Keep the `{:else}` empty-text branch and audio transcription branch unchanged. The buttons therefore remain scoped to non-empty, non-audio extracted text.

- [ ] **Step 5: Add local styles without changing other panels**

Extend the existing `ItemAssetPanel.svelte` style block:

```css
.left-text-panel-meta {
  position: relative;
}

.left-text-panel-meta__details {
  display: flex;
  min-width: 0;
  align-items: baseline;
  gap: var(--space-2);
  overflow: hidden;
}

.left-text-panel-meta__details > :first-child {
  flex: 0 0 auto;
}

.left-text-panel-meta__details .ocr-meta {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.left-text-panel-actions {
  position: relative;
  display: inline-flex;
  flex: 0 0 auto;
  align-items: center;
  gap: var(--space-1);
}

.left-text-panel-export-menu {
  position: absolute;
  z-index: 5;
  top: calc(100% + var(--space-1));
  right: 0;
  display: grid;
  min-width: 13rem;
  gap: var(--space-1);
  padding: var(--space-1);
  border: 1px solid var(--border-panel);
  border-radius: var(--radius-dialog);
  background: color-mix(in srgb, var(--color-surface-elevated) 96%, var(--color-bg));
  box-shadow: var(--shadow-lg);
}

.left-text-panel-export-menu button {
  display: flex;
  width: 100%;
  align-items: center;
  padding: var(--space-2) var(--space-3);
  border: 0;
  border-radius: var(--radius-xs);
  background: transparent;
  color: var(--color-text-secondary);
  font: inherit;
  text-align: start;
  cursor: pointer;
}

.left-text-panel-export-menu button:hover:not(:disabled),
.left-text-panel-export-menu button:focus-visible {
  background: var(--color-accent-faint);
  color: var(--color-text-primary);
}

.left-text-panel-export-menu button:focus-visible {
  outline: none;
  box-shadow: var(--focus-ring);
}

.left-text-panel-export-menu button:disabled {
  cursor: not-allowed;
  opacity: 0.48;
}
```

Use the same `--border-panel`, `--radius-dialog`, `--color-surface-elevated`, `--color-bg`, and `--shadow-lg` tokens as the existing TopBar language menu. Do not introduce a new global token. Preserve the existing card/body/scroll styles.

- [ ] **Step 6: Run the component and i18n tests, then commit the UI unit**

```powershell
pnpm --filter @entropia-pro/desktop test -- src/views/ItemAssetPanel.test.ts src/lib/i18n.test.ts
```

Expected: PASS for visible scoped actions, exact source copy, menu semantics, keyboard dismissal, localized labels, and unchanged audio/document branches.

```powershell
git add apps/desktop/src/lib/i18n.ts apps/desktop/src/views/ItemAssetPanel.svelte apps/desktop/src/views/ItemAssetPanel.test.ts apps/desktop/src/views/__mocks__/MockDocumentViewer.svelte apps/desktop/src/views/__mocks__/MockOcrRichText.svelte
git commit -m "feat(desktop): add OCR copy and download actions"
```

---


### Task 5: Run final typecheck, surface smoke verification, and cleanup

**Files:**
- Modify only if verification reveals a concrete defect: the files from Tasks 1–4.

**Interfaces:**

No new public interface. This task validates the immutable behavior implemented above and removes only temporary export DOM nodes or stale test scaffolding that is proven unused.

- [ ] **Step 1: Run the focused desktop test suite**

```powershell
pnpm --filter @entropia-pro/desktop test -- src/views/ItemAssetPanel.test.ts src/lib/i18n.test.ts src/lib/ocr-export.test.ts src/lib/ocr-rich-text.test.ts
```

Expected: PASS with no unhandled promise rejection, no unresolved dynamic-import warning, and no snapshot changes.

- [ ] **Step 2: Run Lite typecheck**

```powershell
$env:VITE_LOCAL_ML='0'; pnpm --filter @entropia-pro/desktop typecheck
```

Expected: `svelte-check` exits 0 with no missing translation keys, Svelte event typing errors, undeclared module errors, or adapter type mismatches. Reset the PowerShell environment variable after the command if subsequent work switches variants.

- [ ] **Step 3: Launch the actual Lite desktop surface for a visual smoke check**

From `apps/desktop`, use the repository-supported Tauri command without local-ML features:

```powershell
$env:VITE_LOCAL_ML='0'; pnpm exec tauri dev --config src-tauri/tauri.lite.conf.json
```

Exercise one image/PDF asset with non-empty mixed OCR Markdown/HTML and at least one valid `page/bbox` reference:

1. Open the asset and select `Texto extraído`.
2. Confirm `Copiar` and `Descargar` appear immediately after the existing character metadata and match the compact icon-button styling.
3. Copy the text and compare it with the source including Markdown, HTML, line endings, and trailing whitespace.
4. Open `Descargar`; confirm exactly the three requested options.
5. Generate `.md`; open it and confirm source Markdown/HTML plus embedded crop data.
6. Generate `.pdf`; confirm readable rendered headings, paragraphs, tables, code, and inline crop images.
7. Generate `.docx`; open it in a compatible Word viewer and confirm the same rendered content and crop images.
8. Return to `Documento` and any audio/transcription asset; confirm no action buttons or export menu appear there.
9. Trigger an unresolved crop case and confirm `Imagen OCR no disponible` appears in derived exports without changing the stored OCR.

If the native runtime cannot be launched in the current environment, preserve the concrete launch error and use the focused component/export tests as the available behavioral evidence; do not claim visual success without the surface check.

- [ ] **Step 4: Run repository lint only after source changes are stable**

```powershell
pnpm --filter @entropia-pro/desktop lint
```

Expected: PASS without formatter-driven source changes. If lint identifies a real issue, correct only that issue and rerun the focused tests plus Lite typecheck.

- [ ] **Step 5: Commit only verified cleanup**

Review the final changed files, remove no user code, and commit any concrete verification fix as a focused conventional commit:

```powershell
git add apps/desktop/package.json pnpm-lock.yaml apps/desktop/src/lib/ocr-rich-text.ts apps/desktop/src/lib/ocr-rich-text.test.ts apps/desktop/src/lib/ocr-export.ts apps/desktop/src/lib/ocr-export.test.ts apps/desktop/src/types/ocr-export-libraries.d.ts apps/desktop/src/lib/i18n.ts apps/desktop/src/lib/i18n.test.ts apps/desktop/src/views/ItemAssetPanel.svelte apps/desktop/src/views/ItemAssetPanel.test.ts apps/desktop/src/views/__mocks__/MockDocumentViewer.svelte apps/desktop/src/views/__mocks__/MockOcrRichText.svelte
git commit -m "fix(desktop): stabilize OCR export verification" 
```

Only create this final commit if Task 5 produced a source fix; otherwise leave the already committed implementation units unchanged.
