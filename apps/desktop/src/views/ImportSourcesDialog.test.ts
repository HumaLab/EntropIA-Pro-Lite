/** @vitest-environment jsdom */

import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'
import ImportSourcesDialog from './ImportSourcesDialog.svelte'

const { storeRef, navigationRef, fileImportRef, collectionImportRef } = vi.hoisted(() => ({
  storeRef: {
    current: {
      collections: {
        findAll: vi.fn(),
        countItems: vi.fn(),
        create: vi.fn(),
      },
    },
  },
  navigationRef: {
    navigate: vi.fn(),
  },
  fileImportRef: {
    pickFiles: vi.fn(),
  },
  collectionImportRef: {
    importClassifiedPathsIntoCollection: vi.fn(),
  },
}))

type CollectionRow = {
  id: string
  name: string
  description: string | null
  createdAt: number
  updatedAt: number
}

function createStore(rows: CollectionRow[], counts: Record<string, number> = {}) {
  return {
    collections: {
      findAll: vi.fn().mockResolvedValue(rows),
      countItems: vi.fn().mockImplementation((id: string) => Promise.resolve(counts[id] ?? 0)),
      create: vi.fn(),
    },
  }
}

vi.mock('$lib/db', () => ({
  getStore: () => storeRef.current,
}))

vi.mock('$lib/navigation', () => ({
  navigation: navigationRef,
}))

vi.mock('$lib/file-import', () => ({
  pickFiles: fileImportRef.pickFiles,
}))

vi.mock('$lib/collection-import', () => ({
  importClassifiedPathsIntoCollection: collectionImportRef.importClassifiedPathsIntoCollection,
}))

const onClose = vi.fn()

beforeEach(() => {
  locale.set('es')
  onClose.mockReset()
  navigationRef.navigate.mockReset()
  fileImportRef.pickFiles.mockReset()
  collectionImportRef.importClassifiedPathsIntoCollection.mockReset()
  collectionImportRef.importClassifiedPathsIntoCollection.mockResolvedValue({
    classifiedCount: 1,
    rejected: [],
    createdItems: [{ id: 'item-1', title: 'a' }],
    importErrors: [],
    alreadyImported: [],
  })
  storeRef.current = createStore(
    [
      {
        id: 'col-1',
        name: 'Voces',
        description: null,
        createdAt: 3,
        updatedAt: 3,
      },
      {
        id: 'col-2',
        name: 'Movimiento Obrero MdP',
        description: null,
        createdAt: 2,
        updatedAt: 2,
      },
    ],
    { 'col-1': 19, 'col-2': 1244 }
  )
})

