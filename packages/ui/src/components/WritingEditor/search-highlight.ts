import { Extension, type Editor } from '@tiptap/core'
import { Plugin, PluginKey } from '@tiptap/pm/state'
import { Decoration, DecorationSet } from '@tiptap/pm/view'
import { findMatches, type SearchMatch } from './search'

/**
 * Shows where the matches are while the caret is elsewhere.
 *
 * # Why a decoration and not a selection
 *
 * Selecting the current match would be simpler, but a browser does not paint
 * the selection of a contenteditable that has lost focus — and the writer is
 * typing in the search field, not in the manuscript. So a find that only
 * selected would highlight nothing at the exact moment it matters.
 *
 * # Why a decoration and not a mark
 *
 * A mark would change the document, and a changed document is journalled and
 * then saved. Searching is not editing: it must leave no trace. A decoration is
 * a view-layer overlay that never reaches the canonical JSON.
 */

export interface SearchState {
  query: string
  caseSensitive: boolean
  /** Index into `matches`, or -1 when nothing is current. */
  current: number
  matches: SearchMatch[]
}

interface PluginState extends SearchState {
  decorations: DecorationSet
}

export const searchKey = new PluginKey<PluginState>('writingSearch')

const EMPTY: SearchState = { query: '', caseSensitive: false, current: -1, matches: [] }

function decorate(doc: Parameters<typeof findMatches>[0], state: SearchState): DecorationSet {
  if (state.matches.length === 0) return DecorationSet.empty
  return DecorationSet.create(
    doc,
    state.matches.map((match, index) =>
      Decoration.inline(match.from, match.to, {
        class:
          index === state.current
            ? 'writing-search__hit writing-search__hit--current'
            : 'writing-search__hit',
      })
    )
  )
}

export const SearchHighlight = Extension.create({
  name: 'writingSearchHighlight',

  addProseMirrorPlugins() {
    return [
      new Plugin<PluginState>({
        key: searchKey,

        state: {
          init: (_config, editorState) => ({
            ...EMPTY,
            decorations: decorate(editorState.doc, EMPTY),
          }),

          apply: (tr, previous, _oldState, newState) => {
            const meta = tr.getMeta(searchKey) as Partial<SearchState> | undefined
            // Nothing asked and nothing moved: keep the decorations as they are
            // rather than rebuilding a set that would be identical.
            if (!meta && !tr.docChanged) return previous

            const next: SearchState = {
              query: meta?.query ?? previous.query,
              caseSensitive: meta?.caseSensitive ?? previous.caseSensitive,
              current: meta?.current ?? previous.current,
              matches: [],
            }
            next.matches = findMatches(newState.doc, next.query, {
              caseSensitive: next.caseSensitive,
            })
            // An edit can remove the match the writer was standing on, so the
            // index is clamped rather than left pointing past the end.
            if (next.current >= next.matches.length) {
              next.current = next.matches.length > 0 ? next.matches.length - 1 : -1
            }
            return { ...next, decorations: decorate(newState.doc, next) }
          },
        },

        props: {
          decorations: (state) => searchKey.getState(state)?.decorations,
        },
      }),
    ]
  },
})

/** What the editor currently knows about the search. Never null in practice. */
export function readSearch(editor: Editor): SearchState {
  return searchKey.getState(editor.state) ?? EMPTY
}

/**
 * Changes the search without editing the document.
 *
 * The transaction carries no steps, only meta, so it advances nothing the
 * autosave loop would notice — `docChanged` stays false and no revision is
 * earned for having looked for a word.
 */
export function setSearch(editor: Editor, patch: Partial<SearchState>): SearchState {
  editor.view.dispatch(editor.state.tr.setMeta(searchKey, patch))
  return readSearch(editor)
}

/** Moves to another match and scrolls it into view, without focusing the editor. */
export function goToMatch(editor: Editor, index: number): SearchState {
  const state = setSearch(editor, { current: index })
  const match = state.matches[index]
  if (!match) return state
  editor.view.dispatch(editor.state.tr.scrollIntoView())
  return state
}

/**
 * Replaces the current match and reports the state that follows.
 *
 * The index is deliberately not advanced: after replacing, the match that was
 * current is gone and the one that took its place at this index is the next
 * occurrence, so "replace" pressed repeatedly walks forward on its own.
 */
export function replaceCurrent(editor: Editor, replacement: string): SearchState {
  const state = readSearch(editor)
  const match = state.matches[state.current]
  if (!match) return state
  editor
    .chain()
    .insertContentAt({ from: match.from, to: match.to }, replacement, {
      updateSelection: false,
      parseOptions: { preserveWhitespace: 'full' },
    })
    .run()
  return readSearch(editor)
}

/**
 * Replaces every match in one transaction.
 *
 * Back to front: replacing an earlier match shifts every position after it, so
 * working forwards would apply each replacement to a range that had already
 * moved. One transaction also makes the whole thing a single undo.
 */
export function replaceAll(editor: Editor, replacement: string): SearchState {
  const { matches } = readSearch(editor)
  if (matches.length === 0) return readSearch(editor)

  const chain = editor.chain()
  for (let index = matches.length - 1; index >= 0; index -= 1) {
    const match = matches[index]
    if (!match) continue
    chain.insertContentAt({ from: match.from, to: match.to }, replacement, {
      updateSelection: false,
      parseOptions: { preserveWhitespace: 'full' },
    })
  }
  chain.run()
  return readSearch(editor)
}
