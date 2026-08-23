# OCR Rich Text Viewer Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render mixed GLM OCR Markdown/HTML with safe inline page/bbox crops only in the left-panel `Texto extraído` tab, without modifying OCR storage or any other viewer surface.

**Architecture:** Add an OCR-specific TypeScript renderer that uses `markdown-it` for Markdown/HTML structure, a local DOM allowlist sanitizer before `{@html}`, and an in-memory canvas/pdfjs crop resolver for zero-based page/bbox references. Mount it only from `ItemAssetPanel.svelte`; reuse the existing `viewerSrc` and layout reference dimensions already supplied by `ItemView`.

**Tech Stack:** Svelte 5 runes, TypeScript, Vitest, Testing Library Svelte, `markdown-it@^14.1.1`, existing `pdfjs-dist@^4`, HTML canvas, Tauri `convertFileSrc` URLs.

## Global Constraints

- Target only `apps/desktop/src/views/ItemAssetPanel.svelte` non-audio `left-panel-text` content.
- Do not modify `apps/desktop/src/lib/markdown.ts`, `DocumentViewer`, `ItemTextPanel.svelte`, OCR state/persistence, Rust OCR code, or database schemas.
- Preserve `ocrEditedText` exactly; generated HTML and crops are display-only and in-memory.
- Interpret `page` as zero-based and `bbox` as `[left, top, right, bottom]`.
- Preserve an invalid/unresolvable OCR image reference as escaped text at its original position.
- Do not fetch arbitrary remote image URLs; only generated source-asset crops may become rendered images.
- Use `VITE_LOCAL_ML=0` for Lite typecheck and avoid full Tauri release builds.

---

### Task 1: Add the OCR parser and sanitizer contract

**Files:**
- Create: `apps/desktop/src/lib/ocr-rich-text.ts`
- Create: `apps/desktop/src/lib/ocr-rich-text.test.ts`
- Modify: `apps/desktop/package.json` (`markdown-it` dependency)
- Modify: `pnpm-lock.yaml` (desktop importer dependency entry)

**Interfaces:**

```ts
export type OcrSourceType = 'image' | 'pdf'

export interface OcrBbox {
  left: number
  top: number
  right: number
  bottom: number
}

export interface OcrRegionReference {
  token: string
  source: string
  page: number
  bbox: OcrBbox
}

export interface OcrRenderContext {
  assetUrl: string
  sourceType: OcrSourceType
  referenceWidth: number
  referenceHeight: number
}

export type OcrRegionResolver = (
  reference: OcrRegionReference,
  context: OcrRenderContext
) => Promise<string>

export interface OcrMarkup {
  html: string
  references: OcrRegionReference[]
}

export function parseOcrRegionReference(source: string): Omit<OcrRegionReference, 'token'> | null
export function renderOcrMarkup(source: string): OcrMarkup
export function sanitizeOcrHtml(html: string): string
export function replaceOcrRegionPlaceholders(
  html: string,
  replacements: ReadonlyMap<string, string>
): string
```

- [ ] **Step 1: Add the direct parser dependency without changing runtime behavior**

Add the existing locked version to `apps/desktop/package.json`:

```json
"markdown-it": "^14.1.1"
```

Update only the lockfile importer with the package manager so the package remains reproducible:

```powershell
pnpm add --filter @entropia-pro/desktop markdown-it@^14.1.1
```

Expected: `apps/desktop/package.json` contains the direct dependency and `pnpm-lock.yaml` records `markdown-it` under `apps/desktop.dependencies`; the resolved package remains `14.1.1`.

- [ ] **Step 2: Write failing parser and sanitizer tests**

Create `apps/desktop/src/lib/ocr-rich-text.test.ts` with deterministic tests for the public contract:

