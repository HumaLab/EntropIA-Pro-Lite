import { render, screen, fireEvent } from '@testing-library/svelte'
import { describe, it, expect, vi } from 'vitest'
import ItemCard from '../ItemCard.svelte'

describe('ItemCard', () => {
  const baseProps = {
    id: 'item-1',
    title: 'Test Document',
    assetCount: 3,
  }

  it('renders the item title', () => {
    render(ItemCard, { props: baseProps })
    expect(screen.getByText('Test Document')).toBeInTheDocument()
  })

  it('renders the asset count chip', () => {
    render(ItemCard, { props: baseProps })
    expect(screen.getByText('3 assets')).toBeInTheDocument()
  })

  it('renders singular "asset" for count of 1', () => {
    render(ItemCard, { props: { ...baseProps, assetCount: 1 } })
    expect(screen.getByText('1 asset')).toBeInTheDocument()
  })

  it('renders thumbnail when thumbnailPath is provided', () => {
    render(ItemCard, {
      props: { ...baseProps, thumbnailPath: 'asset://localhost/path/to/thumb.jpg' },
    })
    const img = screen.getByRole('img')
    expect(img).toBeInTheDocument()
    expect(img).toHaveAttribute('src', 'asset://localhost/path/to/thumb.jpg')
    expect(img).toHaveAttribute('loading', 'lazy')
    expect(img).toHaveAttribute('decoding', 'async')
  })

  it('renders placeholder icon when no thumbnail provided', () => {
    render(ItemCard, { props: baseProps })
    expect(screen.queryByRole('img')).not.toBeInTheDocument()
    expect(screen.getByTestId('item-placeholder')).toBeInTheDocument()
  })

  it('renders audio play icon when primaryAssetType is audio', () => {
    render(ItemCard, {
      props: { ...baseProps, primaryAssetType: 'audio' },
    })
    expect(screen.queryByRole('img')).not.toBeInTheDocument()
    expect(screen.queryByTestId('item-placeholder')).not.toBeInTheDocument()
    expect(screen.getByTestId('item-audio')).toBeInTheDocument()
  })

  it('renders audio play icon even when thumbnailPath is provided for audio', () => {
    render(ItemCard, {
      props: {
        ...baseProps,
        primaryAssetType: 'audio',
        thumbnailPath: 'asset://localhost/audio.mp3',
      },
    })
    expect(screen.queryByRole('img')).not.toBeInTheDocument()
    expect(screen.getByTestId('item-audio')).toBeInTheDocument()
  })

  it('renders PDF icon when primaryAssetType is pdf and no thumbnailPath', () => {
    render(ItemCard, {
      props: { ...baseProps, primaryAssetType: 'pdf' },
    })
    expect(screen.queryByRole('img')).not.toBeInTheDocument()
    expect(screen.getByTestId('item-pdf-icon')).toBeInTheDocument()
    expect(screen.queryByTestId('item-placeholder')).not.toBeInTheDocument()
  })

  it('renders image thumbnail when primaryAssetType is pdf with thumbnailPath', () => {
    render(ItemCard, {
      props: {
        ...baseProps,
        primaryAssetType: 'pdf',
        thumbnailPath: 'asset://localhost/thumb.png',
      },
    })
    const img = screen.getByRole('img')
    expect(img).toBeInTheDocument()
    expect(img).toHaveAttribute('src', 'asset://localhost/thumb.png')
    expect(screen.queryByTestId('item-pdf-icon')).not.toBeInTheDocument()
  })

  it('renders placeholder icon when primaryAssetType is image but no thumbnailPath', () => {
    render(ItemCard, {
      props: { ...baseProps, primaryAssetType: 'image' },
    })
    expect(screen.queryByRole('img')).not.toBeInTheDocument()
    expect(screen.getByTestId('item-placeholder')).toBeInTheDocument()
    expect(screen.queryByTestId('item-pdf-icon')).not.toBeInTheDocument()
  })

  it('renders image thumbnail when primaryAssetType is image with thumbnailPath', () => {
    render(ItemCard, {
      props: {
        ...baseProps,
        primaryAssetType: 'image',
        thumbnailPath: 'asset://localhost/thumb.jpg',
      },
    })
    const img = screen.getByRole('img')
    expect(img).toBeInTheDocument()
    expect(screen.queryByTestId('item-audio')).not.toBeInTheDocument()
  })

  it('renders metadata preview when provided', () => {
    render(ItemCard, {
      props: { ...baseProps, metadataPreview: 'Author: John Doe' },
    })
    expect(screen.getByText('Author: John Doe')).toBeInTheDocument()
  })

  it('calls onclick when the main card area is clicked', async () => {
    const onclick = vi.fn()
    render(ItemCard, { props: { ...baseProps, onclick } })
    // The main card button contains the content, click on the title area
    const title = screen.getByText('Test Document')
    await fireEvent.click(title)
    expect(onclick).toHaveBeenCalledOnce()
  })

  describe('delete button', () => {
    it('does not render delete button when onDelete is not provided', () => {
      render(ItemCard, { props: baseProps })
      expect(screen.queryByRole('button', { name: /delete/i })).not.toBeInTheDocument()
    })

    it('renders delete button with aria-label when onDelete is provided', () => {
      const onDelete = vi.fn()
      render(ItemCard, { props: { ...baseProps, onDelete } })
      const deleteBtn = screen.getByRole('button', { name: 'Delete Test Document' })
      expect(deleteBtn).toBeInTheDocument()
    })

    it('calls onDelete when delete button is clicked without triggering onclick', async () => {
      const onclick = vi.fn()
      const onDelete = vi.fn()
      render(ItemCard, { props: { ...baseProps, onclick, onDelete } })

      const deleteBtn = screen.getByRole('button', { name: 'Delete Test Document' })
      await fireEvent.click(deleteBtn)

      expect(onDelete).toHaveBeenCalledOnce()
      expect(onclick).not.toHaveBeenCalled()
    })

    it('calls onclick when main card is clicked (not delete button)', async () => {
      const onclick = vi.fn()
      const onDelete = vi.fn()
      render(ItemCard, { props: { ...baseProps, onclick, onDelete } })

      const title = screen.getByText('Test Document')
      await fireEvent.click(title)

      expect(onclick).toHaveBeenCalledOnce()
      expect(onDelete).not.toHaveBeenCalled()
    })
  })
})
