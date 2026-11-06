/** @vitest-environment jsdom */

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'
import type { ModelInfo } from '$lib/settings'

const { listSchemasMock, saveSchemaMock, testOpenrouterConnectionMock } = vi.hoisted(() => ({
  listSchemasMock: vi.fn(),
  saveSchemaMock: vi.fn(),
  testOpenrouterConnectionMock: vi.fn(),
}))

vi.mock('$lib/extraction-schemas', () => ({
  listSchemas: listSchemasMock,
  saveSchema: saveSchemaMock,
  deleteSchema: vi.fn(),
  listRecords: vi.fn(),
  cellText: vi.fn(() => ''),
  recordsCsv: vi.fn(() => ''),
}))

vi.mock('$lib/settings', async () => {
  const actual = await vi.importActual<typeof import('$lib/settings')>('$lib/settings')
  return {
    ...actual,
    testOpenrouterConnection: testOpenrouterConnectionMock,
  }
})

import BatchSchemaPanel from './BatchSchemaPanel.svelte'

const LUNA = 'openai/gpt-5.6-luna-pro'

function catalog(): ModelInfo[] {
  const filler = Array.from({ length: 20 }, (_, index) => ({
    id: `meta-llama/llama-filler-${index + 1}`,
    name: `Llama Filler ${index + 1}`,
    context_length: 131072,
  }))
  return [
    ...filler,
    { id: 'qwen/qwen-2.5-7b', name: 'Qwen 2.5 7B', context_length: 32768 },
    { id: LUNA, name: 'GPT Luna Pro', context_length: 1048576 },
  ]
}

beforeEach(() => {
  locale.set('es')
  listSchemasMock.mockReset()
  listSchemasMock.mockResolvedValue([])
  saveSchemaMock.mockReset()
  saveSchemaMock.mockImplementation(async (schema: { name: string; model: string }) => ({
    ...schema,
    id: 'schema-1',
  }))
  testOpenrouterConnectionMock.mockReset()
  testOpenrouterConnectionMock.mockResolvedValue(catalog())
})

afterEach(() => {
  cleanup()
})

describe('BatchSchemaPanel model field', () => {
  it('guarda el modelo elegido con el esquema', async () => {
    render(BatchSchemaPanel)

    await fireEvent.click(screen.getByRole('button', { name: /Nuevo esquema/ }))

    const model = screen.getByRole('combobox', { name: 'Modelo' }) as HTMLInputElement
    await waitFor(() => expect(testOpenrouterConnectionMock).toHaveBeenCalled())

    await fireEvent.focus(model)
    await fireEvent.input(model, { target: { value: 'luna' } })
    await fireEvent.keyDown(model, { key: 'Enter' })
    expect(model.value).toBe(LUNA)

    await fireEvent.input(screen.getByRole('textbox', { name: 'Nombre del esquema' }), {
      target: { value: 'Ficha de barco' },
    })
    await fireEvent.click(screen.getByRole('button', { name: /Guardar esquema/ }))

    await waitFor(() => expect(saveSchemaMock).toHaveBeenCalledTimes(1))
    expect(saveSchemaMock).toHaveBeenCalledWith(
      expect.objectContaining({ name: 'Ficha de barco', model: LUNA })
    )
  })
})
