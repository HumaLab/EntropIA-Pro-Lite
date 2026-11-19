import { listen } from '@tauri-apps/api/event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import {
  clearInflight,
  downloadKey,
  inflightDownloadEvents,
  isInflight,
  llmJobKey,
  markInflight,
  nlpJobKey,
  registerInflightEvents,
  resetInflightForTests,
  startInflightEventTracking,
  trackInflight,
  transcriptionJobKey,
  whenInflightSettles,
} from './inflight.svelte'

const handlers = new Map<string, (event: { payload: unknown }) => void>()

function emit(event: string, payload: unknown) {
  const handler = handlers.get(event)
  if (!handler) throw new Error(`nothing listens to ${event}`)
  handler({ payload })
}

beforeEach(() => {
  handlers.clear()
  vi.mocked(listen).mockImplementation(async (event, handler) => {
    handlers.set(event, handler as (event: { payload: unknown }) => void)
    return () => handlers.delete(event)
  })
})

afterEach(() => {
  resetInflightForTests()
})

describe('trackInflight', () => {
  it('keeps the key in progress until the action settles, then clears it', async () => {
    let finish: (value: string) => void = () => {}
    const running = trackInflight('backup', () => new Promise<string>((done) => (finish = done)))

    expect(isInflight('backup')).toBe(true)
    const settled = whenInflightSettles('backup')
    expect(settled).not.toBeNull()

    finish('ok')
    await expect(running).resolves.toBe('ok')
    await settled
    expect(isInflight('backup')).toBe(false)
    expect(whenInflightSettles('backup')).toBeNull()
  })

  it('clears the key when the action fails, and passes the error on', async () => {
    const failing = trackInflight('backup', () => Promise.reject(new Error('disk full')))

    await expect(failing).rejects.toThrow('disk full')
    expect(isInflight('backup')).toBe(false)
  })
})

describe('queued jobs', () => {
  it('stays in progress until the backend completion event arrives', async () => {
    const key = nlpJobKey('embed', 'item-1', 'asset-1')
    markInflight(key)
    await startInflightEventTracking()

    emit('nlp:complete', { item_id: 'item-1', asset_id: 'asset-2', job: 'embed' })
    expect(isInflight(key)).toBe(true)

    emit('nlp:complete', { item_id: 'item-1', asset_id: 'asset-1', job: 'embed' })
    expect(isInflight(key)).toBe(false)
  })

  it('clears an item-level NLP run on its first per-asset event', async () => {
    const key = nlpJobKey('ner', 'item-1')
    markInflight(key)
    await startInflightEventTracking()

    emit('nlp:error', { item_id: 'item-1', asset_id: 'asset-1', job: 'ner', error: 'x' })
    expect(isInflight(key)).toBe(false)
  })

  it('clears LLM and transcription jobs on their complete and error events', async () => {
    markInflight(llmJobKey('summarize', 'asset-1'))
    markInflight(llmJobKey('correct_ocr', 'asset-1'))
    markInflight(transcriptionJobKey('audio-1'))
    await startInflightEventTracking()

    emit('llm:complete', { id: 'asset-1', job: 'summarize', result: 'text' })
    emit('llm:error', { id: 'asset-1', job: 'correct_ocr', error: 'x' })
    emit('transcription:complete', { asset_id: 'audio-1' })

    expect(isInflight(llmJobKey('summarize', 'asset-1'))).toBe(false)
    expect(isInflight(llmJobKey('correct_ocr', 'asset-1'))).toBe(false)
    expect(isInflight(transcriptionJobKey('audio-1'))).toBe(false)
  })

  it('wakes up a view waiting for the job when it is cleared', async () => {
    markInflight('job')
    const settled = whenInflightSettles('job')
    clearInflight('job')
    await expect(settled).resolves.toBeUndefined()
  })
})

describe('download events', () => {
  it('listens to download events registered before and after tracking starts', async () => {
    registerInflightEvents(
      inflightDownloadEvents('llm', 'llm:download_complete', 'llm:download_error')
    )
    await startInflightEventTracking()
    registerInflightEvents(
      inflightDownloadEvents('parakeet', 'stt:parakeet_complete', 'stt:parakeet_error')
    )
    await startInflightEventTracking()
    await Promise.resolve()

    markInflight(downloadKey('llm'))
    markInflight(downloadKey('parakeet'))
    emit('llm:download_complete', {})
    emit('stt:parakeet_error', { error: 'x' })

    expect(isInflight(downloadKey('llm'))).toBe(false)
    expect(isInflight(downloadKey('parakeet'))).toBe(false)
  })
})
