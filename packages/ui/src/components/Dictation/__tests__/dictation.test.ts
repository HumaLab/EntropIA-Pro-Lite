import { describe, expect, it } from 'vitest'

import { chooseDictationCaptureStrategy, encodeWavFromPcm, isSilentPcm } from '../dictation'

describe('dictation helpers', () => {
  it('prefers the PCM/WAV fallback on Linux WebKitGTK Tauri runtimes', () => {
    expect(
      chooseDictationCaptureStrategy({
        userAgent: 'Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 Safari/605.1.15',
        platform: 'Linux x86_64',
        hasMediaRecorder: true,
        hasAudioContext: true,
        hasTauriRuntime: true,
      })
    ).toBe('pcm-wav')
  })

  // Pro dev test: WebView2 recorded WebM/Opus and local Parakeet could not
  // decode it, so the dictation failed on Windows.
  it('records WAV in the Windows desktop app, where local engines cannot decode WebM', () => {
    expect(
      chooseDictationCaptureStrategy({
        userAgent:
          'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/157.0.0.0 Safari/537.36 Edg/157.0.0.0',
        platform: 'Win32',
        hasMediaRecorder: true,
        hasAudioContext: true,
        hasTauriRuntime: true,
      })
    ).toBe('pcm-wav')
  })

  it('keeps using MediaRecorder outside the affected environment when available', () => {
    expect(
      chooseDictationCaptureStrategy({
        userAgent:
          'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 Chrome/126.0.0.0 Safari/537.36',
        platform: 'MacIntel',
        hasMediaRecorder: true,
        hasAudioContext: true,
        hasTauriRuntime: false,
      })
    ).toBe('media-recorder')
  })

  it('calls a recording of zeros or near-zero noise silent', () => {
    expect(isSilentPcm([new Float32Array(16000), new Float32Array(16000)])).toBe(true)
    expect(isSilentPcm([Float32Array.from([0.0004, -0.0003, 0])])).toBe(true)
  })

  it('does not call quiet speech silent, wherever its loudest sample falls', () => {
    expect(isSilentPcm([new Float32Array(16000), Float32Array.from([0, -0.01, 0])])).toBe(false)
    expect(isSilentPcm([Float32Array.from([0.2, -0.2])])).toBe(false)
  })

  it('encodes captured PCM samples into a non-empty WAV blob', async () => {
    const wavBlob = encodeWavFromPcm([Float32Array.from([0, 0.25, -0.25, 0.5, -0.5])], 16000)
    const bytes = new Uint8Array(await wavBlob.arrayBuffer())
    const header = String.fromCharCode(...bytes.slice(0, 4))
    const format = String.fromCharCode(...bytes.slice(8, 12))

    expect(wavBlob.type).toBe('audio/wav')
    expect(wavBlob.size).toBeGreaterThan(44)
    expect(header).toBe('RIFF')
    expect(format).toBe('WAVE')
  })
})
