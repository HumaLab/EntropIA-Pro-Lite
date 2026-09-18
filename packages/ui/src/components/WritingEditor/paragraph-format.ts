import { Extension, type CommandProps } from '@tiptap/core'
import TextAlign from '@tiptap/extension-text-align'
import type { Node as ProseMirrorNode } from '@tiptap/pm/model'
import type { EditorState, Selection } from '@tiptap/pm/state'

/**
 * Paragraph formatting: alignment, indent and line spacing.
 *
 * Each is an attribute of a block — a paragraph or a heading — not a mark on
 * its text, so a command acts on every block the selection touches, whole,
 * the way a word processor does. Every value comes from a fixed set and the
 * default of each is the absence of the attribute: a manuscript with none of
 * them saves as it did before they existed.
 *
 * # What a build that predates them does
 *
 * These are node attributes, not marks, so an older build does not refuse a
 * document carrying them (document-contract.ts only refuses unknown nodes and
 * marks). It opens it and drops the attributes; if it then saves, they are
 * gone. That is the cost of staying at schema version 1.
 */

/** The blocks that take paragraph formatting. */
export const PARAGRAPH_TYPES = ['paragraph', 'heading'] as const

export const TEXT_ALIGNMENTS = ['left', 'center', 'right', 'justify'] as const
export type TextAlignment = (typeof TEXT_ALIGNMENTS)[number]

/** Unitless multiples of the font size; the theme's own spacing is the absence of one. */
export const LINE_HEIGHTS = ['1', '1.15', '1.5', '2'] as const
export type LineHeight = (typeof LINE_HEIGHTS)[number]

/** Indent levels run 1..INDENT_MAX; 0 is no indent and is never stored. */
export const INDENT_MAX = 8

/**
 * A stored or pasted alignment, or null for the default. Left is the default,
 * so it is null too: a paragraph aligned left carries nothing.
 */
export function parseTextAlign(value: unknown): Exclude<TextAlignment, 'left'> | null {
  if (typeof value !== 'string' || value === 'left') return null
  return (TEXT_ALIGNMENTS as readonly string[]).includes(value)
    ? (value as Exclude<TextAlignment, 'left'>)
    : null
}

/** A stored or pasted line height from the set, or null for anything else. */
export function parseLineHeight(value: unknown): LineHeight | null {
  if (typeof value !== 'string') return null
  return (LINE_HEIGHTS as readonly string[]).includes(value) ? (value as LineHeight) : null
}

/** A stored or pasted indent level, 0 for none or for anything off the scale. */
export function parseIndent(value: unknown): number {
  const level =
    typeof value === 'number'
      ? value
      : typeof value === 'string' && /^\d+$/.test(value)
        ? Number(value)
        : NaN
  return Number.isInteger(level) && level >= 1 && level <= INDENT_MAX ? level : 0
}

interface Block {
  node: ProseMirrorNode
  pos: number
  parent: ProseMirrorNode | null
}

/**
 * The paragraphs and headings the selection touches, each once. An empty
 * caret touches the one it is in.
 *
 * A footnote's paragraphs are not among them: every export writes a note as
 * one run of prose, so formatting set there would be lost on the way out.
 */
function selectedBlocks(doc: ProseMirrorNode, selection: Selection): Block[] {
  const blocks: Block[] = []
  const seen = new Set<number>()
  for (const range of selection.ranges) {
    doc.nodesBetween(range.$from.pos, range.$to.pos, (node, pos, parent) => {
      if (node.type.name === 'footnote') return false
      if (!(PARAGRAPH_TYPES as readonly string[]).includes(node.type.name)) return true
      if (!seen.has(pos)) {
        seen.add(pos)
        blocks.push({ node, pos, parent })
      }
      return false
    })
  }
  return blocks
}

/** Sets one attribute on every selected block, as one transaction. */
function setBlockAttribute(name: string, value: string | null) {
  return ({ tr, dispatch }: CommandProps): boolean => {
    const blocks = selectedBlocks(tr.doc, tr.selection)
    if (blocks.length === 0) return false
    if (dispatch) {
      for (const { node, pos } of blocks) {
        if (node.attrs[name] === value) continue
        tr.setNodeMarkup(pos, undefined, { ...node.attrs, [name]: value })
      }
    }
    return true
  }
}

function inListItem(selection: Selection): boolean {
  const { $from } = selection
  for (let depth = $from.depth; depth > 0; depth--) {
    if ($from.node(depth).type.name === 'listItem') return true
  }
  return false
}

/**
 * Every selected block one level in or out, each from its own level. A list's
 * own paragraphs are left alone: a list is indented by nesting it.
 */
function shiftIndent(direction: 1 | -1) {
  return ({ tr, dispatch }: CommandProps): boolean => {
    const changes = selectedBlocks(tr.doc, tr.selection)
      .filter((block) => block.parent?.type.name !== 'listItem')
      .map((block) => {
        const current = parseIndent(block.node.attrs.indent)
        const next = Math.min(Math.max(current + direction, 0), INDENT_MAX)
        return { block, current, next }
      })
      .filter(({ current, next }) => current !== next)
    if (changes.length === 0) return false
    if (dispatch) {
      for (const { block, next } of changes) {
        tr.setNodeMarkup(block.pos, undefined, { ...block.node.attrs, indent: next || null })
      }
    }
    return true
  }
}

