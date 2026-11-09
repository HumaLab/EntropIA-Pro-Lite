import { fireEvent, render, screen, within } from '@testing-library/svelte'
import { beforeEach, describe, expect, it } from 'vitest'
import PhraseAnalysis from './PhraseAnalysis.svelte'
import { locale } from '$lib/i18n'
import type { CorpusText } from '$lib/text-analysis'

const texts: CorpusText[] = [
  {
    kind: 'extraction',
    text:
      'La guerra del Paraguay cambió la economía. La guerra del Paraguay terminó en 1870.\n' +
      'La guerra del Paraguay fue dura.',
  },
]

describe('PhraseAnalysis', () => {
  beforeEach(() => locale.set('es'))

  it('lists frequent phrases and changes their length with the stepper', async () => {
    render(PhraseAnalysis, { props: { texts, customStopwords: [] } })
    const phrases = screen.getByRole('list', { name: 'Frases frecuentes' })
    expect(within(phrases).getByText('guerra del paraguay')).toBeInTheDocument()
    expect(within(phrases).getByText('3')).toBeInTheDocument()

    const length = screen.getByRole('spinbutton', { name: 'Palabras por frase' })
    expect(length).toHaveValue(3)
    await fireEvent.click(screen.getByRole('button', { name: 'Disminuir' }))
    expect(length).toHaveValue(2)
    expect(within(phrases).queryByText('guerra del paraguay')).not.toBeInTheDocument()
  })

  it('shows the collocates and the word tree of a word', async () => {
    render(PhraseAnalysis, { props: { texts, customStopwords: [] } })
    await fireEvent.input(screen.getByLabelText('Palabra'), { target: { value: 'Guerra' } })

    const collocates = screen.getByRole('list', { name: 'Palabras asociadas' })
    expect(within(collocates).getByText('paraguay')).toBeInTheDocument()

    const tree = screen.getByRole('tree', { name: 'Árbol de palabras' })
    expect(within(tree).getByText('guerra')).toBeInTheDocument()
    expect(within(tree).getByText('del')).toBeInTheDocument()

    await fireEvent.click(screen.getByRole('button', { name: 'Antes' }))
    expect(screen.getByRole('button', { name: 'Antes' })).toHaveAttribute('aria-pressed', 'true')
    expect(
      within(screen.getByRole('tree', { name: 'Árbol de palabras' })).getAllByText('la').length
    ).toBeGreaterThan(0)
  })

  it('says so when the word does not occur', async () => {
    render(PhraseAnalysis, { props: { texts, customStopwords: [] } })
    await fireEvent.input(screen.getByLabelText('Palabra'), { target: { value: 'tratado' } })
    expect(screen.getByText('«tratado» no aparece en esta colección.')).toBeInTheDocument()
  })
})
