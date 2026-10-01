/** @vitest-environment jsdom */

import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'
import { DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT } from '$lib/document-explorer'
import { CopyError } from '$lib/navegador-copy'
import NavegadorCopyDialog from './NavegadorCopyDialog.svelte'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

const { storeRef, copyRef } = vi.hoisted(() => ({
  storeRef: {
    current: {
      collections: {
        findAll: vi.fn(),
        create: vi.fn(),
        deleteIfEmpty: vi.fn(),
      },
    },
  },
  copyRef: {
    copyCaptureToCollection: vi.fn(),
    findExistingCopy: vi.fn(),
  },
}))

vi.mock('$lib/db', () => ({ getStore: () => storeRef.current }))
vi.mock('$lib/navegador-copy', async (importOriginal) => {
  const actual = await importOriginal<typeof import('$lib/navegador-copy')>()
  return {
    ...actual,
    copyCaptureToCollection: copyRef.copyCaptureToCollection,
    findExistingCopy: copyRef.findExistingCopy,
  }
})

const collection = (id: string, name: string) => ({
  id,
  name,
  description: null,
  createdAt: 1,
  updatedAt: 1,
})

const capture = { id: 'c1', title: 'La cuestión social' }
const onclose = vi.fn()
const onopenitem = vi.fn()

function open() {
  return render(NavegadorCopyDialog, { props: { capture, onclose, onopenitem } })
}

const confirm = () => screen.getByRole('button', { name: 'Copiar' })

beforeEach(() => {
  locale.set('es')
  vi.clearAllMocks()
  storeRef.current = {
    collections: {
      findAll: vi
        .fn()
        .mockResolvedValue([collection('col-1', 'Voces'), collection('col-2', 'Obras')]),
      create: vi.fn().mockResolvedValue(collection('col-new', 'Archivo nuevo')),
      deleteIfEmpty: vi.fn().mockResolvedValue(true),
    },
  }
  copyRef.findExistingCopy.mockResolvedValue(null)
  copyRef.copyCaptureToCollection.mockResolvedValue({
    item: { id: 'item-9', title: 'La cuestión social' },
    collectionId: 'col-1',
  })
})

