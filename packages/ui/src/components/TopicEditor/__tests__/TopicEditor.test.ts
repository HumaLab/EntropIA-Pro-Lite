import { fireEvent, render, screen } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import TopicEditor from '../TopicEditor.svelte'
import topicEditorSource from '../TopicEditor.svelte?raw'

describe('TopicEditor', () => {
  it('renders remove-topic controls with accessible names and no textual multiplication sign', () => {
    render(TopicEditor, {
      props: {
        topics: ['ARCHIVE'],
      },
    })

    const removeButton = screen.getByRole('button', { name: 'Quitar tópico ARCHIVE' })
    expect(removeButton).toBeInTheDocument()
    expect(removeButton).not.toHaveTextContent('×')
  })

  it('removes a topic when its icon button is clicked', async () => {
    const onchange = vi.fn()
    render(TopicEditor, {
      props: {
        topics: ['ARCHIVE', 'LETTER'],
        onchange,
      },
    })

    await fireEvent.click(screen.getByRole('button', { name: 'Quitar tópico ARCHIVE' }))

    expect(onchange).toHaveBeenCalledWith(['LETTER'])
  })

  it('renders Spanish placeholders for empty and populated topic input', async () => {
    const { rerender } = render(TopicEditor, {
      props: {
        topics: [],
      },
    })

    expect(screen.getByPlaceholderText('Escribí un tópico...')).toBeInTheDocument()

    await rerender({ topics: ['ARCHIVE'] })

    expect(screen.getByPlaceholderText('Agregar más...')).toBeInTheDocument()
  })

  it('draws each topic as a neutral StatusBadge, never an accent fill', () => {
    render(TopicEditor, { props: { topics: ['ARCHIVE'] } })

    const remove = screen.getByRole('button', { name: 'Quitar tópico ARCHIVE' })
    const chip = remove.closest('.topic-editor__chip')
    expect(chip).toHaveClass('status-badge', 'status-badge--neutral')
  })

  it('marks a hovered suggestion by contrast and takes layers and motion from tokens', () => {
    const styles = topicEditorSource.slice(topicEditorSource.indexOf('<style>'))
    expect(styles).not.toMatch(/background(-color)?:\s*var\(--color-accent\)/)
    expect(styles).not.toMatch(/z-index:\s*\d/)
    expect(styles).not.toMatch(/transition:[^;]*\d+(\.\d+)?m?s\b/)
    expect(styles).not.toMatch(/line-height:\s*\d/)
    expect(styles).toMatch(
      /\.topic-editor__chip-remove:focus-visible\s*\{[^}]*var\(--focus-ring\)/s
    )
  })
})
