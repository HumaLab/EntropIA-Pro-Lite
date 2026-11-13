import { fireEvent, render, screen } from '@testing-library/svelte'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import HelpSettingsTab, {
  defaultHelpParams,
  type EditableHelpParams,
} from './HelpSettingsTab.svelte'
import { locale } from '$lib/i18n'

function renderTab(overrides: Partial<EditableHelpParams> = {}) {
  const form: EditableHelpParams = { ...defaultHelpParams('vendor/general-model'), ...overrides }
  const onrestore = vi.fn()
  render(HelpSettingsTab, {
    props: { form, inheritedModel: 'vendor/general-model', error: null, onrestore },
  })
  return { form, onrestore }
}

describe('HelpSettingsTab (S14)', () => {
  beforeEach(() => {
    locale.set('es')
  })

  it('renders the model, budget and local-answer controls with their defaults', () => {
    renderTab()

    expect(screen.getByRole('heading', { name: 'Ayuda' })).toBeInTheDocument()
    expect(screen.getByLabelText('model')).toHaveValue('vendor/general-model')
    expect(screen.getByLabelText('model')).toHaveAttribute('placeholder', 'vendor/general-model')
    expect(
      screen.getByText('Modelo de la ayuda: vacío usa el modelo general de Configuración')
    ).toBeInTheDocument()
    expect(screen.getByLabelText('contextChars (4000-60000)')).toHaveValue(24000)
    expect(screen.getByLabelText('maxTokens (200-8000)')).toHaveValue(1500)
    expect(screen.getByLabelText('temperature (0-1)')).toHaveValue(0.2)
    expect(screen.getByLabelText('historyTurns (0-12)')).toHaveValue(6)
    expect(screen.getByRole('checkbox', { name: 'Respuestas locales' })).toBeChecked()
    expect(
      screen.getByText(
        'Las preguntas que van al modelo se cobran en tu cuenta de OpenRouter. El diagnóstico y tus documentos solo viajan si los activás en cada pregunta.'
      )
    ).toBeInTheDocument()
  })

  it('shows stored values instead of the defaults', () => {
    renderTab({
      model: 'vendor/help-model',
      contextChars: '12000',
      maxTokens: '800',
      temperature: '0.7',
      historyTurns: '2',
      localAnswers: false,
    })

    expect(screen.getByLabelText('model')).toHaveValue('vendor/help-model')
    expect(screen.getByLabelText('contextChars (4000-60000)')).toHaveValue(12000)
    expect(screen.getByLabelText('maxTokens (200-8000)')).toHaveValue(800)
    expect(screen.getByLabelText('temperature (0-1)')).toHaveValue(0.7)
    expect(screen.getByLabelText('historyTurns (0-12)')).toHaveValue(2)
    expect(screen.getByRole('checkbox', { name: 'Respuestas locales' })).not.toBeChecked()
  })

  it('writes edits back to the bound form', async () => {
    const { form } = renderTab()

    await fireEvent.input(screen.getByLabelText('model'), {
      target: { value: 'vendor/other-model' },
    })
    await fireEvent.input(screen.getByLabelText('contextChars (4000-60000)'), {
      target: { value: '8000' },
    })
    await fireEvent.click(screen.getByRole('checkbox', { name: 'Respuestas locales' }))

    expect(form.model).toBe('vendor/other-model')
    // NumberField escribe el valor nativo del input (número); el padre lo
    // fija a texto al validar y guardar.
    expect(form.contextChars).toBe(8000)
    expect(form.localAnswers).toBe(false)
  })

  it('asks the parent to restore defaults', async () => {
    const { onrestore } = renderTab()

    await fireEvent.click(screen.getByRole('button', { name: 'Restaurar defaults' }))

    expect(onrestore).toHaveBeenCalledOnce()
  })

  it('shows the validation error the parent reports', () => {
    const form = defaultHelpParams('vendor/general-model')
    render(HelpSettingsTab, {
      props: {
        form,
        inheritedModel: 'vendor/general-model',
        error: 'Parámetro de ayuda inválido: contextChars',
        onrestore: vi.fn(),
      },
    })

    expect(screen.getByRole('alert')).toHaveTextContent('Parámetro de ayuda inválido: contextChars')
  })
})
