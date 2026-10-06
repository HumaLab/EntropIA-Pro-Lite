import { invoke } from '@tauri-apps/api/core'
import type { Node } from './export-document'
import type { ExportPreferences } from './export-preferences'
import { citationsForFormat } from './export-preferences'
import type { StyleSource } from './writing-csl'
import { exportDocument, isExportFailure } from './writing-export'

/**
 * Sharing a manuscript with another account and sending it to the hlab.com.ar
 * blog (T-33). Sharing is the sync server's (EntropIA-Cloud "Documentos
 * compartidos de Escritura"); publishing is the site's
 * `PUT /api/escritura/posts/{documento}`, reached through the backend so the
 * key never leaves the system credential store.
 */

export interface WritingShare {
  document_id: string
  owner_email: string
  members: string[]
  is_owner: boolean
}

export interface PublishedPost {
  url: string
  admin_url: string
  is_active: boolean
  created: boolean
}

/** `null` when the server predates sharing: the Share section stays hidden. */
export function listWritingShares(): Promise<WritingShare[] | null> {
  return invoke<WritingShare[] | null>('sync_writing_shares')
}

export function shareWriting(documentId: string, email: string): Promise<WritingShare> {
  return invoke<WritingShare>('sync_writing_share', { documentId, email })
}

export function unshareWriting(documentId: string, email: string): Promise<void> {
  return invoke('sync_writing_unshare', { documentId, email })
}

/**
 * The article body as the site stores it: the same HTML as "Descargar HTML",
 * without the page around it and without the title heading (the site prints
 * the title itself). A citation kept as a comment has no place in a blog post,
 * so it goes out as a footnote.
 */
export async function articleHtml(
  doc: Node,
  preferences: ExportPreferences,
  style: StyleSource,
  bibliographyHeading: string
): Promise<string> {
  const citations =
    preferences.citations === 'comment'
      ? 'footnote'
      : citationsForFormat(preferences.citations, 'html')
  const result = await exportDocument(doc, {
    format: 'html',
    citations,
    bibliography: preferences.bibliography,
    style,
    title: '',
    bibliographyHeading,
  })
  if (isExportFailure(result)) throw new Error(result.elements.join(', '))
  return bodyOf(new TextDecoder().decode(result.bytes))
}

export function bodyOf(page: string): string {
  const body = /<body>\n?([\s\S]*?)\n?<\/body>/.exec(page)?.[1] ?? page
  return body.trim()
}

export function publishToHlab(
  documentId: string,
  title: string,
  html: string
): Promise<PublishedPost> {
  return invoke<PublishedPost>('writing_publish_hlab', { documentId, title, html })
}