```ts
import { describe, expect, it } from 'vitest'
import {
  parseOcrRegionReference,
  renderOcrMarkup,
  replaceOcrRegionPlaceholders,
  sanitizeOcrHtml,
} from './ocr-rich-text'

describe('parseOcrRegionReference', () => {
  it('parses zero-based page and left/top/right/bottom bbox values', () => {
    expect(parseOcrRegionReference('page=0,bbox=[536,508,1507,1112]')).toEqual({
      source: 'page=0,bbox=[536,508,1507,1112]',
      page: 0,
      bbox: { left: 536, top: 508, right: 1507, bottom: 1112 },
    })
  })

  it('rejects malformed, reversed, negative, and non-finite regions', () => {
    expect(parseOcrRegionReference('page=1,bbox=[1,2,2]')).toBeNull()
    expect(parseOcrRegionReference('page=1,bbox=[2,2,1,3]')).toBeNull()
    expect(parseOcrRegionReference('page=-1,bbox=[1,2,3,4]')).toBeNull()
    expect(parseOcrRegionReference('page=1,bbox=[1,2,Infinity,4]')).toBeNull()
  })
})

describe('renderOcrMarkup', () => {
  it('renders Markdown and mixed centered HTML while extracting image placeholders', () => {
    const result = renderOcrMarkup(
      '<div align="center">\n\n# Título\n\n</div>\n\n**texto**\n\n![](page=0,bbox=[10,20,30,40])'
    )

    expect(result.html).toContain('<h1>Título</h1>')
    expect(result.html).toContain('<strong>texto</strong>')
    expect(result.html).toContain('data-ocr-region-token=')
    expect(result.references).toHaveLength(1)
  })

  it('renders GFM tables and keeps malformed image syntax as text', () => {
    const result = renderOcrMarkup('| A | B |\n| --- | --- |\n| 1 | 2 |\n\n![](page=0,bbox=[1,2,3])')

    expect(result.html).toContain('<table>')
    expect(result.html).toContain('<td>1</td>')
    expect(result.references).toHaveLength(0)
    expect(result.html).toContain('page=0,bbox=[1,2,3]')
  })
})

describe('sanitizeOcrHtml', () => {
  it('keeps semantic OCR elements and safe centered alignment', () => {
    const result = sanitizeOcrHtml(
      '<div align="center"><h2>Seguro</h2><table><tr><td>1</td></tr></table></div>'
    )

    expect(result).toContain('<div align="center">')
    expect(result).toContain('<h2>Seguro</h2>')
    expect(result).toContain('<table>')
  })

  it('removes executable tags, handlers, styles, and unsafe links', () => {
    const result = sanitizeOcrHtml(
      '<script>alert(1)</script><p onclick="alert(2)" style="color:red">Texto</p><a href="javascript:alert(3)">link</a>'
    )

    expect(result).not.toContain('<script')
    expect(result).not.toContain('onclick')
    expect(result).not.toContain('style=')
    expect(result).not.toContain('javascript:')
    expect(result).toContain('Texto')
    expect(result).toContain('link')
  })

  it('replaces a failed region with escaped original source', () => {
    const markup = renderOcrMarkup('antes ![](page=4,bbox=[1,2,3,4]) después')
    const html = replaceOcrRegionPlaceholders(
      markup.html,
      new Map([[markup.references[0]!.token, '&lt;regiona&gt;']])
    )

    expect(sanitizeOcrHtml(html)).toContain('&lt;regiona&gt;')
  })
})
```

Run the new test before implementation:

```powershell
pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-rich-text.test.ts
```

Expected: FAIL because the module and exported functions do not exist.

- [ ] **Step 3: Implement the minimal Markdown/HTML renderer and sanitizer**

Implement `ocr-rich-text.ts` with these concrete rules:

1. Normalize CRLF to LF.
2. Replace valid OCR Markdown image references with synthetic Markdown image destinations (`ocr-region:<token>`) before Markdown-it parses them, retaining each original source in `OcrRegionReference`.
3. Create one module-level Markdown-it instance with `{ html: true, breaks: true, linkify: false }` and enable the `table` rule.
4. Override the image rule so synthetic `ocr-region:` destinations emit `<span data-ocr-region-token="..." aria-hidden="true"></span>`. Render non-OCR images as escaped source text; never emit arbitrary remote `<img src>` values.
5. Sanitize generated HTML by parsing it into a `template`, recursively cloning only the allowlisted tags and attributes. Keep `div align` only for `left|center|right`; keep safe `href` protocols; keep bounded positive `colspan`/`rowspan` and `scope`; keep generated `data:image/*` sources only. Drop `script`, `style`, iframe/object/embed/form content and all `on*`/unknown attributes. Escape text nodes.
6. Replace placeholders with trusted generated replacement strings or already-escaped fallback text. Do not sanitize before replacement if the placeholder is needed; always run `sanitizeOcrHtml` on the final string.
7. In non-DOM environments, return escaped text rather than raw HTML.

