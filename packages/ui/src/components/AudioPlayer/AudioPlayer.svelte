<script lang="ts">
  import { onDestroy } from 'svelte'
  import { ActionIcon } from '../Button'

  interface AudioPlayerLabels {
    skipBack: string
    play: string
    pause: string
    skipForward: string
    seek: string
    volume: string
  }

  let {
    src,
    fallbackBlobLoader,
    labels: labelsProp = {},
  }: {
    src: string
    fallbackBlobLoader?: () => Promise<Blob>
    labels?: Partial<AudioPlayerLabels>
  } = $props()

  const defaultLabels: AudioPlayerLabels = {
    skipBack: 'Skip back 5 seconds',
    play: 'Play',
    pause: 'Pause',
    skipForward: 'Skip forward 5 seconds',
    seek: 'Seek',
    volume: 'Volume',
  }

  const labels = $derived({ ...defaultLabels, ...labelsProp })

  let playing = $state(false)
  let currentTime = $state(0)
  let duration = $state(0)
  let volume = $state(1)
  let audioEl: HTMLAudioElement | undefined = $state()
  let blobUrl = $state<string | null>(null)
  let loadError = $state(false)
  let fallbackDiagnostic = $state<string | null>(null)
  let fallbackStage = $state<string | null>(null)
  let activeBlobUrl: string | null = null
  let lastSrc: string | null = null
  let fallbackAttempt = 0

  $effect(() => {
    if (src === lastSrc) return

    lastSrc = src
    fallbackAttempt += 1
    playing = false
    currentTime = 0
    duration = 0
    loadError = false
    fallbackDiagnostic = null
    fallbackStage = null
    clearBlobUrl()

    if (audioEl) {
      audioEl.pause()
      audioEl.currentTime = 0
      audioEl.load()
    }
  })

  onDestroy(() => {
    clearBlobUrl()
  })

  const progress = $derived(duration > 0 ? currentTime / duration : 0)

  const formattedTime = $derived(formatTime(currentTime))
  const formattedDuration = $derived(formatTime(duration))

  function formatTime(seconds: number): string {
    if (!Number.isFinite(seconds) || seconds <= 0) return '0:00'

    const s = Math.floor(seconds)
    const m = Math.floor(s / 60)
    const sec = s % 60
    if (m >= 60) {
      const h = Math.floor(m / 60)
      const rm = m % 60
      return `${h}:${String(rm).padStart(2, '0')}:${String(sec).padStart(2, '0')}`
    }
    return `${m}:${String(sec).padStart(2, '0')}`
  }

  function togglePlay() {
    if (!audioEl) return
    if (playing) {
      audioEl.pause()
    } else {
      void audioEl.play()
    }
  }

  function seek(e: MouseEvent) {
    if (!audioEl || duration <= 0) return
    const target = e.currentTarget as HTMLElement
    const rect = target.getBoundingClientRect()
    const ratio = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width))
    audioEl.currentTime = ratio * duration
  }

  function skip(seconds: number) {
    if (!audioEl) return
    audioEl.currentTime = Math.max(0, Math.min(duration, audioEl.currentTime + seconds))
  }

  function setVolume(v: number) {
    if (!audioEl) return
    const clamped = Math.max(0, Math.min(1, v))
    audioEl.volume = clamped
    volume = clamped
  }

  function handleTimeUpdate() {
    if (!audioEl) return
    currentTime = Number.isFinite(audioEl.currentTime) ? audioEl.currentTime : 0
  }

  function handleLoadedMetadata() {
    if (!audioEl) return
    duration = Number.isFinite(audioEl.duration) && audioEl.duration > 0 ? audioEl.duration : 0
  }

  function handlePlay() {
    playing = true
  }

  function handlePause() {
    playing = false
  }

  function handleEnded() {
    playing = false
    currentTime = 0
  }

  function handleVolumeChange() {
    if (!audioEl) return
    volume = audioEl.volume
  }

  async function handleError() {
    if (!src || !audioEl) return

    if (blobUrl) {
      failFallbackPlayback()
      loadError = true
      return
    }

    const attemptedSrc = src
    const attempt = ++fallbackAttempt

    try {
      const blob = fallbackBlobLoader
        ? await fallbackBlobLoader()
        : await fetchFallbackBlob(attemptedSrc)
      if (attempt !== fallbackAttempt || attemptedSrc !== src || !audioEl) return

      const typedBlob = ensureAudioBlobType(blob, attemptedSrc)
      const nextBlobUrl = URL.createObjectURL(typedBlob)
      replaceBlobUrl(nextBlobUrl)
      fallbackStage = 'custom-loader'
      audioEl.src = nextBlobUrl
      audioEl.load()
    } catch (customFallbackError) {
      if (attempt !== fallbackAttempt || attemptedSrc !== src || !audioEl) return

      if (!fallbackBlobLoader) {
        failFallbackLoad(customFallbackError)
        return
      }

      try {
        const blob = await fetchFallbackBlob(attemptedSrc)
        if (attempt !== fallbackAttempt || attemptedSrc !== src || !audioEl) return

        const typedBlob = ensureAudioBlobType(blob, attemptedSrc)
        const nextBlobUrl = URL.createObjectURL(typedBlob)
        replaceBlobUrl(nextBlobUrl)
        fallbackStage = 'fetch'
        audioEl.src = nextBlobUrl
        audioEl.load()
      } catch (fetchFallbackError) {
        if (attempt !== fallbackAttempt) return
        failFallbackLoad(fetchFallbackError, customFallbackError)
      }
    }
  }

  async function fetchFallbackBlob(source: string): Promise<Blob> {
    const response = await fetch(source)
    if (!response.ok) throw new Error(`HTTP ${response.status}`)
    return response.blob()
  }

  function failFallbackLoad(error: unknown, customFallbackError?: unknown) {
    console.error('[AudioPlayer] Fallback load failed:', error)
    if (customFallbackError) {
      console.error('[AudioPlayer] Custom fallback load failed:', customFallbackError)
    }
    fallbackDiagnostic = error instanceof Error ? error.message : null
    loadError = true
  }

  function failFallbackPlayback() {
    const code = audioEl?.error?.code
    const detail = code ? `media error code ${code}` : 'unknown media error'
    fallbackDiagnostic = `Fallback audio loaded via ${fallbackStage ?? 'blob'}, but playback failed (${detail})`
    console.error('[AudioPlayer] Fallback playback failed:', fallbackDiagnostic)
  }

  function replaceBlobUrl(nextBlobUrl: string) {
    if (activeBlobUrl) {
      URL.revokeObjectURL(activeBlobUrl)
    }
    activeBlobUrl = nextBlobUrl
    blobUrl = nextBlobUrl
  }

  function clearBlobUrl() {
    if (activeBlobUrl) {
      URL.revokeObjectURL(activeBlobUrl)
      activeBlobUrl = null
    }
    blobUrl = null
  }

  function ensureAudioBlobType(blob: Blob, source: string): Blob {
    if (blob.type && blob.type !== 'application/octet-stream') return blob

    const mimeType = audioMimeTypeFromSource(source)
    return mimeType ? new Blob([blob], { type: mimeType }) : blob
  }

  function audioMimeTypeFromSource(source: string): string | null {
    const extension = source.split(/[?#]/, 1)[0]?.split('.').pop()?.toLowerCase()
    switch (extension) {
      case 'wav':
        return 'audio/wav'
      case 'mp3':
        return 'audio/mpeg'
      case 'flac':
        return 'audio/flac'
      case 'm4a':
        return 'audio/mp4'
      case 'aac':
        return 'audio/aac'
      case 'ogg':
        return 'audio/ogg'
      default:
        return null
    }
  }
</script>

<div class="audio-player" data-testid="audio-player">
  <audio
    bind:this={audioEl}
    src={blobUrl ?? src}
    preload="metadata"
    ontimeupdate={handleTimeUpdate}
    onloadedmetadata={handleLoadedMetadata}
    onplay={handlePlay}
    onpause={handlePause}
    onended={handleEnded}
    onvolumechange={handleVolumeChange}
    onerror={handleError}
  ></audio>

  {#if loadError}
    <p class="audio-player__error" data-testid="audio-load-error">
      No se pudo reproducir el audio. Probá abrir el archivo original o convertirlo a un formato
      compatible.{#if fallbackDiagnostic} Detalle: {fallbackDiagnostic}.{/if}
    </p>
  {/if}

  <!-- Transport controls -->
  <div class="audio-player__transport">
    <button
      type="button"
      class="audio-player__btn audio-player__btn--skip"
      data-testid="audio-skip-back"
      aria-label={labels.skipBack}
      onclick={() => skip(-5)}
    >
      <ActionIcon name="skip-back" size={16} />
      <span>5s</span>
    </button>

    <button
      type="button"
      class="audio-player__btn audio-player__btn--play"
      data-testid="audio-play-pause"
      aria-label={playing ? labels.pause : labels.play}
      onclick={togglePlay}
    >
      <ActionIcon name={playing ? 'pause' : 'play'} size={24} />
    </button>

    <button
      type="button"
      class="audio-player__btn audio-player__btn--skip"
      data-testid="audio-skip-forward"
      aria-label={labels.skipForward}
      onclick={() => skip(5)}
    >
      <span>5s</span>
      <ActionIcon name="skip-forward" size={16} />
    </button>
  </div>

  <!-- Progress bar -->
  <div
    class="audio-player__progress"
    data-testid="audio-progress-bar"
    onclick={seek}
    onkeydown={(e) => {
      if (e.key === 'ArrowRight') {
        skip(5)
      } else if (e.key === 'ArrowLeft') {
        skip(-5)
      } else if (e.key === 'Home') {
        if (audioEl) audioEl.currentTime = 0
      } else if (e.key === 'End') {
        if (audioEl) audioEl.currentTime = duration
      }
    }}
    role="slider"
    tabindex="0"
    aria-label={labels.seek}
    aria-valuemin={0}
    aria-valuemax={100}
    aria-valuenow={Math.round(progress * 100)}
  >
    <div class="audio-player__progress-fill" style={`width: ${progress * 100}%`}></div>
  </div>

  <!-- Time display -->
  <div class="audio-player__time" data-testid="audio-time">
    <span data-testid="audio-current-time">{formattedTime}</span>
    <span class="audio-player__time-sep">/</span>
    <span data-testid="audio-duration">{formattedDuration}</span>
  </div>

  <!-- Volume -->
  <div class="audio-player__volume">
    <label class="audio-player__volume-label" for="audio-volume-slider">
      <ActionIcon name="volume" size={16} />
    </label>
    <input
      id="audio-volume-slider"
      type="range"
      min="0"
      max="1"
      step="0.05"
      value={volume}
      oninput={(e) => setVolume(parseFloat((e.target as HTMLInputElement).value))}
      class="audio-player__volume-slider"
      data-testid="audio-volume-slider"
      aria-label={labels.volume}
    />
  </div>
</div>

<style>
  .audio-player {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: var(--space-3);
    height: 100%;
    padding: var(--space-6);
    background-color: var(--color-bg);
  }

  .audio-player__transport {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .audio-player__btn {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-control);
    background-color: var(--color-surface);
    color: var(--color-text-primary);
    cursor: pointer;
    font-size: var(--font-size-sm);
    transition:
      background-color var(--transition-base),
      border-color var(--transition-base);
  }

  .audio-player__btn:hover {
    background-color: var(--color-surface-raised);
    border-color: var(--color-text-muted);
  }

  .audio-player__btn:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }

  .audio-player__btn--play {
    width: 56px;
    height: 56px;
    border-radius: var(--radius-surface);
    font-size: var(--font-size-xl);
    border-color: var(--color-accent);
    background-color: var(--color-surface-raised);
  }

  .audio-player__btn--play:hover {
    background-color: var(--color-accent);
    color: var(--color-bg);
  }

  .audio-player__progress {
    width: 100%;
    max-width: 480px;
    height: 6px;
    background-color: var(--color-border);
    border-radius: var(--radius-xs);
    cursor: pointer;
    position: relative;
    overflow: hidden;
  }

  .audio-player__progress:hover {
    height: 8px;
  }

  .audio-player__progress-fill {
    height: 100%;
    background-color: var(--color-accent);
    border-radius: var(--radius-xs);
    transition: width 0.1s linear;
  }

  .audio-player__time {
    display: flex;
    gap: var(--space-1);
    font-family: var(--font-mono);
    font-size: var(--font-size-sm);
    color: var(--color-text-secondary);
  }

  .audio-player__time-sep {
    color: var(--color-text-muted);
  }

  .audio-player__volume {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }

  .audio-player__volume-label {
    display: inline-flex;
    cursor: default;
  }

  .audio-player__volume-slider {
    width: 80px;
    accent-color: var(--color-accent);
  }

  .audio-player__error {
    margin: 0;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--color-danger-soft);
    color: var(--color-danger);
    font-size: var(--font-size-sm);
    text-align: center;
    max-width: 480px;
  }
</style>
