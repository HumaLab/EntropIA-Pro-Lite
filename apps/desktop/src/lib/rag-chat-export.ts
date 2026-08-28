import { downloadDir, join } from '@tauri-apps/api/path'
import { writeFile } from '@tauri-apps/plugin-fs'

import { renderMarkdown } from './markdown'
import { generateNativeOcrPdfBytes } from './ocr-pdf'
import { ragGetConversation, type RagConversation } from './rag'

const EXPORT_CLASS = 'ocr-export-document'
const MAX_FILENAME_STEM_LENGTH = 80

function escapeHtml(value: string): string {
  return value
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;')
}

function questionHtml(content: string): string {
  const normalized = content.replace(/\r\n?/g, '\n')
  return `<p>${escapeHtml(normalized).replaceAll('\n', '<br>')}</p>`
}

function messageHtml(role: 'user' | 'assistant', content: string): string {
  const roleLabel = role === 'user' ? 'Question' : 'Answer'
  const body = role === 'assistant' ? renderMarkdown(content) : questionHtml(content)
  return `<h2>${roleLabel}</h2>${body || '<p></p>'}`
}

function filenameStem(title: string): string {
  const cleaned = title
    .normalize('NFKC')
    .replace(/[<>:"/\\|?*\p{Cc}]/gu, '')
    .replace(/\s+/g, ' ')
    .trim()
    .slice(0, MAX_FILENAME_STEM_LENGTH)
  return cleaned || 'conversation'
}

function filenameIdSuffix(id: string): string {
  const cleaned = id.slice(0, 8).replace(/[^A-Za-z0-9_-]/g, '')
  return cleaned || 'conversation'
}

function conversationFilename(conversation: RagConversation): string {
  return `${filenameStem(conversation.title)} - ${filenameIdSuffix(conversation.id)}.pdf`
}

export function buildRagConversationPdfHtml(conversation: RagConversation): string {
  const messages = conversation.messages
    .map((message) => messageHtml(message.role, message.content))
    .join('')

  return `<!doctype html><html><head><meta charset="utf-8"></head><body><div class="${EXPORT_CLASS}"><h1>${escapeHtml(conversation.title)}</h1>${messages}</div></body></html>`
}

export async function downloadRagConversationPdf(conversationId: string): Promise<string> {
  const conversation = await ragGetConversation(conversationId)
  const bytes = await generateNativeOcrPdfBytes(buildRagConversationPdfHtml(conversation))
  const directory = await downloadDir()
  const path = await join(directory, conversationFilename(conversation))
  await writeFile(path, bytes)
  return path
}
