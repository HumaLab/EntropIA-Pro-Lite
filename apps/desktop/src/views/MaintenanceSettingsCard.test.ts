import { fireEvent, render, screen, waitFor, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import MaintenanceSettingsCard from './MaintenanceSettingsCard.svelte'
import { locale } from '$lib/i18n'
import type { PathRepair, RepairScan } from '$lib/image-version-repair'

/**
 * Mantenimiento › "Revisar versiones de imágenes": the review lists first and
 * writes only what the person confirms.
 */

const { invokeMock, listenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))
vi.mock('@tauri-apps/api/event', () => ({ listen: listenMock }))

function repair(n: number): PathRepair {
  return {
    assetId: `asset-${n}`,
    itemId: `item-${n}`,
    name: `Acta ${n}`,
    shown: { fileName: `acta${n}_v4.jpg`, version: 4, width: 3456, height: 4608 },
    proposed: { fileName: `acta${n}_v11.jpg`, version: 11, width: 747, height: 1169 },
    token: `token-${n}`,
  }
}

function scanResult(repairs: PathRepair[], completions = 0): RepairScan {
  return {
    counts: {
      scanned: 1324,
      matching: 1201,
      repairable: repairs.length,
      needsLayoutRerun: 6,
      ambiguous: 1,
      noCandidate: 0,
      candidateAfterLayout: 0,
      fileMissing: 0,
    },
    repairs,
    completions: Array.from({ length: completions }, (_, index) => ({
      assetId: `old-${index}`,
      name: `Vieja ${index}`,
      fileName: `vieja${index}.jpg`,
      token: `complete-${index}`,
    })),
  }
}

/** Answers the two commands; `apply` may be replaced per test. */
function backend(scan: RepairScan, apply: () => Promise<unknown> = async () => undefined) {
  invokeMock.mockImplementation((command: string) => {
    if (command === 'scan_image_version_repairs') return Promise.resolve(scan)
    if (command === 'apply_image_version_repairs') return apply()
    return Promise.reject(new Error(`unexpected command ${command}`))
  })
}

const applyCalls = () =>
  invokeMock.mock.calls.filter(([command]) => command === 'apply_image_version_repairs')

/** What the only apply so far was sent. */
const appliedWith = () => applyCalls()[0]?.[1] as { repairs: unknown[]; completions: unknown[] }

const scanCalls = () =>
  invokeMock.mock.calls.filter(([command]) => command === 'scan_image_version_repairs')

async function review() {
  await fireEvent.click(screen.getByRole('button', { name: 'Revisar' }))
  await screen.findByText('Resultado de la revisión')
}

const applyButton = () => screen.getByRole('button', { name: 'Aplicar' })

