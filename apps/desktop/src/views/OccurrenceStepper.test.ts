import { fireEvent, render, screen } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import OccurrenceStepper from './OccurrenceStepper.svelte'
import { t } from '$lib/i18n'

describe('OccurrenceStepper', () => {
  it('steps through matches with accessible buttons disabled at the ends', async () => {
    const onchange = vi.fn()
    const props = {
      index: 0,
      total: 3,
      kind: 'entity' as const,
      status: 'exact' as const,
      translate: t,
      onchange,
    }
    const { rerender } = render(OccurrenceStepper, props)
    expect(screen.getByText('Aparición 1 de 3')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Anterior' })).toBeDisabled()
    await fireEvent.click(screen.getByRole('button', { name: 'Siguiente' }))
    expect(onchange).toHaveBeenLastCalledWith(1)
    await rerender({ ...props, index: 2 })
    expect(screen.getByRole('button', { name: 'Siguiente' })).toBeDisabled()
    await fireEvent.click(screen.getByRole('button', { name: 'Anterior' }))
    expect(onchange).toHaveBeenLastCalledWith(1)
  })
  it('identifies relation roles and never hides approximate or missing evidence', async () => {
    const props = {
      index: 0,
      total: 1,
      kind: 'relation' as const,
      status: 'approximate' as const,
      translate: t,
      onchange: vi.fn(),
    }
    const { rerender } = render(OccurrenceStepper, props)
    expect(screen.getByText('Sujeto')).toBeInTheDocument()
    expect(screen.getByText('Vínculo')).toBeInTheDocument()
    expect(screen.getByText('Objeto')).toBeInTheDocument()
    expect(screen.getByRole('status')).toHaveTextContent('aproximada')
    await rerender({ ...props, status: 'none' })
    expect(screen.getByRole('status')).toHaveTextContent('No se encontró')
  })
})
