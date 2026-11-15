import { fireEvent, render, screen } from '@testing-library/svelte'
import { createRawSnippet } from 'svelte'
import { describe, expect, it, vi } from 'vitest'
import Input from '../Input.svelte'
import InputElementHarness from './InputElementHarness.svelte'

const text = (content: string, className: string) =>
  createRawSnippet(() => ({ render: () => `<span class="${className}">${content}</span>` }))

describe('Input', () => {
  it('keeps the original API: label, value, placeholder, type, hint and error', async () => {
    const { rerender } = render(Input, {
      props: { label: 'Name', value: 'Ada', placeholder: 'Type', type: 'email', hint: 'Help' },
    })

    const field = screen.getByLabelText('Name') as HTMLInputElement
    expect(field.value).toBe('Ada')
    expect(field.placeholder).toBe('Type')
    expect(field.type).toBe('email')
    expect(field).toHaveAccessibleDescription('Help')
    expect(field).toHaveAttribute('aria-invalid', 'false')

    await fireEvent.input(field, { target: { value: 'Grace' } })
    expect(field.value).toBe('Grace')

    await rerender({ error: 'Required' })
    expect(field).toHaveAttribute('aria-invalid', 'true')
    expect(screen.getByRole('alert')).toHaveTextContent('Required')
    expect(field).toHaveAccessibleDescription('Required')
  })

  it('stays disabled when asked', () => {
    render(Input, { props: { label: 'Name', disabled: true } })
    expect(screen.getByLabelText('Name')).toBeDisabled()
  })

  it('passes every other input attribute to the native input', async () => {
    const oninput = vi.fn()
    const onkeydown = vi.fn()
    render(Input, {
      props: {
        label: 'Year',
        name: 'year',
        autocomplete: 'off',
        maxlength: 4,
        required: true,
        'aria-label': 'Publication year',
        'data-testid': 'year-field',
        oninput,
        onkeydown,
      },
    })

    const field = screen.getByTestId('year-field') as HTMLInputElement
    expect(field).toHaveAttribute('name', 'year')
    expect(field).toHaveAttribute('autocomplete', 'off')
    expect(field).toHaveAttribute('maxlength', '4')
    expect(field).toBeRequired()
    expect(field).toHaveAttribute('aria-label', 'Publication year')

    await fireEvent.keyDown(field, { key: 'Enter' })
    expect(onkeydown).toHaveBeenCalledTimes(1)

    // The caller's handler runs and the bound value still follows the input.
    await fireEvent.input(field, { target: { value: '1999' } })
    expect(oninput).toHaveBeenCalledTimes(1)
    expect(field.value).toBe('1999')
  })

  it('labels the input with an id the caller provides', () => {
    render(Input, { props: { label: 'Title', id: 'custom-title' } })
    expect(screen.getByLabelText('Title')).toHaveAttribute('id', 'custom-title')
  })

  it('merges a caller class with its own on the native input', () => {
    render(Input, { props: { label: 'Name', class: 'my-field' } })
    const field = screen.getByLabelText('Name')
    expect(field).toHaveClass('input-field__input')
    expect(field).toHaveClass('my-field')
  })

  it('hands the native input to a bound element', () => {
    const onelement = vi.fn()
    render(InputElementHarness, { props: { onelement } })
    expect(onelement).toHaveBeenLastCalledWith(screen.getByLabelText('Name'))
  })

  it('is medium by default and small when asked', async () => {
    const { container, rerender } = render(Input, { props: { label: 'Name' } })
    const field = container.querySelector('.input-field')
    expect(field).toHaveClass('input-field--md')
    expect(field).not.toHaveClass('input-field--sm')

    await rerender({ size: 'sm' })
    expect(field).toHaveClass('input-field--sm')
    expect(field).not.toHaveClass('input-field--md')
  })

  it('renders leading and trailing content around the input', () => {
    const { container } = render(Input, {
      props: {
        label: 'Amount',
        leading: text('$', 'test-leading'),
        trailing: text('USD', 'test-trailing'),
      },
    })

    const control = container.querySelector('.input-field__control')
    expect(control).not.toBeNull()
    const leading = control?.querySelector('.input-field__leading .test-leading')
    const trailing = control?.querySelector('.input-field__trailing .test-trailing')
    expect(leading).toHaveTextContent('$')
    expect(trailing).toHaveTextContent('USD')
    expect(control?.querySelector('.input-field__input')).toBe(screen.getByLabelText('Amount'))
    expect(control).toHaveClass('input-field__control--leading')
    expect(control).toHaveClass('input-field__control--trailing')
  })

  it('adds no adornment wrappers when there is nothing to put in them', () => {
    const { container } = render(Input, { props: { label: 'Name' } })
    expect(container.querySelector('.input-field__leading')).toBeNull()
    expect(container.querySelector('.input-field__trailing')).toBeNull()
  })

  it('never renders a search box: search fields are always SearchBar', () => {
    // @ts-expect-error -- 'search' is not an accepted Input type.
    render(Input, { props: { label: 'Find', type: 'search' } })
    expect(screen.getByLabelText('Find')).toHaveAttribute('type', 'text')
  })

  it('sets keys, paths and other machine text in the mono face when asked', () => {
    render(Input, { props: { label: 'Key', font: 'mono' } })
    expect(screen.getByLabelText('Key')).toHaveClass('input-field__input--mono')
  })

  it('keeps the interface face by default', () => {
    render(Input, { props: { label: 'Name' } })
    expect(screen.getByLabelText('Name')).not.toHaveClass('input-field__input--mono')
  })
})
