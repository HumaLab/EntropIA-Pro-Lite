import { describe, expect, it } from 'vitest'
import { RAG_PROGRESS_EVENT, formatWaitTime, ragProgressLabelKey } from './rag-progress'

describe('rag progress', () => {
  it('listens on the event the backend emits', () => {
    expect(RAG_PROGRESS_EVENT).toBe('rag://progress')
  })

  it('names each stage with its own label', () => {
    expect(ragProgressLabelKey('understanding')).toBe('ragChat.stage.understanding')
    expect(ragProgressLabelKey('searching')).toBe('ragChat.thinking')
    expect(ragProgressLabelKey('writing')).toBe('ragChat.stage.writing')
  })

  it('formats the wait as minutes and seconds', () => {
    expect(formatWaitTime(0)).toBe('0:00')
    expect(formatWaitTime(7)).toBe('0:07')
    expect(formatWaitTime(83)).toBe('1:23')
    expect(formatWaitTime(600)).toBe('10:00')
  })
})
