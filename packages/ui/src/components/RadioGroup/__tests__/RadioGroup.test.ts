import { fireEvent, render, screen } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import RadioGroupTestHost from './RadioGroupTestHost.svelte'

const styles = (file: string) =>
  readFileSync(resolve(import.meta.dirname, '..', file), 'utf-8')
    .split(/<style[^>]*>/)
    .slice(1)
    .join('\n')
    .replace(/\/\*[\s\S]*?\*\//g, '')

const GROUP_CSS = styles('RadioGroup.svelte')
const CARD_CSS = styles('RadioCard.svelte')

function ruleFor(css: string, selector: string): string {
  const at = css.indexOf(`${selector} {`)
  expect(at, `${selector} is not in the stylesheet`).toBeGreaterThan(-1)
  return css.slice(at, css.indexOf('}', at))
}

const radio = (name: string) => screen.getByRole('radio', { name }) as HTMLInputElement

describe('RadioGroup', () => {
  it('is one radiogroup named by its legend', () => {
    render(RadioGroupTestHost)

    const group = screen.getByRole('radiogroup', { name: 'Destino' })
    expect(group.tagName).toBe('FIELDSET')
    expect(group.querySelector('legend')?.textContent?.trim()).toBe('Destino')
  })

  it('keeps a hidden legend for assistive technology', () => {
    render(RadioGroupTestHost, { props: { hideLabel: true } })

    expect(screen.getByRole('radiogroup', { name: 'Destino' })).toBeTruthy()
    expect(document.querySelector('legend')?.className).toContain('radio-group__legend--hidden')
  })

  /** Native radios sharing one name: arrow keys move the choice, with no key handling of our own. */
  it('renders real radios that share one name', () => {
    render(RadioGroupTestHost)

    const radios = screen.getAllByRole('radio') as HTMLInputElement[]
    expect(radios.map((r) => r.type)).toEqual(['radio', 'radio', 'radio'])
    expect(new Set(radios.map((r) => r.name)).size).toBe(1)
    expect(radios.map((r) => r.value)).toEqual(['a', 'b', 'new'])
  })

  it('checks the option matching its value, and only that one', () => {
    render(RadioGroupTestHost, { props: { value: 'b' } })

    expect(radio('Alfa').checked).toBe(false)
    expect(radio('Beta').checked).toBe(true)
  })

  it('moves the value and reports it when an option is chosen', async () => {
    const onchange = vi.fn()
    render(RadioGroupTestHost, { props: { onchange } })

    await fireEvent.click(radio('Beta'))

    expect(onchange).toHaveBeenCalledWith('b')
    expect(screen.getByTestId('value').textContent).toBe('b')
  })

  it('selects an option when its card, not only its radio, is clicked', async () => {
    render(RadioGroupTestHost)

    await fireEvent.click(screen.getByText('Beta'))
    expect(screen.getByTestId('value').textContent).toBe('b')
  })

  it('disables every option, embedded controls included, when disabled', () => {
    render(RadioGroupTestHost, { props: { disabled: true } })

    expect(screen.getByRole('radiogroup')).toBeDisabled()
    expect(radio('Alfa')).toBeDisabled()
    expect(screen.getByRole('textbox', { name: 'Nombre de la colección' })).toBeDisabled()
  })

  it('lays cards out in as many columns as fit, never wider than its box', () => {
    expect(ruleFor(GROUP_CSS, '.radio-group--card .radio-group__options')).toMatch(
      /grid-template-columns:\s*repeat\(\s*auto-fit,\s*minmax\(min\(100%,\s*var\(--radio-group-column-min\)\),\s*1fr\)\s*\)/
    )
  })

  it('takes the narrowest column from its caller', () => {
    render(RadioGroupTestHost, { props: { columnMin: '120px' } })

    const group = screen.getByRole('radiogroup') as HTMLElement
    expect(group.style.getPropertyValue('--radio-group-column-min')).toBe('120px')
  })

  it('stacks options as rows in the list variant', () => {
    render(RadioGroupTestHost, { props: { variant: 'list' } })

    expect(screen.getByRole('radiogroup').className).toContain('radio-group--list')
    expect(document.querySelectorAll('.radio-card--list')).toHaveLength(3)
    expect(ruleFor(GROUP_CSS, '.radio-group--list .radio-group__options')).toMatch(
      /flex-direction:\s*column/
    )
  })
})

describe('RadioCard', () => {
  it('names the radio by its label alone, and describes it by its description', () => {
    render(RadioGroupTestHost)

    const alfa = radio('Alfa')
    const description = document.getElementById(alfa.getAttribute('aria-describedby')!)
    expect(description?.textContent?.trim()).toBe('Primera opción')
  })

  it('shows its extra content without adding it to the name', () => {
    render(RadioGroupTestHost)

    expect(screen.getByTestId('preview-a').closest('.radio-card')).toBeTruthy()
    expect(screen.getByRole('radio', { name: 'Alfa' })).toBeTruthy()
  })

  it('marks the selected option with a glyph, not colour alone', async () => {
    render(RadioGroupTestHost)

    const card = (name: string) => radio(name).closest('.radio-card')!
    expect(card('Alfa').querySelector('.radio-card__check svg')).toBeTruthy()
    expect(card('Beta').querySelector('.radio-card__check svg')).toBeNull()

    await fireEvent.click(radio('Beta'))
    expect(card('Beta').querySelector('.radio-card__check svg')).toBeTruthy()
    expect(card('Alfa').querySelector('.radio-card__check svg')).toBeNull()
  })

  it('does not change while disabled', async () => {
    const onchange = vi.fn()
    render(RadioGroupTestHost, { props: { disableB: true, onchange } })

    expect(radio('Beta')).toBeDisabled()
    await fireEvent.click(radio('Beta'))
    expect(onchange).not.toHaveBeenCalled()
  })

  it('selects its option when the embedded control takes focus', async () => {
    const onchange = vi.fn()
    render(RadioGroupTestHost, { props: { onchange } })

    const field = screen.getByRole('textbox', { name: 'Nombre de la colección' })
    await fireEvent.focusIn(field)

    expect(onchange).toHaveBeenCalledWith('new')
    expect(radio('Nueva colección').checked).toBe(true)
  })

  it('keeps the embedded control out of the radio label', () => {
    render(RadioGroupTestHost)

    const field = screen.getByRole('textbox', { name: 'Nombre de la colección' })
    expect(field.closest('label')).toBeNull()
    expect(field.closest('.radio-card')).toBe(radio('Nueva colección').closest('.radio-card'))
  })

  it('wraps a long label instead of cutting it', () => {
    expect(ruleFor(CARD_CSS, '.radio-card__label')).not.toMatch(/white-space:\s*nowrap/)
  })

  it('hides the native radio visually but keeps it focusable', () => {
    const rule = ruleFor(CARD_CSS, '.radio-card__input')
    expect(rule).toMatch(/clip:\s*rect\(0 0 0 0\)/)
    expect(rule).not.toMatch(/display:\s*none/)
  })

  it('draws one focus ring on the card, from the radio', () => {
    expect(CARD_CSS).toMatch(
      /\.radio-card:has\(\.radio-card__input:focus-visible\)\s*\{[^}]*box-shadow:\s*var\(--focus-ring\)/
    )
  })

  it('dims with the disabled token and stops moving under reduced motion', () => {
    expect(ruleFor(CARD_CSS, '.radio-card--disabled')).toMatch(
      /opacity:\s*var\(--opacity-disabled\)/
    )
    expect(CARD_CSS).toMatch(
      /@media \(prefers-reduced-motion: reduce\)\s*\{[^}]*\.radio-card[^{]*\{\s*transition:\s*none/
    )
  })
})
