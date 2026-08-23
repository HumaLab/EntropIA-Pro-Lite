import { render, screen, waitFor } from '@testing-library/svelte'
import { describe, expect, it, vi } from 'vitest'
import OcrRichText from './OcrRichText.svelte'

const renderOcrHtmlMock = vi.hoisted(() => vi.fn())
vi.mock('$lib/ocr-rich-text', () => ({
  renderOcrHtml: renderOcrHtmlMock,
}))

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((nextResolve) => {
    resolve = nextResolve
  })
  return { promise, resolve }
}

describe('OcrRichText', () => {
  it('renders resolved HTML and ignores an older async render', async () => {
    const first = deferred<string>()
    const second = deferred<string>()
    renderOcrHtmlMock.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise)

    const view = render(OcrRichText, {
      text: '# viejo',
      assetUrl: 'asset://source',
      sourceType: 'image',
      referenceWidth: 100,
      referenceHeight: 100,
    })
    await waitFor(() => expect(renderOcrHtmlMock).toHaveBeenCalledTimes(1))

    view.rerender({
      text: '# nuevo',
      assetUrl: 'asset://source',
      sourceType: 'image',
      referenceWidth: 100,
      referenceHeight: 100,
    })
    await waitFor(() => expect(renderOcrHtmlMock).toHaveBeenCalledTimes(2))

    second.resolve('<h1>nuevo</h1>')
    await waitFor(() => expect(screen.getByRole('heading', { level: 1 })).toHaveTextContent('nuevo'))

    first.resolve('<h1>viejo</h1>')
    await Promise.resolve()
    expect(screen.queryByText('viejo')).not.toBeInTheDocument()
  })
})
