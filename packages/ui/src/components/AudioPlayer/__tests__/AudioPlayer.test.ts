import { fireEvent, render, screen, waitFor } from '@testing-library/svelte'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import AudioPlayer from '../AudioPlayer.svelte'

describe('AudioPlayer', () => {
  beforeEach(() => {
    vi.restoreAllMocks()
    Object.defineProperty(URL, 'createObjectURL', {
      configurable: true,
      value: vi.fn(() => 'blob:audio-fallback'),
    })
    Object.defineProperty(URL, 'revokeObjectURL', {
      configurable: true,
      value: vi.fn(),
    })
  })

  afterEach(() => {
    vi.unstubAllGlobals()
  })

  it('keeps fallback blob URLs alive until the source changes', async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      blob: () => Promise.resolve(new Blob(['audio'], { type: 'application/octet-stream' })),
    })
    vi.stubGlobal('fetch', fetchMock)

    const view = render(AudioPlayer, { props: { src: '/audio/interview.wav' } })
    const audio = screen.getByTestId('audio-player').querySelector('audio') as HTMLAudioElement

    await fireEvent.error(audio)

    await waitFor(() => expect(URL.createObjectURL).toHaveBeenCalledTimes(1))
    expect(fetchMock).toHaveBeenCalledWith('/audio/interview.wav')
    expect((URL.createObjectURL as ReturnType<typeof vi.fn>).mock.calls[0]![0].type).toBe(
      'audio/wav'
    )
    expect(URL.revokeObjectURL).not.toHaveBeenCalled()
    expect(audio.src).toContain('blob:audio-fallback')

    await view.rerender({ src: '/audio/next.mp3' })

    expect(URL.revokeObjectURL).toHaveBeenCalledWith('blob:audio-fallback')
    expect(screen.getByTestId('audio-duration')).toHaveTextContent('0:00')
  })

  it('uses a custom fallback blob loader before fetch', async () => {
    const fetchMock = vi.fn()
    const fallbackBlobLoader = vi.fn().mockResolvedValue(new Blob(['audio'], { type: 'audio/wav' }))
    vi.stubGlobal('fetch', fetchMock)

    render(AudioPlayer, {
      props: { src: '/audio/interview.wav', fallbackBlobLoader },
    })
    const audio = screen.getByTestId('audio-player').querySelector('audio') as HTMLAudioElement

    await fireEvent.error(audio)

    await waitFor(() => expect(URL.createObjectURL).toHaveBeenCalledTimes(1))
    expect(fallbackBlobLoader).toHaveBeenCalledTimes(1)
    expect(fetchMock).not.toHaveBeenCalled()
    expect((URL.createObjectURL as ReturnType<typeof vi.fn>).mock.calls[0]![0].type).toBe(
      'audio/wav'
    )
    expect(URL.revokeObjectURL).not.toHaveBeenCalled()
    expect(audio.src).toContain('blob:audio-fallback')
  })

  it('reports a diagnostic when the fallback blob also fails to play', async () => {
    const fallbackBlobLoader = vi.fn().mockResolvedValue(new Blob(['audio'], { type: 'audio/wav' }))

    render(AudioPlayer, {
      props: { src: '/audio/interview.wav', fallbackBlobLoader },
    })
    const audio = screen.getByTestId('audio-player').querySelector('audio') as HTMLAudioElement

    await fireEvent.error(audio)
    await waitFor(() => expect(URL.createObjectURL).toHaveBeenCalledTimes(1))
    await fireEvent.error(audio)

    expect(screen.getByTestId('audio-load-error')).toHaveTextContent(
      'Fallback audio loaded via custom-loader, but playback failed'
    )
  })

  it('ignores non-finite metadata durations', async () => {
    render(AudioPlayer, { props: { src: '/audio/interview.wav' } })
    const audio = screen.getByTestId('audio-player').querySelector('audio') as HTMLAudioElement

    Object.defineProperty(audio, 'duration', { configurable: true, value: Infinity })
    await fireEvent.loadedMetadata(audio)
    expect(screen.getByTestId('audio-duration')).toHaveTextContent('0:00')

    Object.defineProperty(audio, 'duration', { configurable: true, value: 65 })
    await fireEvent.loadedMetadata(audio)
    expect(screen.getByTestId('audio-duration')).toHaveTextContent('1:05')
  })
})
