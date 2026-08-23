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

declare module 'html-docx-js/dist/html-docx.js' {
  export function asBlob(html: string, options?: Record<string, unknown>): Blob
}

declare module 'html-docx-js/dist/html-docx.js?url' {
  const url: string
  export default url
}

declare global {
  interface Window {
    htmlDocx?: {
      asBlob(html: string, options?: Record<string, unknown>): Blob
    }
  }
}

export {}
