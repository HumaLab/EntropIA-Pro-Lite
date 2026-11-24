import type { I18nKey } from './i18n'

/**
 * The stage a chat answer is in, as `rag_ask` reports it on
 * `rag://progress` (src-tauri/src/rag/commands.rs). A local answer can take
 * minutes on CPU; naming the stage and the wait keeps it from looking hung.
 */
export const RAG_PROGRESS_EVENT = 'rag://progress'

export type RagProgressStage = 'understanding' | 'searching' | 'writing'

export interface RagProgressPayload {
  stage: RagProgressStage
}

const LABEL_KEYS: Record<RagProgressStage, I18nKey> = {
  understanding: 'ragChat.stage.understanding',
  // The searching stage keeps the line the chat always showed.
  searching: 'ragChat.thinking',
  writing: 'ragChat.stage.writing',
}

export function ragProgressLabelKey(stage: RagProgressStage): I18nKey {
  return LABEL_KEYS[stage]
}

/** `83` → `1:23`. */
export function formatWaitTime(totalSeconds: number): string {
  const minutes = Math.floor(totalSeconds / 60)
  const seconds = totalSeconds % 60
  return `${minutes}:${String(seconds).padStart(2, '0')}`
}
