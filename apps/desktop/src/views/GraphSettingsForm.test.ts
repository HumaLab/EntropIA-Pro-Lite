import { describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen } from '@testing-library/svelte'
import GraphSettingsForm from './GraphSettingsForm.svelte'
import { DEFAULT_GRAPH_SETTINGS, normalizeGraphSettings } from '$lib/analysis-settings'

function setup(graph = normalizeGraphSettings(undefined)) {
  const onchange = vi.fn()
  render(GraphSettingsForm, { graph, onchange })
  return onchange
}

describe('GraphSettingsForm', () => {
  it('groups the Relaciones 3D options into labelled sections with the defaults', () => {
    setup()
    expect(screen.getByRole('heading', { name: 'Relaciones 3D' })).toBeInTheDocument()
    for (const section of ['Qué se muestra', 'Representación', 'Exploración']) {
      expect(screen.getByRole('group', { name: section })).toBeInTheDocument()
    }
    expect(screen.getByLabelText('Máximo de entidades visibles')).toHaveValue(300)
    expect(screen.getByLabelText('Máximo de relaciones visibles')).toHaveValue(1000)
    expect(screen.getByLabelText('Mínimo de conexiones por entidad')).toHaveValue(0)
    expect(screen.getByLabelText('Mínimo de apariciones como sujeto')).toHaveValue(1)
    expect(screen.getByLabelText('Mínimo de apariciones como objeto')).toHaveValue(1)
    expect(screen.getByLabelText('Mínimo de repeticiones por relación')).toHaveValue(1)
    expect(screen.getByLabelText('Incluir nodos aislados')).toBeChecked()
    expect(screen.getByLabelText('Tamaño de los nodos')).toHaveValue(1)
    expect(screen.getByLabelText('Opacidad de las relaciones')).toHaveValue(0.2)
    expect(screen.getByLabelText('Mostrar flechas')).toBeChecked()
    expect(screen.getByLabelText('Mostrar el vínculo al pasar el mouse')).toBeChecked()
    expect(screen.getByLabelText('Distancia entre nodos conectados')).toHaveValue(30)
    expect(screen.getByLabelText('Fuerza de repulsión')).toHaveValue(30)
    expect(screen.getByLabelText('Tiempo de acomodamiento (segundos)')).toHaveValue(15)
    expect(screen.getByLabelText('Rotación automática')).not.toBeChecked()
  })

  it('emits a normalized structure change and clamps out-of-range values', async () => {
    const onchange = setup()
    const max = screen.getByLabelText('Máximo de entidades visibles')
    await fireEvent.change(max, { target: { value: '50' } })
    expect(onchange).toHaveBeenLastCalledWith(
      expect.objectContaining({
        structure: { ...DEFAULT_GRAPH_SETTINGS.structure, maxEntities: 50 },
        render: { ...DEFAULT_GRAPH_SETTINGS.render },
      })
    )
    await fireEvent.change(max, { target: { value: '99999' } })
    expect(onchange.mock.lastCall?.[0].structure.maxEntities).toBe(3000)
    expect(max).toHaveValue(3000)
  })

  it('emits render changes, including repulsion as a negative charge and seconds as ms', async () => {
    const onchange = setup()
    await fireEvent.change(screen.getByLabelText('Opacidad de las relaciones'), {
      target: { value: '0.6' },
    })
    expect(onchange.mock.lastCall?.[0].render.linkOpacity).toBe(0.6)
    await fireEvent.change(screen.getByLabelText('Fuerza de repulsión'), {
      target: { value: '120' },
    })
    expect(onchange.mock.lastCall?.[0].render.chargeStrength).toBe(-120)
    await fireEvent.change(screen.getByLabelText('Tiempo de acomodamiento (segundos)'), {
      target: { value: '4' },
    })
    expect(onchange.mock.lastCall?.[0].render.cooldownTime).toBe(4000)
    await fireEvent.click(screen.getByLabelText('Incluir nodos aislados'))
    expect(onchange.mock.lastCall?.[0].structure.includeIsolated).toBe(false)
  })

  it('restores only the graph defaults', async () => {
    const changed = normalizeGraphSettings({
      structure: { maxEntities: 40, includeIsolated: false },
      render: { autoRotate: true },
    })
    const onchange = setup(changed)
    await fireEvent.click(screen.getByRole('button', { name: 'Restaurar predeterminados' }))
    expect(onchange).toHaveBeenLastCalledWith(normalizeGraphSettings(undefined))
  })
})