Use the existing `packages/ui/src/components/NoteEditor/note-content.ts` sanitizer as the recursion/style reference, but do not import or modify it because its allowlist intentionally excludes tables and images.

- [ ] **Step 4: Run parser/sanitizer tests to verify the first contract**

```powershell
pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-rich-text.test.ts
```

Expected: PASS for all parser, mixed HTML/Markdown, table, placeholder, and XSS tests. No existing `markdown.test.ts` behavior should change because `apps/desktop/src/lib/markdown.ts` remains untouched.

- [ ] **Step 5: Commit the parser unit**

```powershell
git add apps/desktop/package.json pnpm-lock.yaml apps/desktop/src/lib/ocr-rich-text.ts apps/desktop/src/lib/ocr-rich-text.test.ts
git commit -m "feat(desktop): add safe OCR rich-text parser"
```

---

### Task 2: Implement in-memory image and PDF region resolution

**Files:**
- Modify: `apps/desktop/src/lib/ocr-rich-text.ts`
- Modify: `apps/desktop/src/lib/ocr-rich-text.test.ts`

**Interfaces:**

```ts
export interface OcrCropRect {
  left: number
  top: number
  width: number
  height: number
}

export function scaleOcrBbox(
  bbox: OcrBbox,
  referenceWidth: number,
  referenceHeight: number,
  sourceWidth: number,
  sourceHeight: number
): OcrCropRect

export async function resolveOcrRegion(
  reference: OcrRegionReference,
  context: OcrRenderContext
): Promise<string>

export async function renderOcrHtml(
  source: string,
  context: OcrRenderContext,
  resolveRegion?: OcrRegionResolver
): Promise<string>
```

- [ ] **Step 1: Add failing geometry and replacement tests**

Append tests that prove the observable crop contract and per-reference fallback:

```ts
import { scaleOcrBbox, renderOcrHtml, type OcrRenderContext, type OcrRegionResolver } from './ocr-rich-text'

describe('scaleOcrBbox', () => {
  it('maps OCR reference coordinates to the raster dimensions', () => {
    expect(scaleOcrBbox(
      { left: 100, top: 200, right: 500, bottom: 700 },
      1000,
      2000,
      2000,
      4000
    )).toEqual({ left: 200, top: 400, width: 800, height: 1000 })
  })

  it('rejects missing dimensions and out-of-range coordinates', () => {
    expect(() => scaleOcrBbox(
      { left: 0, top: 0, right: 10, bottom: 10 },
      0,
      100,
      100,
      100
    )).toThrow(/reference dimensions/i)
    expect(() => scaleOcrBbox(
      { left: 0, top: 0, right: 101, bottom: 10 },
      100,
      100,
      100,
      100
    )).toThrow(/bounds/i)
  })
})

describe('renderOcrHtml', () => {
  const context: OcrRenderContext = {
    assetUrl: 'asset://source',
    sourceType: 'image',
    referenceWidth: 100,
    referenceHeight: 100,
  }

  it('places resolved data URLs inline and keeps source order', async () => {
    const resolver: OcrRegionResolver = async () => 'data:image/png;base64,AAAA'
    const html = await renderOcrHtml(
      'antes ![](page=0,bbox=[10,20,30,40]) después',
      context,
      resolver
    )

    expect(html).toContain('src="data:image/png;base64,AAAA"')
    expect(html.indexOf('antes')).toBeLessThan(html.indexOf('<img'))
    expect(html.indexOf('<img')).toBeLessThan(html.indexOf('después'))
  })

  it('keeps only the failed reference as text when a resolver rejects', async () => {
    const resolver: OcrRegionResolver = async () => {
      throw new Error('source unavailable')
    }
    const html = await renderOcrHtml(
      'antes ![](page=0,bbox=[10,20,30,40]) después',
      context,
      resolver
    )

    expect(html).toContain('page=0,bbox=[10,20,30,40]')
    expect(html).not.toContain('<img')
    expect(html).toContain('antes')
    expect(html).toContain('después')
  })
})
```

Run and verify the tests fail at the missing exports:

```powershell
pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-rich-text.test.ts
```

Expected: FAIL only for the new geometry/async exports.

- [ ] **Step 2: Implement coordinate scaling and raster crop helpers**

Implement `scaleOcrBbox` with these exact invariants:

```ts
if (![referenceWidth, referenceHeight, sourceWidth, sourceHeight].every(Number.isFinite)) {
  throw new Error('OCR crop dimensions must be finite')
}
if (referenceWidth <= 0 || referenceHeight <= 0 || sourceWidth <= 0 || sourceHeight <= 0) {
  throw new Error('OCR crop reference dimensions must be positive')
}
if (
  bbox.left < 0 || bbox.top < 0 ||
  bbox.right <= bbox.left || bbox.bottom <= bbox.top ||
  bbox.right > referenceWidth || bbox.bottom > referenceHeight
) {
  throw new Error('OCR crop bbox is outside reference bounds')
}

const scaleX = sourceWidth / referenceWidth
const scaleY = sourceHeight / referenceHeight
const left = Math.floor(bbox.left * scaleX)
const top = Math.floor(bbox.top * scaleY)
const right = Math.ceil(bbox.right * scaleX)
const bottom = Math.ceil(bbox.bottom * scaleY)
if (right <= left || bottom <= top || right > sourceWidth || bottom > sourceHeight) {
  throw new Error('OCR crop bbox is outside source bounds')
}
return { left, top, width: right - left, height: bottom - top }
```

Use an offscreen canvas to draw the source raster and call `toDataURL('image/png')` on the crop canvas. Do not mutate or replace the original asset URL.

- [ ] **Step 3: Implement image and PDF source loading**

For `sourceType === 'image'`:

```ts
const image = new Image()
image.decoding = 'async'
image.src = context.assetUrl
await image.decode()
const sourceWidth = image.naturalWidth
const sourceHeight = image.naturalHeight
// draw image into a source canvas, then crop with scaleOcrBbox
```

Reject a zero-dimension image. If `referenceWidth`/`referenceHeight` are unavailable, use the image natural dimensions as the reference dimensions before scaling.

For `sourceType === 'pdf'`, mirror `DocumentViewer.svelte`:

```ts
const pdfjs = await import('pdfjs-dist')
pdfjs.GlobalWorkerOptions.workerSrc = new URL(
  'pdfjs-dist/build/pdf.worker.min.mjs',
  import.meta.url
).href
const document = await pdfjs.getDocument(context.assetUrl).promise
const page = await document.getPage(reference.page + 1)
const naturalViewport = page.getViewport({ scale: 1 })
const scale = context.referenceWidth > 0
  ? context.referenceWidth / naturalViewport.width
  : 1
const viewport = page.getViewport({ scale })
// render page to an offscreen canvas, then crop with scaleOcrBbox
```

Cache the PDF document and rendered page canvas by `assetUrl` and zero-based page index. Reuse one rendered page for multiple references. Let page/load/render errors reject only that region so `renderOcrHtml` can apply the textual fallback.

- [ ] **Step 4: Implement async replacement orchestration**

`renderOcrHtml` must render the source once, resolve all references independently, and sanitize only the final HTML:

```ts
export async function renderOcrHtml(
  source: string,
  context: OcrRenderContext,
  resolveRegion: OcrRegionResolver = resolveOcrRegion
): Promise<string> {
  const markup = renderOcrMarkup(source)
  const replacements = new Map<string, string>()

  await Promise.all(markup.references.map(async (reference) => {
    try {
      const dataUrl = await resolveRegion(reference, context)
      replacements.set(
        reference.token,
        `<img src="${escapeHtml(dataUrl)}" alt="OCR region from page ${reference.page + 1}" />`
      )
    } catch {
      replacements.set(reference.token, escapeHtml(reference.source))
    }
  }))

  return sanitizeOcrHtml(replaceOcrRegionPlaceholders(markup.html, replacements))
}
```

The implementation must validate that generated data URLs are `data:image/*` before inserting them. Any invalid resolver result follows the same escaped-text fallback.

- [ ] **Step 5: Run resolver tests and commit**

```powershell
pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-rich-text.test.ts
```

Expected: PASS for geometry, inline data URL placement, per-region failure fallback, and parser/sanitizer coverage.

```powershell
git add apps/desktop/src/lib/ocr-rich-text.ts apps/desktop/src/lib/ocr-rich-text.test.ts
git commit -m "feat(desktop): resolve OCR image regions in memory"
```

---

### Task 3: Add the Svelte rich-content component