/**
 * What the selection's blocks share, for the toolbar. `null` means they
 * differ, so no option is shown as the current one.
 */
export interface ParagraphFormat {
  /** Whether the selection touches any block the tools act on. */
  applicable: boolean
  /** Left when none is set. */
  alignment: TextAlignment | null
  /** `'default'` when none is set. */
  lineHeight: LineHeight | 'default' | null
}

export function paragraphFormatOf(state: EditorState): ParagraphFormat {
  const blocks = selectedBlocks(state.doc, state.selection)
  const shared = <T>(values: T[]): T | null => {
    const distinct = new Set(values)
    return distinct.size === 1 ? values[0]! : null
  }
  return {
    applicable: blocks.length > 0,
    alignment: shared(blocks.map(({ node }) => parseTextAlign(node.attrs.textAlign) ?? 'left')),
    lineHeight: shared(
      blocks.map(({ node }) => parseLineHeight(node.attrs.lineHeight) ?? ('default' as const))
    ),
  }
}

/**
 * The stock alignment extension, reading and writing only the four it knows.
 * Its shortcuts (Mod-Shift-L, E, R, J) are kept as they are.
 *
 * Left is stored as nothing, so `setTextAlign('left')` takes the attribute
 * off; a pasted `text-align` outside the set is read as none.
 */
export const WritingTextAlign = TextAlign.extend({
  addOptions() {
    return {
      ...this.parent?.(),
      types: [...PARAGRAPH_TYPES],
      alignments: [...TEXT_ALIGNMENTS],
      defaultAlignment: null,
    }
  },

  addGlobalAttributes() {
    return [
      {
        types: [...PARAGRAPH_TYPES],
        attributes: {
          textAlign: {
            default: null,
            parseHTML: (element) => parseTextAlign(element.style.textAlign),
            renderHTML: (attributes) => {
              const alignment = parseTextAlign(attributes.textAlign)
              return alignment ? { style: `text-align: ${alignment}` } : {}
            },
          },
        },
      },
    ]
  },

  addCommands() {
    return {
      ...this.parent?.(),
      setTextAlign: (alignment: string) => (props: CommandProps) =>
        (TEXT_ALIGNMENTS as readonly string[]).includes(alignment)
          ? setBlockAttribute('textAlign', parseTextAlign(alignment))(props)
          : false,
      unsetTextAlign: () => (props: CommandProps) => setBlockAttribute('textAlign', null)(props),
    }
  },
})

declare module '@tiptap/core' {
  interface Commands<ReturnType> {
    paragraphFormat: {
      /** In a list, sinks the item; elsewhere, every selected block one level in. */
      increaseIndent: () => ReturnType
      /** In a list, lifts the item; elsewhere, every selected block one level out. */
      decreaseIndent: () => ReturnType
      /** A line height from the set onto every selected block; null takes it off. */
      setLineHeight: (value: LineHeight | null) => ReturnType
    }
  }
}

/**
 * Indent and line spacing. Both render as inline style on the block and are
 * read back from what this editor renders, which is what keeps them through a
 * copy and paste inside the app.
 *
 * The indent step is drawn by the surface's stylesheet from `--writing-indent`
 * (WritingEditor.svelte), so a heading steps as far as the body text around it
 * rather than by its own larger em.
 *
 * Tab and Shift-Tab are not bound here: in a list they already move the item,
 * and outside one they belong to the focus order.
 */
export const ParagraphFormat = Extension.create({
  name: 'paragraphFormat',

  addGlobalAttributes() {
    return [
      {
        types: [...PARAGRAPH_TYPES],
        attributes: {
          indent: {
            default: null,
            parseHTML: (element) => parseIndent(element.getAttribute('data-indent')) || null,
            renderHTML: (attributes) => {
              const level = parseIndent(attributes.indent)
              return level
                ? { 'data-indent': String(level), style: `--writing-indent: ${level}` }
                : {}
            },
          },
          lineHeight: {
            default: null,
            parseHTML: (element) => parseLineHeight(element.style.lineHeight),
            renderHTML: (attributes) => {
              const value = parseLineHeight(attributes.lineHeight)
              return value ? { style: `line-height: ${value}` } : {}
            },
          },
        },
      },
    ]
  },

  addCommands() {
    return {
      increaseIndent: () => (props) =>
        inListItem(props.tr.selection)
          ? props.commands.sinkListItem('listItem')
          : shiftIndent(1)(props),
      decreaseIndent: () => (props) =>
        inListItem(props.tr.selection)
          ? props.commands.liftListItem('listItem')
          : shiftIndent(-1)(props),
      setLineHeight: (value) => (props) =>
        value === null || parseLineHeight(value) !== null
          ? setBlockAttribute('lineHeight', value)(props)
          : false,
    }
  },
})
