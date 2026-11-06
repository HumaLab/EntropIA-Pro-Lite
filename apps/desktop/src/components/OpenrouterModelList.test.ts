/** @vitest-environment jsdom */

import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { locale } from '$lib/i18n'
import type { ModelInfo } from '$lib/settings'

const { testOpenrouterConnectionMock, navigateActiveMock, requestSettingsTabMock } = vi.hoisted(
  () => ({
    testOpenrouterConnectionMock: vi.fn(),
    navigateActiveMock: vi.fn(),
    requestSettingsTabMock: vi.fn(),
  })
)

vi.mock('$lib/settings', async () => {
  const actual = await vi.importActual<typeof import('$lib/settings')>('$lib/settings')
  return {
    ...actual,
    testOpenrouterConnection: testOpenrouterConnectionMock,
  }
})

vi.mock('$lib/settings-tab-request', () => ({
  requestSettingsTab: requestSettingsTabMock,
}))

vi.mock('$lib/workspace', () => ({
  workspace: { navigateActive: navigateActiveMock },
}))

import OpenrouterModelList from './OpenrouterModelList.svelte'

const EMPTY_LABEL = 'google/gemma-4-26b-a4b-it'
const LUNA = 'openai/gpt-5.6-luna-pro'
const LUNA_MINI = 'openai/gpt-5.6-luna-mini'

/**
 * A catalog shaped like the real one: long enough that a field showing only
 * the first fifteen never reaches `luna`, and with a model whose name -- not
 * its id -- is what the reader recognises.
 */
function catalog(): ModelInfo[] {
  const filler = Array.from({ length: 20 }, (_, index) => ({
    id: `meta-llama/llama-filler-${index + 1}`,
    name: `Llama Filler ${index + 1}`,
    context_length: 131072,
  }))
  return [
    ...filler,
    { id: 'openrouter/deepseek-r1', name: 'DeepSeek R1', context_length: 65536 },
    { id: LUNA, name: 'GPT Luna Pro', context_length: 1048576 },
    { id: LUNA_MINI, name: 'GPT Luna Mini', context_length: 262144 },
    { id: 'qwen/qwen-2.5-7b', name: 'Qwen 2.5 7B', context_length: 32768 },
  ]
}

function picker(selected = '', onchoose = vi.fn()) {
  render(OpenrouterModelList, {
    props: { selected, onchoose, label: 'Modelo', emptyLabel: EMPTY_LABEL },
  })
  return { onchoose }
}

function box(): HTMLInputElement {
  return screen.getByRole('combobox', { name: 'Modelo' }) as HTMLInputElement
}

function optionNames(): string[] {
  return screen.getAllByRole('option').map((option) => option.textContent?.trim() ?? '')
}

async function type(value: string) {
  await fireEvent.input(box(), { target: { value } })
}

async function loaded() {
  await waitFor(() => expect(box()).toBeInTheDocument())
}

beforeEach(() => {
  locale.set('es')
  testOpenrouterConnectionMock.mockReset()
  testOpenrouterConnectionMock.mockResolvedValue(catalog())
  navigateActiveMock.mockReset()
  requestSettingsTabMock.mockReset()
})

afterEach(() => {
  cleanup()
})

