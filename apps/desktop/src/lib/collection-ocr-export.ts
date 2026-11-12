import { save } from '@tauri-apps/plugin-dialog'
import { writeFile } from '@tauri-apps/plugin-fs'
import type { StoreApi } from '@entropia/store'

import { t } from './i18n'
import { getAssetPathLabel } from './item-metadata'
import { buildPrintableHtml, generateDocxBytes, type OcrExportGenerators } from './ocr-export'
import { generateNativeOcrPdfBytes } from './ocr-pdf'
import {
  escapeHtml,
  renderOcrMarkup,
  replaceOcrRegionPlaceholders,
  replaceOcrRegionReferences,
  sanitizeOcrHtml,
  type OcrRegionReference,
} from './ocr-rich-text'

/**
 * Exports the extracted text of a whole collection as one file: every
 * document's text (OCR, or native PDF text when that is all it has) in
 * collection order, under a heading naming its item and file.
 *
 * Embedded OCR image regions are not cropped here (that needs each viewer's
 * page geometry); they become a short textual placeholder instead.
 */

export type CollectionOcrExportFormat = 'txt' | 'markdown' | 'csv' | 'pdf' | 'docx'

export interface CollectionOcrDocument {
  itemTitle: string
  fileName: string
  /** True when the text is the PDF's native text layer rather than OCR. */
  native: boolean
  source: string
}

/** Thrown when the collection has no extracted text to export. */
export class CollectionOcrExportEmptyError extends Error {
  constructor() {
    super('The collection has no extracted text to export')
    this.name = 'CollectionOcrExportEmptyError'
  }
}

const EXPORT_OPTIONS: Record<CollectionOcrExportFormat, { name: string; extension: string }> = {
  txt: { name: 'Text', extension: 'txt' },
  markdown: { name: 'Markdown', extension: 'md' },
  csv: { name: 'CSV', extension: 'csv' },
  pdf: { name: 'PDF', extension: 'pdf' },
  docx: { name: 'Microsoft Word', extension: 'docx' },
}

const TXT_RULE = '='.repeat(50)

export async function loadCollectionOcrDocuments(
  store: StoreApi,
  collectionId: string
): Promise<CollectionOcrDocument[]> {
  const items = await store.items.findByCollection(collectionId)
  const documents: CollectionOcrDocument[] = []

  for (const item of items) {
    const assets = await store.assets.findByItem(item.id)
    for (const asset of assets) {
      if (asset.type === 'audio') continue
      const extraction = await store.extractions.findByAsset(asset.id)
      const source = extraction?.textContent.replace(/\r\n?/g, '\n') ?? ''
      if (!source.trim()) continue
      documents.push({
        itemTitle: item.title,
        fileName: getAssetPathLabel(asset.path),
        native: extraction?.method === 'native',
        source,
      })
    }
  }

  return documents
}

function regionPlaceholder(reference: OcrRegionReference): string {
  return t('collection.ocrExport.imagePlaceholder', { page: reference.page + 1 })
}

function plainSource(document: CollectionOcrDocument): string {
  return replaceOcrRegionReferences(document.source, regionPlaceholder)
}

function fileLabel(document: CollectionOcrDocument): string {
  return document.native
    ? `${document.fileName} ${t('collection.ocrExport.nativeSuffix')}`
    : document.fileName
}

function buildMarkdown(collectionName: string, documents: CollectionOcrDocument[]): string {
  const sections = documents.map((document) =>
    [
      `## ${document.itemTitle}`,
      '',
      `*${fileLabel(document)}*`,
      '',
      plainSource(document),
      '',
    ].join('\n')
  )
  return [`# ${collectionName}`, '', ...sections].join('\n')
}

function buildText(collectionName: string, documents: CollectionOcrDocument[]): string {
  const sections = documents.map((document) =>
    [
      TXT_RULE,
      `${document.itemTitle} · ${fileLabel(document)}`,
      TXT_RULE,
      '',
      plainSource(document),
      '',
    ].join('\n')
  )
  return [collectionName, '', ...sections].join('\n')
}

function csvCell(value: string): string {
  return /[;"\r\n]/.test(value) ? `"${value.replaceAll('"', '""')}"` : value
}

function buildCsv(documents: CollectionOcrDocument[]): string {
  const header = [
    t('collection.ocrExport.csvDocument'),
    t('collection.ocrExport.csvFile'),
    t('collection.ocrExport.csvOrigin'),
    t('collection.ocrExport.csvText'),
  ]
  const rows = documents.map((document) => [
    document.itemTitle,
    document.fileName,
    document.native ? t('collection.ocrExport.originNative') : t('collection.ocrExport.originOcr'),
    plainSource(document),
  ])
  const records = [header, ...rows].map((row) => row.map(csvCell).join(';'))
  return `\uFEFF${records.join('\r\n')}\r\n`
}

function documentHtml(document: CollectionOcrDocument): string {
  const markup = renderOcrMarkup(document.source)
  const body = sanitizeOcrHtml(
    replaceOcrRegionPlaceholders(
      markup.html,
      new Map(
        markup.references.map((reference) => [
          reference.token,
          `<span>${escapeHtml(regionPlaceholder(reference))}</span>`,
        ])
      )
    )
  )
  return `<h2>${escapeHtml(document.itemTitle)}</h2><p><em>${escapeHtml(fileLabel(document))}</em></p>${body}`
}

function buildHtml(collectionName: string, documents: CollectionOcrDocument[]): string {
  return buildPrintableHtml(
    `<h1>${escapeHtml(collectionName)}</h1>${documents.map(documentHtml).join('')}`
  )
}

export async function buildCollectionOcrExportBytes(
  collectionName: string,
  documents: CollectionOcrDocument[],
  format: CollectionOcrExportFormat,
  generators: Partial<OcrExportGenerators> = {}
): Promise<Uint8Array> {
  const encoder = new TextEncoder()
  switch (format) {
    case 'markdown':
      return encoder.encode(buildMarkdown(collectionName, documents))
    case 'txt':
      return encoder.encode(buildText(collectionName, documents))
    case 'csv':
      return encoder.encode(buildCsv(documents))
    case 'pdf':
      return (generators.pdf ?? generateNativeOcrPdfBytes)(buildHtml(collectionName, documents))
    case 'docx':
      return (generators.docx ?? generateDocxBytes)(buildHtml(collectionName, documents))
  }
}

function defaultFileName(collectionName: string, extension: string): string {
  const stem = collectionName.replace(/[\\/:*?"<>|]+/g, '-').trim() || 'coleccion'
  return `${stem}-texto.${extension}`
}

/**
 * Collects the collection's text, asks where to save it and writes the file.
 * Returns the written path, or null when the collection is unknown or the
 * dialog is cancelled; throws CollectionOcrExportEmptyError when no document
 * has text, before any dialog opens.
 */
export async function exportCollectionOcrText(
  store: StoreApi,
  collectionId: string,
  format: CollectionOcrExportFormat,
  generators: Partial<OcrExportGenerators> = {}
): Promise<string | null> {
  const collection = await store.collections.findById(collectionId)
  if (!collection) return null

  const documents = await loadCollectionOcrDocuments(store, collectionId)
  if (documents.length === 0) throw new CollectionOcrExportEmptyError()

  const option = EXPORT_OPTIONS[format]
  const filePath = await save({
    defaultPath: defaultFileName(collection.name, option.extension),
    filters: [{ name: option.name, extensions: [option.extension] }],
  })
  if (!filePath) return null

  const bytes = await buildCollectionOcrExportBytes(collection.name, documents, format, generators)
  await writeFile(filePath, bytes)
  return filePath
}
