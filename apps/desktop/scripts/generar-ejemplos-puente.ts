/**
 * Genera los archivos de ejemplo del puente con la web, corriendo el código real
 * de exportación (`exportCollectionById`) — acá no se escribe JSON a mano.
 *
 * Los archivos quedan en `EJEMPLOS_PUENTE_DIR` (por defecto, una carpeta dentro
 * del temporal del sistema) y de ahí van al repositorio de la web, que los usa
 * como prueba fija del formato. Si cambia la forma del archivo, esa prueba se
 * pone en rojo: hay que volver a correr este guion y reemplazarlos, NO editarlos
 * a mano — editarlos a mano es justo lo que estos ejemplos existen para evitar.
 *
 * Cómo correrlo, desde `apps/desktop`:
 *
 *   npx vitest run --config scripts/vitest.ejemplos.config.ts
 *
 * Para elegir a dónde van:
 *
 *   EJEMPLOS_PUENTE_DIR=/ruta/que/quieras npx vitest run --config scripts/vitest.ejemplos.config.ts
 *
 * No corre en la batería de pruebas normal (`pnpm test`): vive fuera de `src/`,
 * que es lo único que mira `vitest.config.ts`.
 *
 * Qué garantiza y qué no: la FORMA del archivo es la de producción, porque sale
 * del mismo código que el botón Exportar. Los DATOS son de laboratorio (la base
 * real vive detrás de la app de escritorio y no se puede leer desde acá).
 *
 * El contrato que fija esto vive en el vault:
 * `03 - HLab/Puente EntropIA — contrato del archivo.md`.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { exportCollectionById, type ExportAiConfig } from '../src/lib/export'

const OUT_DIR = process.env.EJEMPLOS_PUENTE_DIR ?? join(tmpdir(), 'ejemplos-puente')

// ---------------------------------------------------------------------------
// Archivos binarios de verdad, chiquitos: estos ejemplos entran al repositorio
// de la web y no pueden pesar megas.
// ---------------------------------------------------------------------------

/** El ícono de la app: un PNG real de 1.335 bytes. */
const PNG_BYTES = new Uint8Array(readFileSync('src-tauri/icons/32x32.png'))

/** Un PDF de una página, válido y con su tabla de referencias bien calculada. */
function buildPdfBytes(): Uint8Array {
  const content = 'BT /F1 14 Tf 60 720 Td (Acta de inauguracion - Bristol, 1913) Tj ET'
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>',
    `<< /Length ${content.length} >>\nstream\n${content}\nendstream`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  ]

  let pdf = '%PDF-1.4\n'
  const offsets: number[] = []
  objects.forEach((body, index) => {
    offsets.push(pdf.length)
    pdf += `${index + 1} 0 obj\n${body}\nendobj\n`
  })

  const xrefOffset = pdf.length
  pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`
  for (const offset of offsets) {
    pdf += `${String(offset).padStart(10, '0')} 00000 n \n`
  }
  pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xrefOffset}\n%%EOF\n`

  return new Uint8Array(Buffer.from(pdf, 'latin1'))
}

const PDF_BYTES = buildPdfBytes()

/** Cabecera RIFF y nada más: el audio nunca cruza el puente, sólo su desgrabación. */
const WAV_BYTES = new Uint8Array([0x52, 0x49, 0x46, 0x46, 0x00, 0x00, 0x00, 0x00])

// ---------------------------------------------------------------------------
// Los datos de la colección de ejemplo
// ---------------------------------------------------------------------------

/** Llave de mentira, y que se note: nunca poner una real acá. */
const AI_CONFIG: ExportAiConfig = {
  provider: 'openrouter',
  apiKey: 'sk-or-v1-ejemplo-no-sirve',
  model: 'google/gemma-4-26b-a4b-it',
  baseUrl: 'https://openrouter.ai/api/v1',
  name: 'Llave de OpenRouter de EntropIA',
}

const COLLECTION = {
  id: 'col-bristol-1913',
  name: 'Archivo Bristol 1913-1938',
  description: 'Correspondencia y actas del Hotel Bristol',
  createdAt: 1754000000000,
  updatedAt: 1754009000000,
}

const ROOT = 'C:\\Users\\rf\\AppData\\Roaming\\entropia\\assets\\col-bristol-1913'

