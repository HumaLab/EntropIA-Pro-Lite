import { invoke } from '@tauri-apps/api/core'

/**
 * The Biblioteca read adapter (P2): the works of the synced Zotero
 * libraries, one work's ficha, and one cataloged attachment prepared for
 * the in-app viewer. Every function is a thin wrapper over a read-only
 * backend command — nothing here writes, opens or launches anything.
 */

/** One work row as the Biblioteca list shows it. */
export interface BibliographyWorkRow {
  /**
   * The catalog's stable internal id (`bibliographic_items.id`): what the
   * work view is opened by. Minted once and preserved across upserts.
   */
  itemId: string
  /** The durable Zotero native key of the work. */
  itemKey: string
  /** Internal `zotero_libraries.id`, the scope this row belongs to. */
  libraryRowId: string
  title: string
  /** CSL family names (or literal names), comma-separated; empty when none. */
  authors: string
  year: number | null
  libraryName: string
  /** `user` or `group` plus the native id: the identity Zotero uses. */
  libraryType: string
  libraryNativeId: string
  /** The catalog's last CSL-JSON, the offline fallback of the ficha. */
  cslJson: string
}

/** The order the Biblioteca lists works in. */
export type BibliotecaSort = 'title' | 'recent'

/** Stable key for the remembered listing order. */
export const BIBLIOTECA_SORT_STORAGE_KEY = 'entropia:biblioteca:sort'

/**
 * Reads a remembered order out of a raw stored value. Anything may be in
 * localStorage — an older build, a hand edit — so anything unrecognised is
 * the default title order rather than a failure.
 */
export function parseBibliotecaSort(raw: string | null): BibliotecaSort {
  return raw === 'recent' ? 'recent' : 'title'
}

/** The remembered order, or the title order when storage is unusable. */
export function readBibliotecaSort(): BibliotecaSort {
  try {
    return parseBibliotecaSort(localStorage.getItem(BIBLIOTECA_SORT_STORAGE_KEY))
  } catch {
    // Storage can be unavailable outright: the title order still lists.
    return 'title'
  }
}

/** Remembers the listing order. Best effort, like every localStorage write. */
export function writeBibliotecaSort(sort: BibliotecaSort): void {
  try {
    localStorage.setItem(BIBLIOTECA_SORT_STORAGE_KEY, sort)
  } catch {
    // Unavailable storage forgets the choice for this session only.
  }
}

export interface BibliographyWorkList {
  works: BibliographyWorkRow[]
  /** How many works the scope holds in total (paging, not a capped count). */
  total: number
}

/**
 * One page of the catalog's works in the requested order (title by default,
 * tombstones excluded) with the scope's total. The optional library is named
 * the way Zotero names it (`user`/`group` + native id) and is resolved by the
 * backend.
 */
export function bibliographyListWorks(options: {
  library?: { libraryType: 'user' | 'group'; libraryId: string } | null
  offset?: number
  limit?: number
  /** Substring filter over title and creators; meaning-level search is `bibliographySearchWorks`. */
  query?: string
  /** `title` (the default) or `recent` — Zotero's item_version, newest first. */
  sort?: BibliotecaSort
}): Promise<BibliographyWorkList> {
  return invoke<BibliographyWorkList>('bibliography_list_works', {
    request: {
      offset: options.offset ?? 0,
      limit: options.limit ?? 50,
      query: options.query ?? null,
      sort: options.sort ?? 'title',
      zoteroLibraryType: options.library?.libraryType ?? null,
      zoteroLibraryId: options.library?.libraryId ?? null,
    },
  })
}

/** One Zotero creator of a work, camelCase as the detail answers. */
export interface BibliographyWorkCreator {
  creatorType?: string | null
  firstName?: string | null
  lastName?: string | null
  name?: string | null
}

/** Attachment metadata only: names to list, never paths to open. */
export interface BibliographyWorkAttachmentRef {
  attachmentKey: string
  contentType?: string | null
  linkMode?: string | null
  filename?: string | null
  url?: string | null
}

/** The catalog projection of one work item. */
export interface BibliographyWorkDetailItem {
  itemKey: string
  itemType: string | null
  title: string | null
  creators: BibliographyWorkCreator[] | null
  publicationTitle: string | null
  publisher: string | null
  date: string | null
  doi: string | null
  isbn: string | null
  /** `abstract` on the wire: the one reserved-looking name kept verbatim. */
  abstract: string | null
  language: string | null
  url: string | null
  itemVersion: number | null
  collections: string[]
  tags: string[]
  attachments: BibliographyWorkAttachmentRef[]
}

/** One work's ficha: display line plus the catalog projection. */
export interface BibliographyWorkDetail {
  itemId: string
  itemKey: string
  title: string
  authors: string
  year: number | null
  libraryName: string
  libraryType: string
  libraryNativeId: string
  cslJson: string
  item: BibliographyWorkDetailItem
}

/** Reads one work by its catalog item id. */
export function bibliographyWorkDetail(itemId: string): Promise<BibliographyWorkDetail> {
  return invoke<BibliographyWorkDetail>('bibliography_work_detail', { itemId })
}

/** One extracted page text of a work attachment. */
export interface BibliographyWorkPageText {
  pageNumber: number
  /** `native` or `ocr`. */
  method: string
  /** `rich`, `sparse`, `empty` or `unreadable`. */
  quality: string
  text: string
}

export interface BibliographyWorkOpen {
  itemId: string
  itemKey: string
  title: string
  attachmentKey: string
  /** What the app can show: a PDF file or an HTML snapshot's stored text. */
  originalKind: 'pdf' | 'html' | null
  /**
   * The PDF the viewer was just granted (one file, at runtime). `null` for
   * HTML snapshots and whenever nothing could be granted.
   */
  originalPath: string | null
  /** Why the original cannot be shown (`reason: detail`), when it cannot. */
  openError: string | null
  /** The per-page extracted texts, in page order; empty when none. */
  pages: BibliographyWorkPageText[]
  /** The whole-document extraction (HTML snapshots render from it). */
  snapshotText: string
  /** False when nothing was extracted yet: the "Texto" tab stays pending. */
  extracted: boolean
}

/**
 * Prepares one cataloged attachment of one work for the in-app viewer: the
 * backend resolves the registered attachment, validates the PDF and allows
 * that one file on the asset protocol. The extracted texts travel with the
 * answer even when no original is viewable. Nothing opens outside the app.
 */
export function bibliographyOpenWorkAttachment(
  itemId: string,
  attachmentKey: string
): Promise<BibliographyWorkOpen> {
  return invoke<BibliographyWorkOpen>('bibliography_open_work_attachment', {
    itemId,
    attachmentKey,
  })
}
