import { render, screen, fireEvent } from '@testing-library/svelte'
import { describe, expect, it } from 'vitest'
import { tick } from 'svelte'
import Textarea from '../Textarea.svelte'

describe('Textarea', () => {
  it('labels the field and describes it with its hint', () => {
    render(Textarea, { props: { label: 'Notes', hint: 'Markdown allowed' } })

    const field = screen.getByRole('textbox', { name: 'Notes' })
    expect(field.tagName).toBe('TEXTAREA')
    expect(field).toHaveAttribute('aria-invalid', 'false')
    const hint = screen.getByText('Markdown allowed')
    expect(field.getAttribute('aria-describedby')).toBe(hint.id)
  })

  it('shows the error instead of the hint and marks the field invalid', () => {
    render(Textarea, { props: { label: 'Notes', hint: 'Markdown allowed', error: 'Required' } })

    const field = screen.getByRole('textbox', { name: 'Notes' })
    expect(field).toHaveAttribute('aria-invalid', 'true')
    const error = screen.getByRole('alert')
    expect(error).toHaveTextContent('Required')
    expect(field.getAttribute('aria-describedby')).toBe(error.id)
    expect(screen.queryByText('Markdown allowed')).toBeNull()
  })

  it('has no description without a hint or an error', () => {
    render(Textarea, { props: { label: 'Notes' } })
    expect(screen.getByRole('textbox')).not.toHaveAttribute('aria-describedby')
  })

  it('shows its value and reports what the user types', async () => {
    render(Textarea, { props: { label: 'Notes', value: 'first' } })
    const field = screen.getByRole('textbox') as HTMLTextAreaElement
    expect(field.value).toBe('first')

    await fireEvent.input(field, { target: { value: 'second' } })
    expect(field.value).toBe('second')
  })

  it('binds its value both ways', async () => {
    let bound = 'start'
    const { rerender } = render(Textarea, {
      props: {
        label: 'Notes',
        get value() {
          return bound
        },
        set value(next: string) {
          bound = next
        },
      },
    })
    const field = screen.getByRole('textbox') as HTMLTextAreaElement
    await fireEvent.input(field, { target: { value: 'typed' } })
    expect(bound).toBe('typed')

    await rerender({ value: 'from parent' })
    expect(field.value).toBe('from parent')
  })

  it('passes rows and the remaining attributes to the native textarea', () => {
    render(Textarea, {
      props: {
        label: 'Notes',
        rows: 5,
        placeholder: 'Write here',
        maxlength: 200,
        name: 'notes',
        'data-testid': 'notes-field',
        class: 'extra',
      },
    })
    const field = screen.getByTestId('notes-field')
    expect(field).toHaveAttribute('rows', '5')
    expect(field).toHaveAttribute('placeholder', 'Write here')
    expect(field).toHaveAttribute('maxlength', '200')
    expect(field).toHaveAttribute('name', 'notes')
    expect(field).toHaveClass('textarea-field__control', 'extra')
  })

  it('defaults to three rows', () => {
    render(Textarea, { props: { label: 'Notes' } })
    expect(screen.getByRole('textbox')).toHaveAttribute('rows', '3')
  })

  it('cannot be edited while disabled', () => {
    const { container } = render(Textarea, { props: { label: 'Notes', disabled: true } })
    expect(screen.getByRole('textbox')).toBeDisabled()
    expect(container.querySelector('.textarea-field')).toHaveClass('textarea-field--disabled')
  })

  it.each([
    [undefined, 'textarea-field__control--ui'],
    ['ui', 'textarea-field__control--ui'],
    ['reading', 'textarea-field__control--reading'],
    ['mono', 'textarea-field__control--mono'],
  ] as const)('font %s sets the %s class', (font, expected) => {
    render(Textarea, { props: { label: 'Notes', ...(font ? { font } : {}) } })
    const field = screen.getByRole('textbox')
    expect(field).toHaveClass(expected)
    for (const other of ['ui', 'reading', 'mono']) {
      if (`textarea-field__control--${other}` !== expected) {
        expect(field).not.toHaveClass(`textarea-field__control--${other}`)
      }
    }
  })

  it('exposes the native textarea through bind:this', () => {
    let element: HTMLTextAreaElement | undefined
    render(Textarea, {
      props: {
        label: 'Notes',
        get element() {
          return element
        },
        set element(next: HTMLTextAreaElement | undefined) {
          element = next
        },
      },
    })
    expect(element).toBe(screen.getByRole('textbox'))
  })

  it('grows with its content when autosize is on', async () => {
    render(Textarea, { props: { label: 'Notes', autosize: true } })
    const field = screen.getByRole('textbox') as HTMLTextAreaElement
    Object.defineProperty(field, 'scrollHeight', { configurable: true, value: 140 })

    await fireEvent.input(field, { target: { value: 'line\nline\nline\nline' } })
    await tick()
    expect(field.style.height).toBe('140px')
  })

  it('stops growing at maxRows and scrolls instead', async () => {
    render(Textarea, { props: { label: 'Notes', autosize: true, maxRows: 4 } })
    const field = screen.getByRole('textbox') as HTMLTextAreaElement
    field.style.lineHeight = '20px'
    Object.defineProperty(field, 'scrollHeight', { configurable: true, value: 400 })

    await fireEvent.input(field, { target: { value: 'long text' } })
    await tick()
    expect(field.style.height).toBe('80px')
    expect(field.style.overflowY).toBe('auto')
  })

  it('leaves its height alone without autosize', async () => {
    render(Textarea, { props: { label: 'Notes' } })
    const field = screen.getByRole('textbox') as HTMLTextAreaElement
    Object.defineProperty(field, 'scrollHeight', { configurable: true, value: 140 })

    await fireEvent.input(field, { target: { value: 'text' } })
    await tick()
    expect(field.style.height).toBe('')
  })
})