**Files:**
- Create: `apps/desktop/src/components/OcrRichText.svelte`
- Create: `apps/desktop/src/components/OcrRichText.test.ts`

**Interfaces:**

```ts
interface OcrRichTextProps {
  text: string
  assetUrl: string
  sourceType: 'image' | 'pdf'
  referenceWidth: number
  referenceHeight: number
}
```

- [ ] **Step 1: Write the component lifecycle test first**

Create a component test that mocks `renderOcrHtml` and proves stale async results cannot overwrite newer OCR text:

```ts
import { render, screen, waitFor } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import OcrRichText from './OcrRichText.svelte'

const renderOcrHtmlMock = vi.hoisted(() => vi.fn())
vi.mock('$lib/ocr-rich-text', () => ({
  renderOcrHtml: renderOcrHtmlMock,
}))

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((nextResolve) => {
    resolve = nextResolve
  })
  return { promise, resolve }
}

describe('OcrRichText', () => {
  it('renders resolved HTML and ignores an older async render', async () => {
    const first = deferred<string>()
    const second = deferred<string>()
    renderOcrHtmlMock.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)

    const view = render(OcrRichText, {
      text: '# viejo',
      assetUrl: 'asset://source',
      sourceType: 'image',
      referenceWidth: 100,
      referenceHeight: 100,
    })
    await waitFor(() => expect(renderOcrHtmlMock).toHaveBeenCalledTimes(1))

    view.rerender({
      text: '# nuevo',
      assetUrl: 'asset://source',
      sourceType: 'image',
      referenceWidth: 100,
      referenceHeight: 100,
    })
    await waitFor(() => expect(renderOcrHtmlMock).toHaveBeenCalledTimes(2))

    second.resolve('<h1>nuevo</h1>')
    await waitFor(() => expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('nuevo'))

    first.resolve('<h1>viejo</h1>')
    await Promise.resolve()
    expect(screen.queryByText('viejo')).not.toBeInTheDocument()
  })
})

Run before implementation:

```powershell
pnpm --filter @entropia-pro/desktop test -- src/components/OcrRichText.test.ts
```

Expected: FAIL because the component does not exist.

- [ ] **Step 2: Implement the Svelte 5 component with render-generation cancellation**

Use the existing runes style:

```svelte
<script lang="ts">
  import { renderOcrHtml, type OcrRenderContext } from '$lib/ocr-rich-text'

  interface OcrRichTextProps {
    text: string
    assetUrl: string
    sourceType: 'image' | 'pdf'
    referenceWidth: number
    referenceHeight: number
  }

  let {
    text,
    assetUrl,
    sourceType,
    referenceWidth,
    referenceHeight,
  }: OcrRichTextProps = $props()

  let html = $state('')
  let renderGeneration = 0

  $effect(() => {
    const generation = ++renderGeneration
    const context: OcrRenderContext = {
      assetUrl,
      sourceType,
      referenceWidth,
      referenceHeight,
    }
    html = ''

    void renderOcrHtml(text, context)
      .then((nextHtml) => {
        if (generation === renderGeneration) html = nextHtml
      })
      .catch(() => {
        if (generation === renderGeneration) html = ''
      })
  })
</script>

<div class="ocr-rich-text" data-testid="ocr-rich-text">
  {@html html}
</div>
```

The component must not accept or emit edit callbacks. It is a display-only sink for the existing OCR string.

- [ ] **Step 3: Add semantic rich-content styling**

Add component-local styles that reuse existing theme tokens and ensure the parent’s old `white-space: pre-wrap` does not flatten the structure:

```css
.ocr-rich-text {
  min-width: 0;
  overflow-wrap: anywhere;
  line-height: 1.6;
}

.ocr-rich-text :global(h1),
.ocr-rich-text :global(h2),
.ocr-rich-text :global(h3),
.ocr-rich-text :global(h4),
.ocr-rich-text :global(h5),
.ocr-rich-text :global(h6) {
  margin: var(--space-4) 0 var(--space-2);
  color: var(--color-text-primary);
  line-height: 1.25;
}

.ocr-rich-text :global(p),
.ocr-rich-text :global(ul),
.ocr-rich-text :global(ol),
.ocr-rich-text :global(blockquote),
.ocr-rich-text :global(table),
.ocr-rich-text :global(pre) {
  margin: 0 0 var(--space-3);
}

