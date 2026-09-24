import { describe, expect, it } from 'vitest'
import { getDbBrowserCellContent, pickInitialDbBrowserTable } from './db-browser-view'

describe('getDbBrowserCellContent', () => {
  it('pretty-prints JSON strings for expanded viewing', () => {
    const result = getDbBrowserCellContent('{"title":"Acta","meta":{"page":2}}', '—')

    expect(result.rawText).toBe('{"title":"Acta","meta":{"page":2}}')
    expect(result.isJson).toBe(true)
    expect(result.canExpand).toBe(true)
    expect(result.expandedText).toContain('\n  "title": "Acta"')
    expect(result.expandedText).toContain('\n  "meta": {')
  })

  it('serializes object values without losing pretty JSON in the dialog', () => {
    const result = getDbBrowserCellContent({ ok: true, count: 3 }, '—')

    expect(result.rawText).toBe('{"ok":true,"count":3}')
    expect(result.isJson).toBe(true)
    expect(result.canExpand).toBe(true)
    expect(result.expandedText).toContain('"ok": true')
    expect(result.expandedText).toContain('"count": 3')
  })

  it('keeps long plain text copyable and expandable', () => {
    const longText = 'Texto largo '.repeat(20).trim()

    const result = getDbBrowserCellContent(longText, '—')

    expect(result.rawText).toBe(longText)
    expect(result.isJson).toBe(false)
    expect(result.canExpand).toBe(true)
    expect(result.expandedText).toBe(longText)
  })

  it('summarizes a BLOB by size and keeps its Base64 payload for copy and expand', () => {
    // 240 Base64 chars without padding decode to 180 bytes.
    const payload = 'QUJDREVG'.repeat(30)

    const result = getDbBrowserCellContent(payload, '—', (bytes) => `BLOB · ${bytes} bytes`)

    expect(result.rawText).toBe('BLOB · 180 bytes')
    expect(result.copyText).toBe(payload)
    expect(result.expandedText).toBe(payload)
    expect(result.canExpand).toBe(true)
    expect(result.isJson).toBe(false)
    expect(result.hasValue).toBe(true)
  })

  it('counts Base64 padding when sizing a BLOB', () => {
    const summary = (bytes: number) => `${bytes}`

    expect(getDbBrowserCellContent('aGk=', '—', summary).rawText).toBe('2')
    expect(getDbBrowserCellContent('aA==', '—', summary).rawText).toBe('1')
    expect(getDbBrowserCellContent('', '—', summary).rawText).toBe('0')
  })

  it('copies exactly what a non-BLOB cell shows', () => {
    expect(getDbBrowserCellContent('Acta', '—').copyText).toBe('Acta')
    expect(getDbBrowserCellContent(42, '—').copyText).toBe('42')
  })

  it('returns the empty placeholder without actions for null values', () => {
    const result = getDbBrowserCellContent(null, '—')

    expect(result.rawText).toBe('—')
    expect(result.canExpand).toBe(false)
    expect(result.hasValue).toBe(false)
  })
})

describe('pickInitialDbBrowserTable', () => {
  it('opens on extractions, the table people come here to read', () => {
    expect(
      pickInitialDbBrowserTable([
        { name: '_migrations' },
        { name: 'assets' },
        { name: 'extractions' },
      ])
    ).toBe('extractions')
  })

  it('falls back to the first listed table when extractions is not browsable', () => {
    expect(pickInitialDbBrowserTable([{ name: 'assets' }, { name: 'items' }])).toBe('assets')
  })

  it('returns null when there is nothing to browse', () => {
    expect(pickInitialDbBrowserTable([])).toBeNull()
  })
})