describe('ImportSourcesDialog', () => {
  it('lists existing collections with their document counts, and a "new collection" option', async () => {
    render(ImportSourcesDialog, { props: { onClose } })

    expect(await screen.findByText('Voces')).toBeInTheDocument()
    expect(screen.getByText('Movimiento Obrero MdP')).toBeInTheDocument()
    expect(screen.getByText('+ Nueva colección')).toBeInTheDocument()
    expect(screen.getByText('19')).toBeInTheDocument()
    expect(screen.getByText('1.244')).toBeInTheDocument()
  })

  it('exposes the destination choice as an accessible group with a legend', async () => {
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    expect(screen.getByRole('group', { name: 'Destino' })).toBeInTheDocument()
  })

  it('keeps "Elegir archivos" disabled until a destination is chosen', async () => {
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    expect(screen.getByRole('button', { name: 'Elegir archivos' })).toBeDisabled()

    await fireEvent.click(screen.getByRole('radio', { name: /Voces/ }))
    expect(screen.getByRole('button', { name: 'Elegir archivos' })).not.toBeDisabled()
  })

  it('keeps "Elegir archivos" disabled when "new collection" is chosen without a name', async () => {
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    await fireEvent.click(screen.getByRole('radio', { name: /Nueva colección/ }))
    expect(screen.getByRole('button', { name: 'Elegir archivos' })).toBeDisabled()

    await fireEvent.input(screen.getByPlaceholderText('Nombre de la colección'), {
      target: { value: 'Fuentes orales' },
    })
    expect(screen.getByRole('button', { name: 'Elegir archivos' })).not.toBeDisabled()
  })

  it('closes without side effects when Cancelar is clicked', async () => {
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    await fireEvent.click(screen.getByRole('button', { name: 'Cancelar' }))

    expect(onClose).toHaveBeenCalledOnce()
    expect(fileImportRef.pickFiles).not.toHaveBeenCalled()
  })

  it('closes without side effects on Escape', async () => {
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    await fireEvent.keyDown(window, { key: 'Escape' })

    expect(onClose).toHaveBeenCalledOnce()
  })

  it('imports nothing and stays open when the file picker is cancelled', async () => {
    fileImportRef.pickFiles.mockResolvedValue([])
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    await fireEvent.click(screen.getByRole('radio', { name: /Voces/ }))
    await fireEvent.click(screen.getByRole('button', { name: 'Elegir archivos' }))

    await waitFor(() => expect(fileImportRef.pickFiles).toHaveBeenCalledOnce())
    expect(collectionImportRef.importClassifiedPathsIntoCollection).not.toHaveBeenCalled()
    expect(onClose).not.toHaveBeenCalled()
    expect(storeRef.current.collections.create).not.toHaveBeenCalled()
  })

  it('imports into the chosen existing collection and navigates there', async () => {
    fileImportRef.pickFiles.mockResolvedValue(['/src/a.png'])
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    await fireEvent.click(screen.getByRole('radio', { name: /Voces/ }))
    await fireEvent.click(screen.getByRole('button', { name: 'Elegir archivos' }))

    await waitFor(() =>
      expect(collectionImportRef.importClassifiedPathsIntoCollection).toHaveBeenCalledWith(
        ['/src/a.png'],
        'col-1',
        expect.objectContaining({ baseErrorMessage: expect.any(String) })
      )
    )
    expect(storeRef.current.collections.create).not.toHaveBeenCalled()
    expect(onClose).toHaveBeenCalledOnce()
    expect(navigationRef.navigate).toHaveBeenCalledWith({
      name: 'collection',
      id: 'col-1',
      collectionName: 'Voces',
    })
  })

  it('creates the new collection only after files were picked, then imports into it', async () => {
    fileImportRef.pickFiles.mockResolvedValue(['/src/a.png'])
    storeRef.current.collections.create.mockResolvedValue({
      id: 'col-new',
      name: 'Fuentes orales',
      description: null,
      createdAt: 9,
      updatedAt: 9,
    })
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    await fireEvent.click(screen.getByRole('radio', { name: /Nueva colección/ }))
    await fireEvent.input(screen.getByPlaceholderText('Nombre de la colección'), {
      target: { value: 'Fuentes orales' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Elegir archivos' }))

    await waitFor(() => expect(storeRef.current.collections.create).toHaveBeenCalledOnce())
    expect(storeRef.current.collections.create).toHaveBeenCalledWith({
      name: 'Fuentes orales',
      description: null,
    })
    expect(collectionImportRef.importClassifiedPathsIntoCollection).toHaveBeenCalledWith(
      ['/src/a.png'],
      'col-new',
      expect.objectContaining({ baseErrorMessage: expect.any(String) })
    )
    expect(navigationRef.navigate).toHaveBeenCalledWith({
      name: 'collection',
      id: 'col-new',
      collectionName: 'Fuentes orales',
    })
  })

  it('does not create a new collection when the file picker is cancelled', async () => {
    fileImportRef.pickFiles.mockResolvedValue([])
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    await fireEvent.click(screen.getByRole('radio', { name: /Nueva colección/ }))
    await fireEvent.input(screen.getByPlaceholderText('Nombre de la colección'), {
      target: { value: 'Fuentes orales' },
    })
    await fireEvent.click(screen.getByRole('button', { name: 'Elegir archivos' }))

    await waitFor(() => expect(fileImportRef.pickFiles).toHaveBeenCalledOnce())
    expect(storeRef.current.collections.create).not.toHaveBeenCalled()
    expect(collectionImportRef.importClassifiedPathsIntoCollection).not.toHaveBeenCalled()
  })

  it('shows an inline error when loading collections fails', async () => {
    storeRef.current.collections.findAll.mockRejectedValue(new Error('db locked'))
    render(ImportSourcesDialog, { props: { onClose } })

    expect(await screen.findByRole('alert')).toHaveTextContent('db locked')
  })
})
