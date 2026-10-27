import { render, screen, waitFor } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import BibliographyAnnotationsPanel from './BibliographyAnnotationsPanel.svelte'
import type { AnnotationView } from '$lib/bibliography-annotations'

function area(over: Partial<AnnotationView> = {}): AnnotationView {
  return {
    key: 'AREA2345',
    parentKey: 'ATT1',
    data: {
      annotationType: 'image',
      annotationColor: '#2ea8e5',
      annotationPosition: '{"pageIndex":0,"rects":[[10,20,110,120]]}',
    },
    syncState: 'synced',
    conflictFields: [],
    conflictRemote: null,
    remoteTrashed: false,
    lastError: null,
    updatedAt: 7,
    ...over,
  }
}

describe('BibliographyAnnotationsPanel', () => {
  it('shows a preview of an area annotation, rendered once per revision', async () => {
    const renderThumbnail = vi.fn(async () => 'data:image/png;base64,AAA')
    render(BibliographyAnnotationsPanel, { props: { annotations: [area()], renderThumbnail } })

    const preview = await screen.findByTestId('annotation-preview')
    expect(preview.getAttribute('src')).toBe('data:image/png;base64,AAA')
    expect(renderThumbnail).toHaveBeenCalledTimes(1)
    expect(renderThumbnail).toHaveBeenCalledWith(
      expect.objectContaining({ id: 'AREA2345:7', pageIndex: 0, rects: [[10, 20, 110, 120]] })
    )
  })

  it('renders no preview without the PDF and says why', async () => {
    const renderThumbnail = vi.fn(async () => 'data:image/png;base64,AAA')
    render(BibliographyAnnotationsPanel, {
      props: { annotations: [area()], renderThumbnail, pdfAvailable: false },
    })

    expect(await screen.findByTestId('annotations-pdf-unavailable')).toBeInTheDocument()
    await waitFor(() => expect(screen.queryByTestId('annotation-preview')).not.toBeInTheDocument())
    expect(renderThumbnail).not.toHaveBeenCalled()
  })
})
