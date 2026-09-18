export type DictationCaptureStrategy = 'media-recorder' | 'pcm-wav'

type DictationCaptureEnvironment = {
  userAgent?: string
  platform?: string
  hasMediaRecorder: boolean
  hasAudioContext: boolean
  hasTauriRuntime: boolean
}

export function chooseDictationCaptureStrategy(
  environment: DictationCaptureEnvironment
): DictationCaptureStrategy | null {
  const userAgent = environment.userAgent?.toLowerCase() ?? ''
  const platform = environment.platform?.toLowerCase() ?? ''
  const isLinux = platform.includes('linux') || userAgent.includes('linux')
  const isWebKit = userAgent.includes('applewebkit')
  const isChromium = /(chrome|chromium|crios|edg|opr)\//.test(userAgent)
  const isFirefox = userAgent.includes('firefox/')
  const shouldPreferWavFallback =
    environment.hasAudioContext &&
    ((environment.hasTauriRuntime && isLinux) || (isLinux && isWebKit && !isChromium && !isFirefox))

  if (shouldPreferWavFallback) {
    return 'pcm-wav'
  }

  if (environment.hasMediaRecorder) {
    return 'media-recorder'
  }

  if (environment.hasAudioContext) {
    return 'pcm-wav'
  }

  return null
}

function clampPcmSample(sample: number) {
  return Math.max(-1, Math.min(1, sample))
}

export function encodeWavFromPcm(samples: Float32Array[], sampleRate: number): Blob {
  const totalSamples = samples.reduce((total, chunk) => total + chunk.length, 0)
  const bytesPerSample = 2
  const headerBytes = 44
  const buffer = new ArrayBuffer(headerBytes + totalSamples * bytesPerSample)
  const view = new DataView(buffer)

  writeAscii(view, 0, 'RIFF')
  view.setUint32(4, 36 + totalSamples * bytesPerSample, true)
  writeAscii(view, 8, 'WAVE')
  writeAscii(view, 12, 'fmt ')
  view.setUint32(16, 16, true)
  view.setUint16(20, 1, true)
  view.setUint16(22, 1, true)
  view.setUint32(24, sampleRate, true)
  view.setUint32(28, sampleRate * bytesPerSample, true)
  view.setUint16(32, bytesPerSample, true)
  view.setUint16(34, 16, true)
  writeAscii(view, 36, 'data')
  view.setUint32(40, totalSamples * bytesPerSample, true)

  let offset = headerBytes
  for (const chunk of samples) {
    for (let index = 0; index < chunk.length; index += 1) {
      const sample = clampPcmSample(chunk[index] ?? 0)
      const value = sample < 0 ? sample * 0x8000 : sample * 0x7fff
      view.setInt16(offset, Math.round(value), true)
      offset += bytesPerSample
    }
  }

  return new Blob([buffer], { type: 'audio/wav' })
}

function writeAscii(view: DataView, offset: number, value: string) {
  for (let index = 0; index < value.length; index += 1) {
    view.setUint8(offset + index, value.charCodeAt(index))
  }
}
