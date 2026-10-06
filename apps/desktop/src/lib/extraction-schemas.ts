import { invoke } from '@tauri-apps/api/core'

/**
 * Extraction with a user-defined schema (T-51): the user names the fields
 * once and a batch fills one record per case the text describes. The schema
 * runs as a batch operation `schema:<id>`; its records are read back here.
 */

export interface SchemaField {
  name: string
  description: string
  /** One ship, many cargoes: the field holds a list. */
  repeatable: boolean
}

export interface ExtractionSchema {
  /** Empty for a schema not saved yet. */
  id: string
  name: string
  fields: SchemaField[]
  /** OpenRouter model for this schema (T-26); '' uses the general one. */
  model: string
}

export interface ExtractionRecordRow {
  item_id: string
  item_title: string
  asset_id: string
  record: Record<string, string | string[] | null>
}

export function listSchemas(): Promise<ExtractionSchema[]> {
  return invoke<ExtractionSchema[]>('extraction_schemas_list')
}

export function saveSchema(schema: ExtractionSchema): Promise<ExtractionSchema> {
  return invoke<ExtractionSchema>('extraction_schema_save', { schema })
}

export function deleteSchema(schemaId: string): Promise<void> {
  return invoke('extraction_schema_delete', { schemaId })
}

export function listRecords(schemaId: string): Promise<ExtractionRecordRow[]> {
  return invoke<ExtractionRecordRow[]>('extraction_records_list', { schemaId })
}

/** The batch operation that runs `schemaId`. */
export function schemaOperation(schemaId: string): string {
  return `schema:${schemaId}`
}

/** A list cell joins its values with "; ", the way a spreadsheet reads them. */
export function cellText(value: string | string[] | null | undefined): string {
  if (Array.isArray(value)) return value.join('; ')
  return value ?? ''
}

/**
 * One row per record, the document title first. Quoted per RFC 4180 and
 * opened with a BOM so a spreadsheet reads the accents.
 */
export function recordsCsv(
  schema: ExtractionSchema,
  rows: ExtractionRecordRow[],
  documentHeading: string
): string {
  const quote = (text: string) => `"${text.replaceAll('"', '""')}"`
  const header = [documentHeading, ...schema.fields.map((field) => field.name)]
  const lines = rows.map((row) =>
    [row.item_title, ...schema.fields.map((field) => cellText(row.record[field.name]))]
      .map(quote)
      .join(',')
  )
  return '﻿' + [header.map(quote).join(','), ...lines].join('\r\n') + '\r\n'
}
