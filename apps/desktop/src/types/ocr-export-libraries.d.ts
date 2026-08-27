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