describe('MaintenanceSettingsCard', () => {
  beforeEach(() => {
    locale.set('es')
    invokeMock.mockReset()
    listenMock.mockReset().mockResolvedValue(() => {})
  })

  it('reads nothing until the person asks for the review', () => {
    backend(scanResult([]))
    render(MaintenanceSettingsCard)

    expect(
      screen.getByRole('heading', { name: 'Revisar versiones de imágenes' })
    ).toBeInTheDocument()
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('shows the counts by reason and the proposals, all checked', async () => {
    backend(scanResult([repair(1), repair(2)]))
    render(MaintenanceSettingsCard)

    await review()

    const summary = screen.getByTestId('maintenance-summary')
    expect(
      within(summary).getByText('Imágenes con layout revisadas').nextElementSibling
    ).toHaveTextContent('1324')
    expect(
      within(summary).getByText(/hay que volver a correr el layout/).nextElementSibling
    ).toHaveTextContent('6')
    expect(
      within(summary).getByText(/Más de una versión coincide/).nextElementSibling
    ).toHaveTextContent('1')

    expect(screen.getByText('Imágenes a reparar: 2')).toBeInTheDocument()
    const rows = screen.getAllByTestId('maintenance-repair')
    expect(rows).toHaveLength(2)
    expect(rows[0]).toHaveTextContent('Acta 1')
    expect(rows[0]).toHaveTextContent('Versión 4 (3456×4608)')
    expect(rows[0]).toHaveTextContent('Versión 11 (747×1169)')
    for (const row of rows) expect(within(row).getByRole('checkbox')).toBeChecked()
    expect(screen.getByText('Seleccionadas: 2')).toBeInTheDocument()
    expect(applyCalls()).toHaveLength(0)
  })

  it('asks before applying, and cancelling writes nothing', async () => {
    backend(scanResult([repair(1), repair(2)]))
    render(MaintenanceSettingsCard)
    await review()

    await fireEvent.click(applyButton())

    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveTextContent('Vas a cambiar la versión mostrada de 2 imágenes.')
    expect(dialog).toHaveTextContent('No se borra ni se modifica ningún archivo.')
    expect(applyCalls()).toHaveLength(0)

    await fireEvent.click(within(dialog).getByRole('button', { name: 'Cancelar' }))

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
    expect(applyCalls()).toHaveLength(0)
    expect(scanCalls()).toHaveLength(1)
  })

  it('confirms one image and one layout in the singular', async () => {
    backend(scanResult([repair(1)], 1))
    render(MaintenanceSettingsCard)
    await review()

    await fireEvent.click(
      screen.getByRole('checkbox', { name: 'Anotar también el origen de 1 layout anterior' })
    )
    await fireEvent.click(applyButton())

    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveTextContent('Vas a cambiar la versión mostrada de 1 imagen.')
    expect(dialog).toHaveTextContent('Vas a anotar el origen de 1 layout.')
  })

  it('confirms one image and one layout in the singular in English', async () => {
    locale.set('en')
    backend(scanResult([repair(1)], 1))
    render(MaintenanceSettingsCard)
    await fireEvent.click(screen.getByRole('button', { name: 'Review' }))
    await screen.findByText('Review result')

    await fireEvent.click(
      screen.getByRole('checkbox', { name: 'Also record the source of 1 earlier layout' })
    )
    await fireEvent.click(screen.getByRole('button', { name: 'Apply' }))

    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveTextContent('You are about to change the version shown of 1 image.')
    expect(dialog).toHaveTextContent('You are about to record the source of 1 layout.')
  })

  it('sends only the checked proposals, then reviews again', async () => {
    backend(scanResult([repair(1), repair(2), repair(3)], 2), async () => ({
      repaired: [
        { itemId: 'item-1', assetId: 'asset-1', path: 'assets/a/acta1_v11.jpg' },
        { itemId: 'item-3', assetId: 'asset-3', path: 'assets/a/acta3_v11.jpg' },
      ],
      completed: 0,
    }))
    render(MaintenanceSettingsCard)
    await review()

    const second = screen.getAllByTestId('maintenance-repair')[1] as HTMLElement
    await fireEvent.click(within(second).getByRole('checkbox'))
    expect(screen.getByText('Seleccionadas: 2')).toBeInTheDocument()

    await fireEvent.click(applyButton())
    await fireEvent.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: 'Aplicar' })
    )

    await waitFor(() => expect(applyCalls()).toHaveLength(1))
    // Asset ids and the tokens of the scan: never a path. The origins of the
    // old layouts were not asked for, so none is sent.
    expect(appliedWith()).toEqual({
      repairs: [
        { assetId: 'asset-1', token: 'token-1' },
        { assetId: 'asset-3', token: 'token-3' },
      ],
      completions: [],
    })
    expect(
      await screen.findByText('Listo. Imágenes reparadas: 2. Orígenes de layout anotados: 0.')
    ).toBeInTheDocument()
    await waitFor(() => expect(scanCalls()).toHaveLength(2))
  })

  it('records the origin of the old layouts only when that is ticked', async () => {
    backend(scanResult([], 2), async () => ({ repaired: [], completed: 2 }))
    render(MaintenanceSettingsCard)
    await review()

    expect(screen.getByText('No hay imágenes para reparar.')).toBeInTheDocument()
    expect(applyButton()).toBeDisabled()

    await fireEvent.click(
      screen.getByRole('checkbox', { name: /Anotar también el origen de 2 layouts anteriores/ })
    )
    await fireEvent.click(applyButton())
    const dialog = screen.getByRole('dialog')
    expect(dialog).toHaveTextContent('Vas a anotar el origen de 2 layouts.')
    await fireEvent.click(within(dialog).getByRole('button', { name: 'Aplicar' }))

    await waitFor(() => expect(applyCalls()).toHaveLength(1))
    expect(appliedWith()).toEqual({
      repairs: [],
      completions: [
        { assetId: 'old-0', token: 'complete-0' },
        { assetId: 'old-1', token: 'complete-1' },
      ],
    })
  })

  it('cannot apply with nothing selected, and selects all again', async () => {
    backend(scanResult([repair(1), repair(2)]))
    render(MaintenanceSettingsCard)
    await review()

    await fireEvent.click(screen.getByRole('button', { name: 'Ninguna' }))

    expect(screen.getByText('Seleccionadas: 0')).toBeInTheDocument()
    expect(applyButton()).toBeDisabled()
    for (const row of screen.getAllByTestId('maintenance-repair')) {
      expect(within(row).getByRole('checkbox')).not.toBeChecked()
    }

    await fireEvent.click(screen.getByRole('button', { name: 'Seleccionar todas' }))

    expect(screen.getByText('Seleccionadas: 2')).toBeInTheDocument()
    expect(applyButton()).toBeEnabled()
  })

  it('shows an error of the review', async () => {
    invokeMock.mockRejectedValue('DB lock poisoned')
    render(MaintenanceSettingsCard)

    await fireEvent.click(screen.getByRole('button', { name: 'Revisar' }))

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'No se pudo revisar: DB lock poisoned'
    )
  })

  it('shows an error of the apply and keeps the list', async () => {
    backend(scanResult([repair(1)]), () => Promise.reject('disk I/O error'))
    render(MaintenanceSettingsCard)
    await review()

    await fireEvent.click(applyButton())
    await fireEvent.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: 'Aplicar' })
    )

    expect(await screen.findByRole('alert')).toHaveTextContent('No se pudo aplicar: disk I/O error')
    expect(screen.getAllByTestId('maintenance-repair')).toHaveLength(1)
  })

  it('says that nothing was applied when the archive changed after the review', async () => {
    backend(scanResult([repair(1)]), () => Promise.reject('stale_proposal: asset-1'))
    render(MaintenanceSettingsCard)
    await review()

    await fireEvent.click(applyButton())
    await fireEvent.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: 'Aplicar' })
    )

    expect(await screen.findByRole('alert')).toHaveTextContent(
      'El archivo cambió después de la revisión y no se aplicó nada. Volvé a revisar.'
    )
  })

  it('renders a long list in pages and still applies every checked proposal', async () => {
    const many = Array.from({ length: 130 }, (_, index) => repair(index + 1))
    backend(scanResult(many), async () => ({ repaired: [], completed: 0 }))
    render(MaintenanceSettingsCard)
    await review()

    expect(screen.getAllByTestId('maintenance-repair')).toHaveLength(100)
    await fireEvent.click(screen.getByRole('button', { name: 'Mostrar 30 más' }))
    expect(screen.getAllByTestId('maintenance-repair')).toHaveLength(130)

    await fireEvent.click(applyButton())
    await fireEvent.click(
      within(screen.getByRole('dialog')).getByRole('button', { name: 'Aplicar' })
    )

    await waitFor(() => expect(applyCalls()).toHaveLength(1))
    expect(appliedWith().repairs).toHaveLength(130)
  })

  it('reports how far the review is while it runs', async () => {
    let progress: (event: { payload: { done: number; total: number } }) => void = () => {}
    listenMock.mockImplementation((_event: string, handler: typeof progress) => {
      progress = handler
      return Promise.resolve(() => {})
    })
    let finish: (scan: RepairScan) => void = () => {}
    invokeMock.mockImplementation(() => new Promise<RepairScan>((resolve) => (finish = resolve)))
    render(MaintenanceSettingsCard)

    await fireEvent.click(screen.getByRole('button', { name: 'Revisar' }))
    await waitFor(() => expect(invokeMock).toHaveBeenCalled())
    progress({ payload: { done: 40, total: 1324 } })

    expect(await screen.findByText('Revisando 40 de 1324 imágenes…')).toBeInTheDocument()
    expect(listenMock).toHaveBeenCalledWith('image-version-repairs:progress', expect.any(Function))

    finish(scanResult([]))
    expect(await screen.findByText('Resultado de la revisión')).toBeInTheDocument()
  })
})