.ocr-rich-text :global(ul),
.ocr-rich-text :global(ol) {
  padding-inline-start: var(--space-6);
}

.ocr-rich-text :global(table) {
  width: 100%;
  border-collapse: collapse;
  font-size: inherit;
}

.ocr-rich-text :global(th),
.ocr-rich-text :global(td) {
  padding: var(--space-2);
  border: 1px solid var(--border-subtle);
  text-align: start;
  vertical-align: top;
}

.ocr-rich-text :global(img) {
  display: inline-block;
  max-width: 100%;
  height: auto;
  vertical-align: middle;
  border-radius: var(--radius-xs);
}
```

Keep all source-asset crop work out of the DOM and out of persistent storage.

- [ ] **Step 4: Run component tests and commit**

```powershell
pnpm --filter @entropia-pro/desktop test -- src/components/OcrRichText.test.ts
```

Expected: PASS with only the newest resolved HTML visible after rerender.

```powershell
git add apps/desktop/src/components/OcrRichText.svelte apps/desktop/src/components/OcrRichText.test.ts
git commit -m "feat(desktop): add OCR rich content component"
```

---

### Task 4: Integrate only the left extracted-text pane

**Files:**
- Modify: `apps/desktop/src/views/ItemAssetPanel.svelte:1-17,210-251,346-362`
- Modify: `apps/desktop/src/views/ItemView.test.ts` (add one left-only rich OCR regression near the existing text-tab tests)

**Interfaces:**

`ItemAssetPanel.svelte` already receives `viewerSrc`, `viewerType`, `layoutReferenceWidth`, and `layoutReferenceHeight`, so no new `ItemView.svelte` prop or OCR state is required.

- [ ] **Step 1: Write the left-only regression test**

Add a test using the existing `createStore` fixture seam:

```ts
it('renders mixed OCR rich content only in the left extracted-text tab', async () => {
  const source = [
    '<div align="center">',
    '',
    '# Radiografía',
    '',
    '</div>',
    '',
    '**Texto**',
    '',
    '| A | B |',
    '| --- | --- |',
    '| 1 | 2 |',
    '',
    '![](page=0,bbox=[10,20,30,40])',
  ].join('\n')

  storeRef.current = createStore({
    assetsRows: [{
      id: 'asset-image-1',
      itemId: 'item-1',
      path: 'docs/source.jpg',
      type: 'image',
      createdAt: 1,
    }],
    extractionsByAsset: { 'asset-image-1': { textContent: source, method: 'glm_ocr' } },
  })

  render(ItemView, { itemId: 'item-1', collectionId: 'col-1' })
  await screen.findByTestId('mock-document-viewer')
  await fireEvent.click(screen.getByRole('tab', { name: /texto extraído/i }))

  const leftPane = screen.getByRole('tabpanel', { name: /texto extraído/i })
  expect(within(leftPane).getByRole('heading', { level: 1, name: 'Radiografía' })).toBeInTheDocument()
  expect(within(leftPane).getByText('Texto')).toBeInTheDocument()
  expect(within(leftPane).getByRole('table')).toBeInTheDocument()
  expect(within(leftPane).queryByText('# Radiografía')).not.toBeInTheDocument()
  expect(within(leftPane).getByText(/page=0,bbox=\[10,20,30,40\]/)).toBeInTheDocument()

  expect(screen.getByTestId('mock-document-viewer')).toHaveAttribute('data-path', 'docs/source.jpg')
  await fireEvent.click(screen.getByRole('tab', { name: /^Texto$/i }))
  expect(screen.getByDisplayValue(source)).toBeInTheDocument()
  expect(storeRef.current.extractions.findByAsset).toHaveBeenCalledWith('asset-image-1')
})
```

Run the focused regression before integration:

```powershell
pnpm --filter @entropia-pro/desktop test -- src/views/ItemView.test.ts -t "renders mixed OCR rich content only"
```

Expected: FAIL because the left pane still renders literal source text.

- [ ] **Step 2: Replace only the OCR interpolation**

In `ItemAssetPanel.svelte`, import the new component:

```ts
import OcrRichText from '../components/OcrRichText.svelte'
```

Replace only the non-audio OCR body:

```svelte
<div class="left-text-panel-body">
  <OcrRichText
    text={ocrEditedText}
    assetUrl={viewerSrc}
    sourceType={viewerType === 'pdf' ? 'pdf' : 'image'}
    referenceWidth={layoutReferenceWidth}
    referenceHeight={layoutReferenceHeight}
  />
