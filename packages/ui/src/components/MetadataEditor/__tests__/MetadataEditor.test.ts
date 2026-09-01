import { render, screen, fireEvent } from '@testing-library/svelte'
import { describe, it, expect, vi } from 'vitest'
import MetadataEditor from '../MetadataEditor.svelte'

describe('MetadataEditor', () => {
  it('renders existing key-value fields', () => {
    render(MetadataEditor, {
      props: {
        value: { Author: 'John', Year: '2024' },
        labels: { fieldLabel: 'Field', valueLabel: 'Value' },
      },
    })
    expect(screen.getByText('Field')).toBeInTheDocument()
    expect(screen.getByText('Value')).toBeInTheDocument()
    const inputs = screen.getAllByRole('textbox')
    // 2 pairs = 4 inputs (key + value each)
    expect(inputs).toHaveLength(4)
    expect(inputs[0]).toHaveValue('Author')
    expect(inputs[1]).toHaveValue('John')
    expect(inputs[2]).toHaveValue('Year')
    expect(inputs[3]).toHaveValue('2024')
  })

  it('hydrates initial rows from incoming value prop', () => {
    render(MetadataEditor, {
      props: { value: { Title: 'Documento', Source: 'Notebook' } },
    })

    const inputs = screen.getAllByRole('textbox')
    expect(inputs).toHaveLength(4)
    expect(inputs[0]).toHaveValue('Title')
    expect(inputs[1]).toHaveValue('Documento')
    expect(inputs[2]).toHaveValue('Source')
    expect(inputs[3]).toHaveValue('Notebook')
  })

  it('preserves in-progress local edits across unrelated rerender', async () => {
    const view = render(MetadataEditor, {
      props: { value: { Author: 'John' } },
    })

    let inputs = screen.getAllByRole('textbox')
    await fireEvent.input(inputs[1]!, { target: { value: 'Jane local edit' } })

    await view.rerender({ value: { Author: 'John' }, onchange: vi.fn() })
    inputs = screen.getAllByRole('textbox')
    expect(inputs[1]).toHaveValue('Jane local edit')
  })

  it('re-syncs rows when parent sends a new metadata object', async () => {
    const view = render(MetadataEditor, {
      props: { value: { Author: 'John' } },
    })

    await view.rerender({ value: { Reviewer: 'Ana' } })

    const inputs = screen.getAllByRole('textbox')
    expect(inputs).toHaveLength(2)
    expect(inputs[0]).toHaveValue('Reviewer')
    expect(inputs[1]).toHaveValue('Ana')
  })

  it('renders empty state with no fields', () => {
    render(MetadataEditor, { props: { labels: { emptyText: 'No metadata loaded.' } } })
    expect(screen.getByText('No metadata loaded.')).toBeInTheDocument()
    const inputs = screen.queryAllByRole('textbox')
    expect(inputs).toHaveLength(0)
  })

  it('shows the add action as an icon with the label as its tooltip', () => {
    render(MetadataEditor, { props: { labels: { addField: 'Agregar campo' } } })
    const addBtn = screen.getByTestId('metadata-add')

    expect(addBtn.textContent?.trim()).toBe('')
    expect(addBtn.querySelector('svg')).not.toBeNull()
    expect(addBtn).toHaveAttribute('title', 'Agregar campo')
    expect(addBtn).toHaveAccessibleName('Agregar campo')
  })

  it('add button creates a new empty row', async () => {
    render(MetadataEditor, { props: {} })
    const addBtn = screen.getByTestId('metadata-add')
    await fireEvent.click(addBtn)

    const inputs = screen.getAllByRole('textbox')
    expect(inputs).toHaveLength(2) // key + value
    expect(inputs[0]).toHaveValue('')
    expect(inputs[1]).toHaveValue('')
  })

  it('delete button removes a row', async () => {
    const onchange = vi.fn()
    render(MetadataEditor, {
      props: {
        value: { Author: 'John', Year: '2024' },
        onchange,
      },
    })

    const deleteBtns = screen.getAllByTestId('metadata-delete')
    expect(deleteBtns).toHaveLength(2)
    expect(screen.getAllByRole('button', { name: 'Remove field' })[0]).not.toHaveTextContent('×')

    await fireEvent.click(deleteBtns[0]!)

    const inputs = screen.getAllByRole('textbox')
    expect(inputs).toHaveLength(2) // Only Year remains
    expect(inputs[0]).toHaveValue('Year')
    expect(inputs[1]).toHaveValue('2024')
  })

  it('emits onchange when a value is edited', async () => {
    const onchange = vi.fn()
    render(MetadataEditor, {
      props: {
        value: { Author: 'John' },
        onchange,
      },
    })

    const inputs = screen.getAllByRole('textbox')
    await fireEvent.input(inputs[1]!, { target: { value: 'Jane' } })

    expect(onchange).toHaveBeenCalledWith({ Author: 'Jane' })
  })

  it('emits onchange when a key is edited', async () => {
    const onchange = vi.fn()
    render(MetadataEditor, {
      props: {
        value: { Author: 'John' },
        onchange,
      },
    })

    const inputs = screen.getAllByRole('textbox')
    await fireEvent.input(inputs[0]!, { target: { value: 'Writer' } })

    expect(onchange).toHaveBeenCalledWith({ Writer: 'John' })
  })

  it('emits onchange when delete is clicked', async () => {
    const onchange = vi.fn()
    render(MetadataEditor, {
      props: {
        value: { Author: 'John', Year: '2024' },
        onchange,
      },
    })

    const deleteBtns = screen.getAllByTestId('metadata-delete')
    await fireEvent.click(deleteBtns[0]!)

    expect(onchange).toHaveBeenCalledWith({ Year: '2024' })
  })

  it('renders icon-only delete buttons without textual multiplication signs', () => {
    render(MetadataEditor, {
      props: {
        value: { Author: 'John' },
      },
    })

    expect(screen.getByRole('button', { name: 'Remove field' })).not.toHaveTextContent('×')
  })
})
