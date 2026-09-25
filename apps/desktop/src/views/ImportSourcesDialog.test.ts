/** @vitest-environment jsdom */

import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'
import { DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT } from '$lib/document-explorer'
import ImportSourcesDialog from './ImportSourcesDialog.svelte'

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((res) => {
    resolve = res
  })
  return { promise, resolve }
}

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

vi.mock('$lib/pane-context', () => ({
  getNavigation: () => navigationRef,
}))

vi.mock('$lib/file-import', () => ({
  pickFiles: fileImportRef.pickFiles,
}))

vi.mock('$lib/collection-import', async (importOriginal) => {
  const actual = await importOriginal<typeof import('$lib/collection-import')>()
  return {
    ...actual,
    importClassifiedPathsIntoCollection: collectionImportRef.importClassifiedPathsIntoCollection,
  }
})

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

  it('passes the id of the exact collection chosen, not just the first one listed', async () => {
    fileImportRef.pickFiles.mockResolvedValue(['/src/a.png'])
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    await fireEvent.click(screen.getByRole('radio', { name: /Movimiento Obrero MdP/ }))
    await fireEvent.click(screen.getByRole('button', { name: 'Elegir archivos' }))

    await waitFor(() =>
      expect(collectionImportRef.importClassifiedPathsIntoCollection).toHaveBeenCalledWith(
        ['/src/a.png'],
        'col-2',
        expect.objectContaining({ baseErrorMessage: expect.any(String) })
      )
    )
    expect(navigationRef.navigate).toHaveBeenCalledWith({
      name: 'collection',
      id: 'col-2',
      collectionName: 'Movimiento Obrero MdP',
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

describe('ImportSourcesDialog progress and result handling (T4b)', () => {
  it('shows import progress and disables Cancel/Escape while importing', async () => {
    fileImportRef.pickFiles.mockResolvedValue(['/src/a.png'])
    const { promise, resolve } = deferred<{
      classifiedCount: number
      rejected: string[]
      createdItems: Array<{ id: string; title: string }>
      importErrors: string[]
      alreadyImported: string[]
    }>()
    collectionImportRef.importClassifiedPathsIntoCollection.mockImplementation(
      (_paths: string[], _id: string, options: { onProgress?: (p: unknown) => void }) => {
        options.onProgress?.({
          total: 3,
          completed: 1,
          imported: 1,
          failed: 0,
          skipped: 0,
          currentFileName: 'a.png',
          stage: 'copyingFile',
        })
        return promise
      }
    )
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    await fireEvent.click(screen.getByRole('radio', { name: /Voces/ }))
    await fireEvent.click(screen.getByRole('button', { name: 'Elegir archivos' }))

    expect(await screen.findByText('1 de 3 archivos procesados.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Cancelar' })).toBeDisabled()

    await fireEvent.keyDown(window, { key: 'Escape' })
    expect(onClose).not.toHaveBeenCalled()

    resolve({
      classifiedCount: 1,
      rejected: [],
      createdItems: [{ id: 'item-1', title: 'a' }],
      importErrors: [],
      alreadyImported: [],
    })
    await waitFor(() => expect(navigationRef.navigate).toHaveBeenCalled())
  })

  it('shows a summary instead of navigating when some files were rejected', async () => {
    fileImportRef.pickFiles.mockResolvedValue(['/src/a.exe'])
    collectionImportRef.importClassifiedPathsIntoCollection.mockResolvedValue({
      classifiedCount: 0,
      rejected: ['a.exe'],
      createdItems: [],
      importErrors: [],
      alreadyImported: [],
    })
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    await fireEvent.click(screen.getByRole('radio', { name: /Voces/ }))
    await fireEvent.click(screen.getByRole('button', { name: 'Elegir archivos' }))

    expect(await screen.findByText('Omitidos: a.exe')).toBeInTheDocument()
    expect(navigationRef.navigate).not.toHaveBeenCalled()
    expect(onClose).not.toHaveBeenCalled()
    expect(screen.getByRole('button', { name: 'Ir a la colección' })).toBeInTheDocument()
  })

  it('shows a summary instead of navigating when a file was already imported', async () => {
    fileImportRef.pickFiles.mockResolvedValue(['/src/a.png'])
    collectionImportRef.importClassifiedPathsIntoCollection.mockResolvedValue({
      classifiedCount: 1,
      rejected: [],
      createdItems: [],
      importErrors: [],
      alreadyImported: ['a.png'],
    })
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    await fireEvent.click(screen.getByRole('radio', { name: /Voces/ }))
    await fireEvent.click(screen.getByRole('button', { name: 'Elegir archivos' }))

    expect(
      await screen.findByText('Ya estaban importados en esta colección: a.png')
    ).toBeInTheDocument()
    expect(navigationRef.navigate).not.toHaveBeenCalled()
  })

  it('keeps the summary open when a stray click lands on the overlay', async () => {
    // Choosing a file with a double click in the OS picker closes the picker on
    // the first click; the rest of the gesture lands on the webview, on this
    // dialog's overlay. A duplicate imports instantly, so the summary is already
    // up by then, and an overlay click used to close it before it was seen.
    fileImportRef.pickFiles.mockResolvedValue(['/src/a.png'])
    collectionImportRef.importClassifiedPathsIntoCollection.mockResolvedValue({
      classifiedCount: 1,
      rejected: [],
      createdItems: [],
      importErrors: [],
      alreadyImported: ['a.png'],
    })
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    await fireEvent.click(screen.getByRole('radio', { name: /Voces/ }))
    await fireEvent.click(screen.getByRole('button', { name: 'Elegir archivos' }))
    await screen.findByText('Ya estaban importados en esta colección: a.png')

    // The overlay floats out of the component into <body> (see `portal`).
    await fireEvent.click(document.querySelector('.confirm-dialog__overlay') as Element)

    expect(onClose).not.toHaveBeenCalled()
    expect(screen.getByText('Ya estaban importados en esta colección: a.png')).toBeInTheDocument()
  })

  it('shows a summary with the per-file error when importing a file fails', async () => {
    fileImportRef.pickFiles.mockResolvedValue(['/src/a.png'])
    collectionImportRef.importClassifiedPathsIntoCollection.mockResolvedValue({
      classifiedCount: 1,
      rejected: [],
      createdItems: [],
      importErrors: ['Importar fuentes (importing a.png): copy failed'],
      alreadyImported: [],
    })
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    await fireEvent.click(screen.getByRole('radio', { name: /Voces/ }))
    await fireEvent.click(screen.getByRole('button', { name: 'Elegir archivos' }))

    expect(
      await screen.findByText('Importar fuentes (importing a.png): copy failed')
    ).toBeInTheDocument()
    expect(navigationRef.navigate).not.toHaveBeenCalled()
  })

  it('navigates to the collection from the summary when "Ir a la colección" is clicked', async () => {
    fileImportRef.pickFiles.mockResolvedValue(['/src/a.exe'])
    collectionImportRef.importClassifiedPathsIntoCollection.mockResolvedValue({
      classifiedCount: 0,
      rejected: ['a.exe'],
      createdItems: [],
      importErrors: [],
      alreadyImported: [],
    })
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    await fireEvent.click(screen.getByRole('radio', { name: /Voces/ }))
    await fireEvent.click(screen.getByRole('button', { name: 'Elegir archivos' }))

    await fireEvent.click(await screen.findByRole('button', { name: 'Ir a la colección' }))

    expect(onClose).toHaveBeenCalledOnce()
    expect(navigationRef.navigate).toHaveBeenCalledWith({
      name: 'collection',
      id: 'col-1',
      collectionName: 'Voces',
    })
  })

  it('closes without navigating from the summary when "Cerrar" is clicked', async () => {
    fileImportRef.pickFiles.mockResolvedValue(['/src/a.exe'])
    collectionImportRef.importClassifiedPathsIntoCollection.mockResolvedValue({
      classifiedCount: 0,
      rejected: ['a.exe'],
      createdItems: [],
      importErrors: [],
      alreadyImported: [],
    })
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    await fireEvent.click(screen.getByRole('radio', { name: /Voces/ }))
    await fireEvent.click(screen.getByRole('button', { name: 'Elegir archivos' }))

    await fireEvent.click(await screen.findByRole('button', { name: 'Cerrar' }))

    expect(onClose).toHaveBeenCalledOnce()
    expect(navigationRef.navigate).not.toHaveBeenCalled()
  })

  it('dispatches the document-explorer collection-changed event after a successful import', async () => {
    fileImportRef.pickFiles.mockResolvedValue(['/src/a.png'])
    const handler = vi.fn()
    window.addEventListener(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, handler)
    render(ImportSourcesDialog, { props: { onClose } })

    await screen.findByText('Voces')
    await fireEvent.click(screen.getByRole('radio', { name: /Voces/ }))
    await fireEvent.click(screen.getByRole('button', { name: 'Elegir archivos' }))

    await waitFor(() => expect(handler).toHaveBeenCalledOnce())
    expect((handler.mock.calls[0]![0] as CustomEvent).detail).toEqual({ collectionId: 'col-1' })
    window.removeEventListener(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, handler)
  })
})
