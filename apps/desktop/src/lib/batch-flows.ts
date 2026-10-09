/**
 * Saved batch flows (T-52): a named set of batch operations — for example
 * "Partes de puerto" = OCR + Entidades + the ship/cargo schema — chosen with
 * one click instead of ticking the same boxes every time. The batch already
 * chains the steps (extraction waits on OCR); a flow only remembers the
 * choice. Stored as JSON in one app setting.
 */

export const BATCH_FLOWS_SETTING = 'batch_flows'

export interface BatchFlowSteps {
  ocr: boolean
  embeddings: boolean
  ner: boolean
  triples: boolean
  /** Custom schema id; '' runs none. */
  schemaId: string
}

export interface BatchFlow {
  name: string
  steps: BatchFlowSteps
}

/** Reads the stored list; anything malformed is dropped, never thrown. */
export function parseFlows(raw: string | null | undefined): BatchFlow[] {
  if (!raw) return []
  try {
    const value: unknown = JSON.parse(raw)
    if (!Array.isArray(value)) return []
    return value.flatMap((entry): BatchFlow[] => {
      const name = typeof entry?.name === 'string' ? entry.name.trim() : ''
      const steps = entry?.steps
      if (!name || typeof steps !== 'object' || steps === null) return []
      return [
        {
          name,
          steps: {
            ocr: steps.ocr === true,
            embeddings: steps.embeddings === true,
            ner: steps.ner === true,
            triples: steps.triples === true,
            schemaId: typeof steps.schemaId === 'string' ? steps.schemaId : '',
          },
        },
      ]
    })
  } catch {
    return []
  }
}

/** Adds the flow, replacing one with the same name (case-insensitive). */
export function upsertFlow(flows: BatchFlow[], flow: BatchFlow): BatchFlow[] {
  const key = flow.name.trim().toLowerCase()
  return [...flows.filter((existing) => existing.name.toLowerCase() !== key), flow].sort((a, b) =>
    a.name.localeCompare(b.name, 'es')
  )
}