const ITEMS = [
  {
    id: 'item-carta',
    title: 'Carta al Intendente (1913)',
    collectionId: COLLECTION.id,
    metadata: '{"anio":1913,"fondo":"Intendencia","folio":"12v"}',
    createdAt: 1754000100000,
    updatedAt: 1754000900000,
  },
  {
    id: 'item-acta',
    title: 'Acta de inauguración (1913)',
    collectionId: COLLECTION.id,
    metadata: '{"anio":1913,"fondo":"Actas"}',
    createdAt: 1754000200000,
    updatedAt: 1754000910000,
  },
  {
    id: 'item-entrevista',
    title: 'Entrevista a la nieta del concesionario (1938)',
    collectionId: COLLECTION.id,
    metadata: '{"anio":1938,"soporte":"audio"}',
    createdAt: 1754000300000,
    updatedAt: 1754000920000,
  },
]

/**
 * Los assets de cada documento.
 *
 * `inflate` declara 40 MB por asset para que el reparto en tandas lo decida el
 * código de verdad. Los bytes que se escriben siguen siendo los chiquitos: el
 * tamaño sólo se usa para estimar el peso, nunca para armar el archivo.
 */
function assetsFor(itemId: string, inflate: boolean) {
  const big = 40 * 1024 * 1024
  if (itemId === 'item-carta') {
    return [
      {
        id: 'asset-carta-p1',
        itemId,
        path: `${ROOT}\\item-carta\\carta-1913-p1.png`,
        type: 'image',
        sortIndex: 0,
        size: inflate ? big : PNG_BYTES.length,
        createdAt: 1754000400000,
        parentAssetId: null,
      },
    ]
  }
  if (itemId === 'item-acta') {
    // PDF padre + su página: el padre viaja sin archivo, la página con archivo.
    return [
      {
        id: 'asset-acta-pdf',
        itemId,
        path: `${ROOT}\\item-acta\\acta-1913.pdf`,
        type: 'pdf',
        sortIndex: 0,
        size: PDF_BYTES.length,
        createdAt: 1754000500000,
        parentAssetId: null,
      },
      {
        id: 'asset-acta-p1',
        itemId,
        path: `${ROOT}\\item-acta\\acta-1913-p1.pdf`,
        type: 'pdf',
        sortIndex: 0,
        size: inflate ? big : PDF_BYTES.length,
        createdAt: 1754000510000,
        parentAssetId: 'asset-acta-pdf',
        pageNumber: 1,
      },
    ]
  }
  return [
    {
      id: 'asset-entrevista',
      itemId,
      path: `${ROOT}\\item-entrevista\\entrevista-1938.wav`,
      type: 'audio',
      sortIndex: 0,
      size: 4_200_000,
      createdAt: 1754000600000,
      parentAssetId: null,
    },
  ]
}

const EXTRACTIONS: Record<string, unknown> = {
  'asset-carta-p1': {
    id: 'extr-carta',
    assetId: 'asset-carta-p1',
    textContent:
      'Señor Intendente Municipal: los abajo firmantes, vecinos de la rambla, solicitan a Ud. la habilitación del Hotel Bristol para la temporada entrante.',
    method: 'ocr',
    confidence: 0.94,
    createdAt: 1754000700000,
  },
  'asset-acta-p1': {
    id: 'extr-acta',
    assetId: 'asset-acta-p1',
    textContent:
      'En la ciudad de Mar del Plata, a los ocho días del mes de enero de mil novecientos trece, se reúne la comisión directiva para dejar constancia de la inauguración del Hotel Bristol.',
    method: 'ocr',
    confidence: 0.91,
    createdAt: 1754000710000,
  },
}

const TRANSCRIPTIONS: Record<string, unknown> = {
  'asset-entrevista': {
    id: 'trans-entrevista',
    assetId: 'asset-entrevista',
    textContent:
      'Mi abuelo tenía la concesión de la confitería. Me contaba que en enero no se podía caminar por la rambla de tanta gente que venía de Buenos Aires.',
    language: 'es',
    model: 'whisper',
    createdAt: 1754000720000,
  },
}