</div>
```

Keep the transcription branch unchanged except for adding a plain-text modifier class to preserve its existing `white-space: pre-wrap` behavior:

```svelte
<div class="left-text-panel-body left-text-panel-body--plain">
  {transcriptionEditedText}
</div>
```

Do not touch the `DocumentViewer` block, tab state, OCR callbacks, or any props passed to the right panel.

- [ ] **Step 3: Scope the body whitespace rule and run the regression**

Move `white-space: pre-wrap` from `.left-text-panel-body` to `.left-text-panel-body--plain` in `ItemAssetPanel.svelte`:

```css
.left-text-panel-body--plain {
  white-space: pre-wrap;
}
```

Run:

```powershell
pnpm --filter @entropia-pro/desktop test -- src/views/ItemView.test.ts -t "renders mixed OCR rich content only"
```

Expected: PASS; the left tab contains semantic heading/strong/table elements, the failed crop reference remains visible as text, the DocumentViewer mock path is unchanged, and the right text control still contains the exact original source.

- [ ] **Step 4: Run the affected desktop tests**

```powershell
pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-rich-text.test.ts src/components/OcrRichText.test.ts src/views/ItemView.test.ts
```

Expected: PASS for the new parser, crop geometry, component lifecycle, and left-only integration tests; existing ItemView tests remain green.

- [ ] **Step 5: Commit the scoped integration**

```powershell
git add apps/desktop/src/views/ItemAssetPanel.svelte apps/desktop/src/views/ItemView.test.ts
git commit -m "feat(desktop): render OCR text as rich content"
```

---

### Task 5: Run final variant checks and verify the actual surface

**Files:**
- No source changes expected; only fix failures in the files listed above.

- [ ] **Step 1: Reinstall from the updated lockfile**

```powershell
pnpm install --frozen-lockfile
```

Expected: install completes without lockfile drift.

- [ ] **Step 2: Run desktop lint and Lite typecheck**

```powershell
pnpm --filter @entropia-pro/desktop lint
$env:VITE_LOCAL_ML='0'; pnpm --filter @entropia-pro/desktop typecheck
$env:VITE_LOCAL_ML='1'
```

Expected: ESLint and Svelte/TypeScript checks pass. Restore the PowerShell variable after the Lite check so later commands do not inherit the wrong variant.

- [ ] **Step 3: Run the complete desktop test script once**

```powershell
pnpm --filter @entropia-pro/desktop test
```

Expected: all desktop Vitest projects pass, including existing DocumentViewer, OCR state, ItemView, and right-panel tests.

- [ ] **Step 4: Exercise the main viewer surface**

Launch the existing desktop development path from `apps/desktop` with Lite settings, open an image or scanned-page asset containing:

```text
<div align="center">

# Encabezado

</div>

**negrita** y *cursiva*

- uno
- dos

| Columna | Valor |
| --- | --- |
| A | B |

![](page=0,bbox=[536,508,1507,1112])
```

Select the left `Texto extraído` tab and verify visually that headings, inline formatting, list, table, centered block, and inline crop render as content. Select `Documento` and confirm the original viewer still renders. Select the right-panel `Texto` section and confirm the original source syntax remains in the editable field.

If the standalone Vite preview lacks Tauri APIs, record that limitation and rely on the focused component/ItemView behavioral tests for the unavailable backend path; do not change production code to accommodate the preview.

- [ ] **Step 5: Confirm the source string is untouched and capture final evidence**

Use the existing extraction fixture/store test seam to assert the exact source string remains the value returned by `findByAsset` after the rich renderer mounts. Review the final diff for only:

```text
apps/desktop/package.json
pnpm-lock.yaml
apps/desktop/src/lib/ocr-rich-text.ts
apps/desktop/src/lib/ocr-rich-text.test.ts
apps/desktop/src/components/OcrRichText.svelte
apps/desktop/src/components/OcrRichText.test.ts
apps/desktop/src/views/ItemAssetPanel.svelte
apps/desktop/src/views/ItemView.test.ts
```

Do not claim completion until the focused tests, lint, Lite typecheck, full desktop tests, and the available UI smoke evidence are recorded.
