const LONG_CELL_THRESHOLD = 120

export interface DbBrowserCellContent {
  /** What the cell shows, and its tooltip. */
  rawText: string
  /** What the copy action writes; differs from `rawText` only for a BLOB. */
  copyText: string
  expandedText: string
  isJson: boolean
  canExpand: boolean
  hasValue: boolean
}

/** True for a column whose declared type gives it BLOB affinity. */
export function isDbBrowserBlobColumn(dataType: string): boolean {
  return /BLOB/i.test(dataType)
}

/**
 * The backend sends BLOB values as padded standard Base64. `blobSummary` is
 * passed only for BLOB columns: the cell then shows the decoded size instead
 * of kilobytes of Base64, while copy and expand keep the full payload.
 */
export function getDbBrowserCellContent(
  value: unknown,
  emptyPlaceholder: string,
  blobSummary?: (bytes: number) => string
): DbBrowserCellContent {
  if (value == null) {
    return {
      rawText: emptyPlaceholder,
      copyText: emptyPlaceholder,
      expandedText: emptyPlaceholder,
      isJson: false,
      canExpand: false,
      hasValue: false,
    }
  }

  if (typeof value === 'string' && blobSummary) {
    return {
      rawText: blobSummary(base64ByteLength(value)),
      copyText: value,
      expandedText: value,
      isJson: false,
      canExpand: true,
      hasValue: true,
    }
  }

  if (typeof value === 'string') {
    const parsedJson = parseJsonString(value)
    if (parsedJson) {
      return {
        rawText: value,
        copyText: value,
        expandedText: JSON.stringify(parsedJson, null, 2),
        isJson: true,
        canExpand: true,
        hasValue: true,
      }
    }

    return {
      rawText: value,
      copyText: value,
      expandedText: value,
      isJson: false,
      canExpand: value.length > LONG_CELL_THRESHOLD,
      hasValue: true,
    }
  }

  if (typeof value === 'number' || typeof value === 'boolean' || typeof value === 'bigint') {
    const text = String(value)
    return {
      rawText: text,
      copyText: text,
      expandedText: text,
      isJson: false,
      canExpand: false,
      hasValue: true,
    }
  }

  try {
    const rawText = JSON.stringify(value)
    const expandedText = JSON.stringify(value, null, 2)

    return {
      rawText,
      copyText: rawText,
      expandedText,
      isJson: true,
      canExpand: true,
      hasValue: true,
    }
  } catch {
    const fallback = String(value)
    return {
      rawText: fallback,
      copyText: fallback,
      expandedText: fallback,
      isJson: false,
      canExpand: fallback.length > LONG_CELL_THRESHOLD,
      hasValue: true,
    }
  }
}

function base64ByteLength(base64: string): number {
  const padding = base64.endsWith('==') ? 2 : base64.endsWith('=') ? 1 : 0
  return Math.max(0, Math.floor((base64.length * 3) / 4) - padding)
}

function parseJsonString(value: string): unknown | null {
  const trimmed = value.trim()
  if (!trimmed) return null

  const looksLikeJson =
    (trimmed.startsWith('{') && trimmed.endsWith('}')) ||
    (trimmed.startsWith('[') && trimmed.endsWith(']'))

  if (!looksLikeJson) return null

  try {
    return JSON.parse(trimmed)
  } catch {
    return null
  }
}
