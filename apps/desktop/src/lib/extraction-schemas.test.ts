import { describe, expect, it } from 'vitest'
import { recordsCsv, type ExtractionSchema } from './extraction-schemas'

describe('recordsCsv', () => {
  it('writes one row per record, joins lists and escapes quotes', () => {
    const schema: ExtractionSchema = {
      id: 's1',
      name: 'Barcos',
      model: '',
      fields: [
        { name: 'barco', description: '', repeatable: false },
        { name: 'carga', description: '', repeatable: true },
      ],
    }
    const csv = recordsCsv(
      schema,
      [
        {
          item_id: 'i1',
          item_title: 'Parte "del puerto"',
          asset_id: 'a1',
          record: { barco: 'La Esperanza', carga: ['cueros', 'sebo'] },
        },
        { item_id: 'i1', item_title: 'Parte', asset_id: 'a1', record: { barco: null, carga: [] } },
      ],
      'Documento'
    )
    expect(csv).toBe(
      '﻿"Documento","barco","carga"\r\n' +
        '"Parte ""del puerto""","La Esperanza","cueros; sebo"\r\n' +
        '"Parte","",""\r\n'
    )
  })
})