describe('OpenrouterModelList', () => {
  it('muestra el modelo elegido en un campo de texto y la etiqueta del campo', async () => {
    picker('qwen/qwen-2.5-7b')
    await loaded()

    expect(screen.getByText('Modelo')).toBeInTheDocument()
    expect(box().value).toBe('qwen/qwen-2.5-7b')
  })

  it('busca en todo el catálogo y Enter elige el primer resultado', async () => {
    const { onchoose } = picker()
    await loaded()

    await type('luna')
    await waitFor(() => {
      expect(optionNames()).toEqual([`${LUNA} · 1024k`, `${LUNA_MINI} · 256k`])
    })

    await fireEvent.keyDown(box(), { key: 'Enter' })

    expect(onchoose).toHaveBeenCalledTimes(1)
    expect(onchoose).toHaveBeenCalledWith(LUNA)
    expect(box().value).toBe(LUNA)
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
  })

  it('busca también por el nombre del modelo', async () => {
    picker()
    await loaded()

    await type('deepseek')
    await waitFor(() => {
      expect(optionNames()).toEqual(['openrouter/deepseek-r1 · 64k'])
    })
  })

  it('no corta los resultados de la búsqueda en quince', async () => {
    picker()
    await loaded()

    await type('llama')

    expect(screen.getAllByRole('option')).toHaveLength(20)
    expect(screen.queryByText(/más: seguí escribiendo/)).not.toBeInTheDocument()
  })

  it('muestra cuántos resultados quedan fuera de los cincuenta', async () => {
    const many = Array.from({ length: 60 }, (_, index) => ({
      id: `qwen/qwen-filler-${index + 1}`,
      name: `Qwen Filler ${index + 1}`,
      context_length: 32768,
    }))
    testOpenrouterConnectionMock.mockResolvedValue(many)
    picker()
    await loaded()

    await type('qwen')

    expect(screen.getAllByRole('option')).toHaveLength(50)
    expect(screen.getByText('10 más: seguí escribiendo para acotar')).toBeInTheDocument()
  })

  it('un id escrito que no está en el catálogo se acepta y se avisa', async () => {
    const { onchoose } = picker()
    await loaded()

    await type('acme/fantasma-1')
    await fireEvent.keyDown(box(), { key: 'Enter' })

    expect(onchoose).toHaveBeenCalledTimes(1)
    expect(onchoose).toHaveBeenCalledWith('acme/fantasma-1')
    expect(box().value).toBe('acme/fantasma-1')
    expect(
      screen.getByText('No está en el catálogo de OpenRouter: se valida al crear.')
    ).toBeInTheDocument()
  })

  it('Escape cierra la lista y restaura el valor anterior', async () => {
    const { onchoose } = picker('qwen/qwen-2.5-7b')
    await loaded()

    await type('luna')
    await waitFor(() => expect(screen.getByRole('listbox')).toBeInTheDocument())

    await fireEvent.keyDown(box(), { key: 'Escape' })

    expect(box().value).toBe('qwen/qwen-2.5-7b')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(onchoose).not.toHaveBeenCalled()
  })

  it('las flechas mueven el resaltado y Enter elige el segundo resultado', async () => {
    const { onchoose } = picker()
    await loaded()

    await type('luna')
    await waitFor(() => expect(screen.getAllByRole('option')).toHaveLength(2))
    expect(screen.getAllByRole('option')[0]).toHaveAttribute('aria-selected', 'true')

    await fireEvent.keyDown(box(), { key: 'ArrowDown' })
    expect(screen.getAllByRole('option')[1]).toHaveAttribute('aria-selected', 'true')

    await fireEvent.keyDown(box(), { key: 'Enter' })
    expect(onchoose).toHaveBeenCalledWith(LUNA_MINI)
  })

  it('con la consulta vacía ofrece el modelo general y las sugerencias de siempre', async () => {
    picker()
    await loaded()

    await fireEvent.focus(box())

    const names = optionNames()
    expect(names[0]).toBe(EMPTY_LABEL)
    expect(names).toContain('meta-llama/llama-filler-1 · 128k')
    // Un modelo ajeno a la lista de palabras clave no es una sugerencia.
    expect(names.some((name) => name.includes('deepseek'))).toBe(false)
  })

  it('no pierde el modelo elegido cuando queda fuera de las sugerencias', async () => {
    picker('openrouter/deepseek-r1')
    await loaded()

    await fireEvent.focus(box())

    await waitFor(() => {
      expect(optionNames()).toContain('openrouter/deepseek-r1 · 64k')
    })
    expect(optionNames()[0]).toBe(EMPTY_LABEL)
  })

  it('borrar el texto y Enter elige la opción vacía', async () => {
    const { onchoose } = picker('qwen/qwen-2.5-7b')
    await loaded()

    await type('')
    await fireEvent.keyDown(box(), { key: 'Enter' })

    expect(onchoose).toHaveBeenCalledTimes(1)
    expect(onchoose).toHaveBeenCalledWith('')
  })

  it('elegir una opción con el mouse no le quita el foco al campo', async () => {
    const { onchoose } = picker()
    await loaded()

    await fireEvent.focus(box())
    await type('qwen')
    const option = await screen.findByRole('option', { name: /qwen\/qwen-2.5-7b/ })

    await fireEvent.mouseDown(option)
    await fireEvent.click(option)

    expect(onchoose).toHaveBeenCalledTimes(1)
    expect(onchoose).toHaveBeenCalledWith('qwen/qwen-2.5-7b')
  })

  it('Tab y blur confirman lo que se escribió', async () => {
    const { onchoose } = picker()
    await loaded()

    await type('acme/por-tab')
    await fireEvent.keyDown(box(), { key: 'Tab' })
    expect(onchoose).toHaveBeenCalledWith('acme/por-tab')

    await type('acme/por-blur')
    await fireEvent.blur(box())
    expect(onchoose).toHaveBeenLastCalledWith('acme/por-blur')
    expect(onchoose).toHaveBeenCalledTimes(2)
  })

  it('sin clave muestra la nota con el enlace a Configuración', async () => {
    testOpenrouterConnectionMock.mockRejectedValue(
      new Error('No protected credential found for openrouter_api_key')
    )
    picker()

    await waitFor(() => {
      expect(screen.getByText(/primero cargá la clave de OpenRouter/)).toBeInTheDocument()
    })
    expect(screen.queryByRole('combobox')).not.toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Ir a Configuración' }))
    expect(requestSettingsTabMock).toHaveBeenCalledWith('api')
    expect(navigateActiveMock).toHaveBeenCalledWith({ name: 'settings' })
  })

  it('si el catálogo falla por otra razón el campo sigue aceptando texto', async () => {
    testOpenrouterConnectionMock.mockRejectedValue(new Error('network unreachable'))
    const { onchoose } = picker()
    await loaded()

    await type('acme/escrito-a-mano')
    await fireEvent.keyDown(box(), { key: 'Enter' })

    expect(onchoose).toHaveBeenCalledWith('acme/escrito-a-mano')
    expect(screen.queryByText(/No está en el catálogo/)).not.toBeInTheDocument()
  })
})
