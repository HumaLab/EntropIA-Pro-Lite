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

// ---------------------------------------------------------------------------
// Phrases, associations and word trees. Every unit stays inside one sentence
// of one text: a phrase or a neighbourhood never spans a full stop.
// ---------------------------------------------------------------------------

const SENTENCE_SPLIT_RE = /[.!?;:…\n\r]+/

/** The sentences of a text, each as its words in order (lowercased, NFC). */
function sentenceWords({ text, kind }: CorpusText): string[][] {
  const prepared = kind === 'transcription' ? stripSpeakerLabels(text.normalize('NFC')) : text
  return prepared
    .normalize('NFC')
    .toLowerCase()
    .split(SENTENCE_SPLIT_RE)
    .map((sentence) =>
      (sentence.match(TOKEN_RE) ?? []).map((raw) => raw.replace(EDGE_TRIM_RE, '')).filter(Boolean)
    )
    .filter((words) => words.length > 0)
}

/** A word that can carry meaning on its own: what `tokenize` keeps. */
function isContentWord(word: string, opts?: FrequencyOptions): boolean {
  const stopwords = opts?.stopwords ?? DEFAULT_STOPWORDS
  const minTokenLength = opts?.minTokenLength ?? DEFAULT_MIN_TOKEN_LENGTH
  return word.length >= minTokenLength && !ISOLATED_NUMBER_RE.test(word) && !stopwords.has(word)
}

export interface NgramFrequency {
  gram: string
  count: number
}

export interface NgramOptions extends FrequencyOptions {
  /** Grams seen fewer times are dropped. Default 2: a phrase, not a one-off. */
  minCount?: number
}

/**
 * Counts the n-grams (2 to 5 words) of the corpus. Inner words may be
 * stopwords ("guerra del paraguay"); the first and last must be content words,
 * so a gram never starts or ends on "de" or "la".
 */
export function buildNgrams(texts: CorpusText[], n: number, opts?: NgramOptions): NgramFrequency[] {
  if (!Number.isInteger(n) || n < 2 || n > 5) throw new RangeError('n must be 2 to 5')
  const minCount = opts?.minCount ?? 2
  const counts = new Map<string, number>()
  for (const text of texts) {
    for (const words of sentenceWords(text)) {
      for (let start = 0; start + n <= words.length; start++) {
        const first = words[start]!
        const last = words[start + n - 1]!
        if (!isContentWord(first, opts) || !isContentWord(last, opts)) continue
        const gram = words.slice(start, start + n).join(' ')
        counts.set(gram, (counts.get(gram) ?? 0) + 1)
      }
    }
  }
  return [...counts.entries()]
    .filter(([, count]) => count >= minCount)
    .map(([gram, count]) => ({ gram, count }))
    .sort((a, b) => b.count - a.count || a.gram.localeCompare(b.gram, 'es'))
}

export interface Collocate {
  word: string
  /** Times it falls inside the window around the node word. */
  count: number
  /** Its frequency in the whole corpus. */
  frequency: number
  /** Dunning's log-likelihood G² of the node/collocate 2×2 table. */
  g2: number
}

export interface CollocateOptions extends FrequencyOptions {
  /** Content words on each side of the node. Default 5. */
  window?: number
  /** Collocates seen fewer times near the node are dropped. Default 2. */
  minCount?: number
}

/** `O · ln(O / E)`, with 0 for an empty cell. */
function llTerm(observed: number, expected: number): number {
  return observed > 0 && expected > 0 ? observed * Math.log(observed / expected) : 0
}

/**
 * The words that keep company with `node`, ranked by log-likelihood G²
 * (Dunning 1993) over content words: the window holds the content words
 * around each occurrence, inside its sentence. Only positive associations
 * (seen more than chance predicts) are kept.
 */
export function buildCollocates(
  texts: CorpusText[],
  node: string,
  opts?: CollocateOptions
): Collocate[] {
  const target = node.normalize('NFC').toLowerCase().trim()
  const window = opts?.window ?? 5
  const minCount = opts?.minCount ?? 2
  const frequency = new Map<string, number>()
  const near = new Map<string, number>()
  let total = 0
  let slots = 0
  for (const text of texts) {
    for (const words of sentenceWords(text)) {
      const content = words.filter((word) => isContentWord(word, opts))
      total += content.length
      content.forEach((word, index) => {
        frequency.set(word, (frequency.get(word) ?? 0) + 1)
        if (word !== target) return
        const from = Math.max(0, index - window)
        const to = Math.min(content.length - 1, index + window)
        for (let i = from; i <= to; i++) {
          if (i === index) continue
          slots++
          near.set(content[i]!, (near.get(content[i]!) ?? 0) + 1)
        }
      })
    }
  }
  if (slots === 0) return []
  const collocates: Collocate[] = []
  for (const [word, o11] of near) {
    if (o11 < minCount) continue
    const c1 = frequency.get(word) ?? 0
    const e11 = (slots * c1) / total
    if (o11 <= e11) continue
    const o12 = slots - o11
    // Overlapping windows can count a neighbour more often than it occurs
    // (o11 > c1): the cells below would go negative. Clamp them instead of
    // letting the log terms silently swallow the inconsistency.
    const o21 = Math.max(0, c1 - o11)
    const o22 = Math.max(0, total - slots - o21)
    const g2 =
      2 *
      (llTerm(o11, e11) +
        llTerm(o12, (slots * (total - c1)) / total) +
        llTerm(o21, ((total - slots) * c1) / total) +
        llTerm(o22, ((total - slots) * (total - c1)) / total))
    collocates.push({ word, count: o11, frequency: c1, g2 })
  }
  return collocates.sort((a, b) => b.g2 - a.g2 || a.word.localeCompare(b.word, 'es'))
}

export interface WordTreeNode {
  word: string
  count: number
  children: WordTreeNode[]
}

export interface WordTreeOptions {
  /** Words to follow from the root. Default 3. */
  depth?: number
  /** What follows the root (`right`) or what precedes it (`left`). */
  side?: 'right' | 'left'
  /** Children kept per node, the most frequent first. Default 8. */
  maxBranches?: number
}

/**
 * A word tree: every continuation of `root` inside its sentence, merged where
 * the words agree, with how often each branch occurs. All words count here,
 * stopwords included: a tree reads as the sentences read.
 */
export function buildWordTree(
  texts: CorpusText[],
  root: string,
  opts?: WordTreeOptions
): WordTreeNode {
  const target = root.normalize('NFC').toLowerCase().trim()
  const depth = opts?.depth ?? 3
  const side = opts?.side ?? 'right'
  const maxBranches = opts?.maxBranches ?? 8
  const tree: WordTreeNode = { word: target, count: 0, children: [] }
  for (const text of texts) {
    for (const sentence of sentenceWords(text)) {
      const words = side === 'right' ? sentence : [...sentence].reverse()
      words.forEach((word, index) => {
        if (word !== target) return
        tree.count++
        let node = tree
        for (const next of words.slice(index + 1, index + 1 + depth)) {
          let child = node.children.find((candidate) => candidate.word === next)
          if (!child) {
            child = { word: next, count: 0, children: [] }
            node.children.push(child)
          }
          child.count++
          node = child
        }
      })
    }
  }
  const prune = (node: WordTreeNode): WordTreeNode => ({
    ...node,
    children: node.children
      .sort((a, b) => b.count - a.count || a.word.localeCompare(b.word, 'es'))
      .slice(0, maxBranches)
      .map(prune),
  })
  return prune(tree)
}
