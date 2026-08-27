# Blank OCR PDF Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make OCR PDF downloads render the prepared text and images instead of producing a blank one-page PDF.

**Architecture:** Keep the existing `html2pdf.js` adapter and prepared-document pipeline. Stage a normal-flow export target inside an off-screen wrapper, then pass only the normal-flow child to `html2pdf` so cloned inline styles cannot displace the captured content.

**Tech Stack:** TypeScript, Vitest 3, happy-dom, Vite 6, `html2pdf.js` 0.14, Chromium.

## Global Constraints

- Preserve the public OCR export API and the current Markdown and DOCX adapters.
- Preserve the prepared HTML, PDF options, save dialog, filenames, and OCR-region resolution.
- Remove the complete temporary DOM subtree after both successful and failed PDF generation.
- Write the regression test first and observe the expected failure before editing production code.
- Do not add dependencies or change PDF styling.

---

### Task 1: Keep the html2pdf render target on-page in its clone

**Files:**
- Modify: `apps/desktop/src/lib/ocr-export.test.ts:290-304`
- Modify: `apps/desktop/src/lib/ocr-export.ts:121-147`

**Interfaces:**
- Consumes: `generateOcrExportBytes(format: OcrExportFormat, document: PreparedOcrExport, generators?: Partial<OcrExportGenerators>): Promise<Uint8Array>` and the existing mocked `Html2PdfWorker.from(element: HTMLElement)` chain.
- Produces: unchanged export APIs; the element supplied to `html2pdf().from(...)` has normal positioning while its parent stages it off-screen.

- [ ] **Step 1: Add the failing regression test**

Add this test inside the existing `describe('OCR export adapters', ...)` block, before the temporary-DOM cleanup tests:

```typescript
it('keeps the html2pdf render target in normal flow while staging it off-screen', async () => {
  const { generateOcrExportBytes } = await loadOcrExport()
  let renderTarget: HTMLElement | null = null
  let stagingWrapper: HTMLElement | null = null

  html2pdfWorker.from.mockImplementationOnce((element: HTMLElement) => {
    renderTarget = element
    return html2pdfWorker
  })
  html2pdfWorker.outputPdf.mockImplementationOnce(async () => {
    if (!renderTarget) throw new Error('html2pdf render target was not provided')

    stagingWrapper = renderTarget.parentElement
    expect(renderTarget.style.position).toBe('')
    expect(renderTarget.style.insetInlineStart).toBe('')
    expect(renderTarget.style.width).toBe('180mm')
    expect(stagingWrapper?.style.position).toBe('fixed')
    expect(stagingWrapper?.style.insetInlineStart).toBe('-100000px')
    expect(document.body.contains(stagingWrapper)).toBe(true)

    return Uint8Array.from([9, 8]).buffer
  })

  await expect(generateOcrExportBytes('pdf', prepared)).resolves.toEqual(Uint8Array.from([9, 8]))
  expect(stagingWrapper).not.toBeNull()
  expect(document.body.contains(stagingWrapper)).toBe(false)
})
```

- [ ] **Step 2: Run the focused test and verify RED**

Run from the repository root:

```bash
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-export.test.ts
```

Expected: the new test fails because the current element passed to `html2pdfWorker.from` has `style.position === 'fixed'` instead of `''`.

- [ ] **Step 3: Move off-screen positioning to a staging wrapper**

Replace `generatePdfBytes` with:

```typescript
async function generatePdfBytes(html: string): Promise<Uint8Array> {
  const { default: html2pdf } = await import('html2pdf.js')
  const stagingWrapper = document.createElement('div')
  stagingWrapper.style.position = 'fixed'
  stagingWrapper.style.insetInlineStart = '-100000px'
  stagingWrapper.style.insetBlockStart = '0'

  const renderTarget = document.createElement('div')
  renderTarget.style.width = '180mm'
  renderTarget.innerHTML = html
  stagingWrapper.append(renderTarget)
  document.body.append(stagingWrapper)

  try {
    const output = await html2pdf()
      .set({
        margin: [12, 12, 12, 12],
        image: { type: 'png', quality: 1 },
        pagebreak: { mode: ['css', 'legacy'], avoid: ['img', 'tr', 'pre'] },
        html2canvas: { scale: 2, useCORS: false, backgroundColor: '#ffffff' },
        jsPDF: { unit: 'mm', format: 'a4', orientation: 'portrait', compressPDF: true },
      } as never)
      .from(renderTarget)
      .outputPdf('arraybuffer')

    return new Uint8Array(output)
  } finally {
    stagingWrapper.remove()
  }
}
```

- [ ] **Step 4: Run the focused test and verify GREEN**

Run:

```bash
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop test -- src/lib/ocr-export.test.ts
```

Expected: `13 tests` pass in `src/lib/ocr-export.test.ts`, including success/failure cleanup coverage.

- [ ] **Step 5: Run the Lite frontend typecheck**

Run:

```bash
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck
```

Expected: `svelte-check found 0 errors and 0 warnings`.

- [ ] **Step 6: Verify the real PDF rendering path in Chromium**

Start the actual desktop Vite surface:

```bash
VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop dev -- --host 127.0.0.1
```

In Chromium at `http://localhost:1420/`, import `/src/lib/ocr-export.ts` and call:

```typescript
const { generateOcrExportBytes } = await import('/src/lib/ocr-export.ts')
const bytes = await generateOcrExportBytes('pdf', {
  markdown: '# Radiografía\nTexto visible',
  html: '<h1>Radiografía</h1><p>Texto visible generado por OCR.</p>',
})
const url = URL.createObjectURL(new Blob([bytes], { type: 'application/pdf' }))
location.href = url
```

Expected: Chromium renders one PDF page containing `Radiografía` and `Texto visible generado por OCR.`; the file is no longer the reproducible 3,058-byte blank artifact.

- [ ] **Step 7: Commit the verified work unit**

```bash
git add apps/desktop/src/lib/ocr-export.ts apps/desktop/src/lib/ocr-export.test.ts
git commit -m "fix(desktop): render OCR PDF content on page"
```

Rollback boundary: reverting this commit restores only the PDF staging behavior and its regression test; Markdown, DOCX, OCR parsing, persistence, and the approved design/plan documents remain intact.
