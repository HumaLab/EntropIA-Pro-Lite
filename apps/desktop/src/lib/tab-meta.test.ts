import { describe, it, expect, beforeEach } from 'vitest'
import { tabTitle, tabIcon } from './tab-meta'
import { locale, t } from './i18n'

describe('tab-meta', () => {
  beforeEach(() => locale.set('es'))

  it('titles every non-writing, non-investigation view from its section name or subject', () => {
    expect(tabTitle({ name: 'home' })).toBe('Inicio')
    expect(tabTitle({ name: 'collections' })).toBe('Colecciones')
    expect(tabTitle({ name: 'collection', id: 'c1', collectionName: 'Archivo' })).toBe('Archivo')
    expect(
      tabTitle({
        name: 'item',
        collectionId: 'c1',
        collectionName: 'Archivo',
        itemId: 'i1',
        itemTitle: 'Acta',
      })
    ).toBe('Acta')
    expect(tabTitle({ name: 'db-browser' })).toBe('Base de datos')
    expect(tabTitle({ name: 'rag-chat' })).toBe('Chat')
    expect(tabTitle({ name: 'research' })).toBe('Investigación')
    expect(tabTitle({ name: 'investigation', jobId: 'j1', title: 'Pregunta' })).toBe('Pregunta')
    expect(tabTitle({ name: 'settings' })).toBe('Configuración')
  })

  it('titles writing by its open document, falling back to the section name with none open', () => {
    expect(tabTitle({ name: 'writing', documentId: 'w1', documentTitle: 'Manuscrito' })).toBe(
      'Manuscrito'
    )
    expect(tabTitle({ name: 'writing' })).toBe(t('writing.title'))
  })

  it('icons every view name, sharing collections/collection and research/investigation', () => {
    expect(tabIcon({ name: 'home' })).toBe('home')
    expect(tabIcon({ name: 'collections' })).toBe('folder')
    expect(tabIcon({ name: 'collection', id: 'c1', collectionName: 'A' })).toBe('folder')
    expect(tabIcon({ name: 'research' })).toBe('research')
    expect(tabIcon({ name: 'investigation', jobId: 'j1', title: 'Q' })).toBe('research')
    expect(tabIcon({ name: 'writing' })).toBe('edit')
  })
})