const ENTITIES: Record<string, unknown[]> = {
  'item-carta': [
    {
      id: 'ent-1',
      itemId: 'item-carta',
      assetId: 'asset-carta-p1',
      entityType: 'institution',
      value: 'Hotel Bristol',
      startOffset: 96,
      endOffset: 109,
      confidence: 0.97,
      source: 'llm',
      modelName: 'google/gemma-4-26b-a4b-it',
      latitude: null,
      longitude: null,
      manualLatitude: null,
      manualLongitude: null,
      geoStatus: 'pending',
      createdAt: 1754000800000,
    },
    {
      id: 'ent-2',
      itemId: 'item-carta',
      assetId: 'asset-carta-p1',
      entityType: 'place',
      value: 'Mar del Plata',
      startOffset: 0,
      endOffset: 13,
      confidence: 0.99,
      source: 'llm',
      modelName: 'google/gemma-4-26b-a4b-it',
      latitude: -38.0055,
      longitude: -57.5426,
      manualLatitude: null,
      manualLongitude: null,
      geoStatus: 'resolved',
      createdAt: 1754000801000,
    },
  ],
  'item-acta': [
    {
      id: 'ent-3',
      itemId: 'item-acta',
      assetId: 'asset-acta-p1',
      entityType: 'date',
      value: '8 de enero de 1913',
      startOffset: 30,
      endOffset: 48,
      confidence: 0.95,
      source: 'llm',
      modelName: 'google/gemma-4-26b-a4b-it',
      latitude: null,
      longitude: null,
      manualLatitude: null,
      manualLongitude: null,
      geoStatus: 'pending',
      createdAt: 1754000802000,
    },
  ],
  'item-entrevista': [
    {
      id: 'ent-4',
      itemId: 'item-entrevista',
      assetId: null,
      entityType: 'person',
      value: 'Elena Sarasola',
      startOffset: 0,
      endOffset: 14,
      confidence: 0.88,
      source: 'manual',
      modelName: null,
      latitude: null,
      longitude: null,
      manualLatitude: null,
      manualLongitude: null,
      geoStatus: 'pending',
      createdAt: 1754000803000,
    },
  ],
}

const TRIPLES: Record<string, unknown[]> = {
  'item-carta': [
    {
      id: 'trip-1',
      itemId: 'item-carta',
      assetId: 'asset-carta-p1',
      subject: 'Hotel Bristol',
      predicate: 'está en',
      object: 'Mar del Plata',
      createdAt: 1754000810000,
    },
  ],
  'item-acta': [
    {
      id: 'trip-2',
      itemId: 'item-acta',
      assetId: 'asset-acta-p1',
      subject: 'Hotel Bristol',
      predicate: 'fue inaugurado en',
      object: '8 de enero de 1913',
      createdAt: 1754000811000,
    },
  ],
  'item-entrevista': [
    {
      id: 'trip-3',
      itemId: 'item-entrevista',
      assetId: null,
      subject: 'Elena Sarasola',
      predicate: 'es nieta de',
      object: 'el concesionario de la confitería',
      createdAt: 1754000812000,
    },
  ],
}

const NOTES: Record<string, unknown[]> = {
  'item-carta': [
    {
      id: 'note-1',
      itemId: 'item-carta',
      assetId: null,
      content: 'Chequear si el expediente sigue en el archivo de la Intendencia.',
      createdAt: 1754000820000,
      updatedAt: 1754000821000,
    },
    {
      id: 'note-2',
      itemId: 'item-carta',
      assetId: 'asset-carta-p1',
      content: 'La firma del margen derecho no se llega a leer.',
      createdAt: 1754000822000,
      updatedAt: 1754000823000,
    },
  ],
  'item-acta': [],
  'item-entrevista': [
    {
      id: 'note-3',
      itemId: 'item-entrevista',
      assetId: null,
      content: 'Pedirle a la familia el permiso de difusión antes de publicar.',
      createdAt: 1754000824000,
      updatedAt: 1754000825000,
    },
  ],
}

const TOPIC_ROWS = [
  { id: 'topic-1', name: 'turismo de temporada', created_at: 1754000830000 },
  { id: 'topic-2', name: 'hotelería', created_at: 1754000831000 },
]

const ITEM_TOPIC_ROWS = [
  { id: 'it-1', item_id: 'item-carta', topic_id: 'topic-2', created_at: 1754000840000 },
  { id: 'it-2', item_id: 'item-acta', topic_id: 'topic-2', created_at: 1754000841000 },
  { id: 'it-3', item_id: 'item-entrevista', topic_id: 'topic-1', created_at: 1754000842000 },
]

