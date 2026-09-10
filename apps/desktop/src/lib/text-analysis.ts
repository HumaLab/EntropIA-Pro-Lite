import { DEFAULT_STOPWORDS } from './stopwords'

/**
 * Pure text-frequency pipeline for the collection analysis panel.
 * No Svelte/Tauri imports — fully unit-testable.
 */

export type CorpusKind = 'extraction' | 'transcription'

export interface CorpusText {
  text: string
  kind: CorpusKind
}

export interface WordFrequency {
  word: string
  count: number
}

export interface FrequencyOptions {
  stopwords?: Set<string>
  minTokenLength?: number
}

const DEFAULT_MIN_TOKEN_LENGTH = 3

// AssemblyAI output uses "Hablante N:" (see assemblyai.rs); "Speaker N:" kept for legacy text.
const SPEAKER_NUMBERED_RE = /^\s*(?:hablante|speaker)\s*\d+\s*:\s*/i
// Manually edited aliases: 1-3 capitalized words before a colon at line start.
const SPEAKER_ALIAS_RE = /^\s*\p{Lu}[\p{L}\p{M}'’.-]*(?:\s+\p{Lu}[\p{L}\p{M}'’.-]*){0,2}\s*:\s*/u

const TOKEN_RE = /[\p{L}\p{N}][\p{L}\p{N}'’-]*/gu
const EDGE_TRIM_RE = /^['’-]+|['’-]+$/g
const ISOLATED_NUMBER_RE = /^\d+([.,]\d+)?$/

/**
 * Remove speaker labels ("Hablante 1:", "Speaker 2:", "María Pérez:") from
 * the start of each line. Only meant for transcription text — extraction
 * (OCR) text legitimately contains "Word:" prefixes that must be kept.
 */
export function stripSpeakerLabels(text: string): string {
  return text
    .split('\n')
    .map((line) => {
      if (SPEAKER_NUMBERED_RE.test(line)) return line.replace(SPEAKER_NUMBERED_RE, '')
      return line.replace(SPEAKER_ALIAS_RE, '')
    })
    .join('\n')
}

/**
 * Normalize and tokenize a single already-prepared text. Keeps accents
 * ("análisis" ≠ "analisis" preserves corpus fidelity); drops short tokens,
 * isolated numbers and stopwords.
 */
export function tokenize(text: string, opts?: FrequencyOptions): string[] {
  const stopwords = opts?.stopwords ?? DEFAULT_STOPWORDS
  const minTokenLength = opts?.minTokenLength ?? DEFAULT_MIN_TOKEN_LENGTH

  const matches = text.normalize('NFC').toLowerCase().match(TOKEN_RE) ?? []
  const tokens: string[] = []
  for (const raw of matches) {
    const token = raw.replace(EDGE_TRIM_RE, '')
    if (token.length < minTokenLength) continue
    if (ISOLATED_NUMBER_RE.test(token)) continue
    if (stopwords.has(token)) continue
    tokens.push(token)
  }
  return tokens
}

function countTokens(
  texts: CorpusText[],
  opts: FrequencyOptions | undefined,
  counts: Map<string, number>
) {
  for (const { text, kind } of texts) {
    const prepared = kind === 'transcription' ? stripSpeakerLabels(text.normalize('NFC')) : text
    for (const token of tokenize(prepared, opts)) {
      counts.set(token, (counts.get(token) ?? 0) + 1)
    }
  }
}

function sortFrequencies(counts: Map<string, number>): WordFrequency[] {
  return [...counts.entries()]
    .map(([word, count]) => ({ word, count }))
    .sort((a, b) => b.count - a.count || a.word.localeCompare(b.word, 'es'))
}

export function buildFrequencies(texts: CorpusText[], opts?: FrequencyOptions): WordFrequency[] {
  const counts = new Map<string, number>()
  countTokens(texts, opts, counts)
  return sortFrequencies(counts)
}

const ASYNC_CHUNK_SIZE = 25

/**
 * Same output as buildFrequencies, but yields the main thread between
 * chunks of texts so the UI can paint loading state on large corpora.
 */
export async function buildFrequenciesAsync(
  texts: CorpusText[],
  opts?: FrequencyOptions
): Promise<WordFrequency[]> {
  const counts = new Map<string, number>()
  for (let i = 0; i < texts.length; i += ASYNC_CHUNK_SIZE) {
    countTokens(texts.slice(i, i + ASYNC_CHUNK_SIZE), opts, counts)
    if (i + ASYNC_CHUNK_SIZE < texts.length) {
      await new Promise((resolve) => setTimeout(resolve, 0))
    }
  }
  return sortFrequencies(counts)
}

export function topN(frequencies: WordFrequency[], n: number): WordFrequency[] {
  return frequencies.slice(0, n)
}

/** Round "nice" Y-axis ticks from 0 up to (at least) maxValue. */
export function computeTicks(maxValue: number, tickCount = 4): number[] {
  if (maxValue <= 0) return [0]
  const rawStep = maxValue / tickCount
  const magnitude = 10 ** Math.floor(Math.log10(rawStep))
  const residual = rawStep / magnitude
  const niceResidual = residual <= 1 ? 1 : residual <= 2 ? 2 : residual <= 5 ? 5 : 10
  const step = niceResidual * magnitude
  const ticks: number[] = []
  for (let value = 0; value < maxValue + step; value += step) {
    ticks.push(Math.round(value * 1000) / 1000)
  }
  return ticks
}

const DEFAULT_LABEL_MAX = 12

export function truncateLabel(word: string, max = DEFAULT_LABEL_MAX): string {
  return word.length > max ? `${word.slice(0, max - 1)}…` : word
}
