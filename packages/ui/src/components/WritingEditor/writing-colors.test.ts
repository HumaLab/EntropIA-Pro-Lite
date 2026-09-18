import { describe, expect, it } from 'vitest'
import {
  PRINT_COLORS,
  WRITING_COLORS,
  colorLabelKey,
  highlightColorVar,
  parseWritingColor,
  textColorVar,
} from './writing-colors'

describe('the manuscript palette', () => {
  it('is a short list of names, not colours', () => {
    expect(WRITING_COLORS).toEqual([
      'gray',
      'red',
      'orange',
      'yellow',
      'green',
      'blue',
      'purple',
      'pink',
    ])
  })

  it('reads a name it knows and nothing else', () => {
    expect(parseWritingColor('red')).toBe('red')
    expect(parseWritingColor(' blue ')).toBeNull()
    expect(parseWritingColor('#ff0000')).toBeNull()
    expect(parseWritingColor('magenta')).toBeNull()
    expect(parseWritingColor(null)).toBeNull()
    expect(parseWritingColor(3)).toBeNull()
    // Inherited properties are not names.
    expect(parseWritingColor('toString')).toBeNull()
  })

  it('points each name at a theme token, so every theme draws it its own way', () => {
    expect(textColorVar('red')).toBe('var(--writing-text-red)')
    expect(highlightColorVar('yellow')).toBe('var(--writing-highlight-yellow)')
  })

  it('has one print colour per name for text and for highlight, as hex', () => {
    expect(Object.keys(PRINT_COLORS).sort()).toEqual([...WRITING_COLORS].sort())
    for (const name of WRITING_COLORS) {
      expect(PRINT_COLORS[name].text, name).toMatch(/^#[0-9a-f]{6}$/)
      expect(PRINT_COLORS[name].highlight, name).toMatch(/^#[0-9a-f]{6}$/)
    }
  })

  it('names the label each colour is announced by', () => {
    expect(colorLabelKey('gray')).toBe('colorGray')
    expect(colorLabelKey('purple')).toBe('colorPurple')
  })
})