describe('NavegadorCopyDialog', () => {
  it('lists the collections and a new-collection option, and says the copy is independent', async () => {
    open()

    expect(await screen.findByText('Voces')).toBeInTheDocument()
    expect(screen.getByText('Obras')).toBeInTheDocument()
    expect(screen.getByText('+ Nueva colección')).toBeInTheDocument()
    expect(screen.getByRole('group', { name: 'Colección de destino' })).toBeInTheDocument()
    expect(screen.getByText(/borrar la fuente no la toca/)).toBeInTheDocument()
    expect(screen.getByText(/La cuestión social/)).toBeInTheDocument()
  })

  it('says a page or a selection is copied as a PDF of its text, not as the page', async () => {
    render(NavegadorCopyDialog, { props: { capture, rendered: true, onclose, onopenitem } })

    expect(await screen.findByText(/PDF con el texto de «La cuestión social»/)).toBeInTheDocument()
    expect(screen.getByText(/No reproduce el diseño ni las imágenes/)).toBeInTheDocument()
    expect(screen.getByText(/borrar la fuente no la toca/)).toBeInTheDocument()
  })

  it('shows the English wording when the language is English', async () => {
    locale.set('en')
    render(NavegadorCopyDialog, { props: { capture, rendered: true, onclose, onopenitem } })

    expect(await screen.findByText(/PDF with the text of "La cuestión social"/)).toBeInTheDocument()
  })

  it('copies nothing until a destination is chosen', async () => {
    open()
    await screen.findByText('Voces')

    expect(confirm()).toBeDisabled()
    expect(copyRef.copyCaptureToCollection).not.toHaveBeenCalled()
  })

  it('copies into the chosen collection and offers to open the new document', async () => {
    const changed = vi.fn()
    window.addEventListener(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, changed)
    open()
    await fireEvent.click(await screen.findByLabelText(/Voces/))

    await fireEvent.click(confirm())

    await screen.findByText(/Copia creada en «Voces»: «La cuestión social»/)
    expect(copyRef.copyCaptureToCollection).toHaveBeenCalledWith(
      expect.objectContaining({ captureId: 'c1', collectionId: 'col-1' })
    )
    expect(copyRef.findExistingCopy).toHaveBeenCalledWith('col-1', 'c1')
    expect(changed).toHaveBeenCalledOnce()

    await fireEvent.click(screen.getByRole('button', { name: 'Abrir documento' }))
    expect(onopenitem).toHaveBeenCalledWith({
      collectionId: 'col-1',
      collectionName: 'Voces',
      itemId: 'item-9',
      itemTitle: 'La cuestión social',
    })
    expect(onclose).toHaveBeenCalled()
    window.removeEventListener(DOCUMENT_EXPLORER_COLLECTION_CHANGED_EVENT, changed)
  })

  it('can be closed after a copy without opening anything', async () => {
    open()
    await fireEvent.click(await screen.findByLabelText(/Voces/))
    await fireEvent.click(confirm())
    await screen.findByText(/Copia creada/)

    await fireEvent.click(screen.getByRole('button', { name: 'Cerrar' }))

    expect(onclose).toHaveBeenCalled()
    expect(onopenitem).not.toHaveBeenCalled()
  })

  describe('a capture that was already copied to that collection', () => {
    beforeEach(() => {
      copyRef.findExistingCopy.mockResolvedValue({ id: 'item-5', title: 'Copia anterior' })
    })

    it('asks before making a second copy and copies nothing meanwhile', async () => {
      open()
      await fireEvent.click(await screen.findByLabelText(/Voces/))

      await fireEvent.click(confirm())

      expect(
        await screen.findByText(
          'Esta captura ya está copiada en «Voces» como «Copia anterior». Si copias otra vez, se crea un segundo documento independiente.'
        )
      ).toBeInTheDocument()
      expect(copyRef.copyCaptureToCollection).not.toHaveBeenCalled()
    })

    it('makes the second copy only when the person says so', async () => {
      open()
      await fireEvent.click(await screen.findByLabelText(/Voces/))
      await fireEvent.click(confirm())

      await fireEvent.click(await screen.findByRole('button', { name: 'Copiar otra vez' }))

      await screen.findByText(/Copia creada/)
      expect(copyRef.copyCaptureToCollection).toHaveBeenCalledOnce()
    })

    it('makes no copy when the person cancels at the question', async () => {
      open()
      await fireEvent.click(await screen.findByLabelText(/Voces/))
      await fireEvent.click(confirm())
      await screen.findByRole('button', { name: 'Copiar otra vez' })

      await fireEvent.click(screen.getByRole('button', { name: 'Cancelar' }))

      expect(copyRef.copyCaptureToCollection).not.toHaveBeenCalled()
      expect(onclose).toHaveBeenCalled()
    })
  })

  describe('into a new collection', () => {
    async function chooseNew(name: string) {
      open()
      await screen.findByText('Voces')
      await fireEvent.click(
        screen.getByLabelText(/\+ Nueva colección/, { selector: 'input[type=radio]' })
      )
      await fireEvent.input(screen.getByLabelText('Nombre de la nueva colección'), {
        target: { value: name },
      })
    }

    it('needs a name before it copies', async () => {
      open()
      await screen.findByText('Voces')
      await fireEvent.click(
        screen.getByLabelText(/\+ Nueva colección/, { selector: 'input[type=radio]' })
      )

      expect(confirm()).toBeDisabled()
    })

    it('creates the collection and copies into it', async () => {
      copyRef.copyCaptureToCollection.mockResolvedValue({
        item: { id: 'item-9', title: 'La cuestión social' },
        collectionId: 'col-new',
      })
      await chooseNew('  Archivo nuevo ')

      await fireEvent.click(confirm())

      await screen.findByText(/Copia creada en «Archivo nuevo»/)
      expect(storeRef.current.collections.create).toHaveBeenCalledWith({
        name: 'Archivo nuevo',
        description: null,
      })
      expect(copyRef.copyCaptureToCollection).toHaveBeenCalledWith(
        expect.objectContaining({ captureId: 'c1', collectionId: 'col-new' })
      )
      expect(copyRef.findExistingCopy).not.toHaveBeenCalled()
    })

    it('does not leave the empty collection behind when the copy fails', async () => {
      copyRef.copyCaptureToCollection.mockRejectedValue(new CopyError('file_missing'))
      await chooseNew('Archivo nuevo')

      await fireEvent.click(confirm())

      await screen.findByText(/el archivo del PDF ya no está en este equipo/)
      expect(storeRef.current.collections.deleteIfEmpty).toHaveBeenCalledWith('col-new')
    })
  })

  it('says why a copy failed and stays open to try again', async () => {
    copyRef.copyCaptureToCollection.mockRejectedValue(new CopyError('file_changed'))
    open()
    await fireEvent.click(await screen.findByLabelText(/Voces/))

    await fireEvent.click(confirm())

    expect(
      await screen.findByText(/lo guardado ya no coincide con lo que se verificó/)
    ).toBeInTheDocument()
    expect(screen.queryByText(/Copia creada/)).toBeNull()
    expect(onclose).not.toHaveBeenCalled()
    expect(confirm()).toBeEnabled()
  })

  it('shows an import failure with its own words', async () => {
    copyRef.copyCaptureToCollection.mockRejectedValue(new CopyError('import_failed', 'disk full'))
    open()
    await fireEvent.click(await screen.findByLabelText(/Voces/))

    await fireEvent.click(confirm())

    expect(await screen.findByText('No se pudo copiar: disk full')).toBeInTheDocument()
  })

  it('cannot be cancelled while the copy runs', async () => {
    const pending = deferred<unknown>()
    copyRef.copyCaptureToCollection.mockReturnValue(pending.promise)
    open()
    await fireEvent.click(await screen.findByLabelText(/Voces/))

    await fireEvent.click(confirm())

    await screen.findByText('Copiando el PDF…')
    expect(screen.getByRole('button', { name: 'Cancelar' })).toBeDisabled()
    pending.resolve({ item: { id: 'i', title: 't' }, collectionId: 'col-1' })
    await screen.findByText(/Copia creada/)
  })

  it('says why the collections could not be read', async () => {
    storeRef.current.collections.findAll.mockRejectedValue(new Error('db locked'))
    open()

    expect(
      await screen.findByText('No se pudieron leer las colecciones: db locked')
    ).toBeInTheDocument()
  })

  it('speaks English', async () => {
    locale.set('en')
    open()
    await fireEvent.click(await screen.findByLabelText(/Voces/))

    await fireEvent.click(screen.getByRole('button', { name: 'Copy' }))

    await waitFor(() => expect(screen.getByText(/Copy created in "Voces"/)).toBeInTheDocument())
    expect(screen.getByRole('button', { name: 'Open document' })).toBeInTheDocument()
  })
})