function buildStore(inflate: boolean) {
  return {
    collections: { findById: vi.fn().mockResolvedValue(COLLECTION) },
    items: { findByCollection: vi.fn().mockResolvedValue(ITEMS) },
    assets: {
      findByItem: vi
        .fn()
        .mockImplementation((itemId: string) => Promise.resolve(assetsFor(itemId, inflate))),
    },
    notes: {
      findByItem: vi
        .fn()
        .mockImplementation((itemId: string) => Promise.resolve(NOTES[itemId] ?? [])),
    },
    extractions: {
      findByAsset: vi
        .fn()
        .mockImplementation((assetId: string) => Promise.resolve(EXTRACTIONS[assetId] ?? null)),
    },
    annotations: {
      findByAsset: vi.fn().mockImplementation((assetId: string) =>
        Promise.resolve(
          assetId === 'asset-carta-p1'
            ? [
                {
                  id: 'ann-1',
                  assetId,
                  page: 1,
                  kind: 'rectangle',
                  color: '#e0b050',
                  x: 0.12,
                  y: 0.08,
                  width: 0.64,
                  height: 0.21,
                  createdAt: 1754000850000,
                  updatedAt: 1754000851000,
                },
              ]
            : []
        )
      ),
    },
    transcriptions: {
      findByAsset: vi
        .fn()
        .mockImplementation((assetId: string) => Promise.resolve(TRANSCRIPTIONS[assetId] ?? null)),
    },
    entities: {
      findByItemId: vi
        .fn()
        .mockImplementation((itemId: string) => Promise.resolve(ENTITIES[itemId] ?? [])),
    },
    triples: {
      findByItemId: vi
        .fn()
        .mockImplementation((itemId: string) => Promise.resolve(TRIPLES[itemId] ?? [])),
    },
    layouts: { findByAsset: vi.fn().mockResolvedValue(null) },
  }
}

function bytesForPath(path: string): Uint8Array {
  if (path.endsWith('.png')) return PNG_BYTES
  if (path.endsWith('.pdf')) return PDF_BYTES
  return WAV_BYTES
}

describe('ejemplos del puente EntropIA', () => {
  beforeEach(async () => {
    mkdirSync(OUT_DIR, { recursive: true })
    vi.clearAllMocks()

    const { writeFile, readFile, stat } = await import('@tauri-apps/plugin-fs')
    const { invoke } = await import('@tauri-apps/api/core')

    vi.mocked(writeFile).mockResolvedValue(undefined)
    vi.mocked(readFile).mockImplementation((path: string) =>
      Promise.resolve(bytesForPath(String(path)))
    )
    vi.mocked(stat).mockImplementation((path: string) =>
      Promise.resolve({ size: bytesForPath(String(path)).length } as never)
    )
    vi.mocked(invoke).mockImplementation((command: string, args?: unknown) => {
      if (command !== 'db_select') return Promise.resolve(null)
      const sql = String((args as Record<string, unknown> | undefined)?.sql ?? '')
      if (sql.includes('FROM item_topics it JOIN items')) return Promise.resolve(ITEM_TOPIC_ROWS)
      if (sql.includes('FROM topics t JOIN item_topics')) return Promise.resolve(TOPIC_ROWS)
      return Promise.resolve([])
    })
  })

  it('escribe coleccion-completa.json', async () => {
    const { save } = await import('@tauri-apps/plugin-dialog')
    const { writeFile } = await import('@tauri-apps/plugin-fs')
    vi.mocked(save).mockResolvedValue(join(OUT_DIR, 'coleccion-completa.json'))

    await exportCollectionById(buildStore(false) as never, COLLECTION.id, {
      includeAssetData: true,
      aiConfig: AI_CONFIG,
      aiConfigFilenameMarker: 'con llave',
    })

    expect(writeFile).toHaveBeenCalledTimes(1)
    const [path, bytes] = vi.mocked(writeFile).mock.calls[0]!
    writeFileSync(String(path), Buffer.from(bytes as Uint8Array))
    console.log(`[ejemplos] ${path}`)
  })

  it('escribe coleccion-tanda-1.json y coleccion-tanda-2.json', async () => {
    const { save } = await import('@tauri-apps/plugin-dialog')
    const { writeFile } = await import('@tauri-apps/plugin-fs')
    vi.mocked(save).mockResolvedValue(join(OUT_DIR, 'coleccion-tanda.json'))

    await exportCollectionById(buildStore(true) as never, COLLECTION.id, {
      includeAssetData: true,
      aiConfig: AI_CONFIG,
      aiConfigFilenameMarker: 'con llave',
    })

    expect(writeFile).toHaveBeenCalledTimes(2)
    const names = ['coleccion-tanda-1.json', 'coleccion-tanda-2.json']
    vi.mocked(writeFile).mock.calls.forEach(([, bytes], index) => {
      const target = join(OUT_DIR, names[index]!)
      writeFileSync(target, Buffer.from(bytes as Uint8Array))
      console.log(`[ejemplos] ${target}`)
    })
  })
})
