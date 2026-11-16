import { render, screen } from '@testing-library/svelte'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

import MetadataEditor from '../MetadataEditor.svelte'
import { stylesOf } from '../../__tests__/style-guards'

const styles = stylesOf(
  readFileSync(resolve(import.meta.dirname, '../MetadataEditor.svelte'), 'utf-8')
)

describe('MetadataEditor design-system controls', () => {
  it('edits keys and values through the package Input', () => {
    render(MetadataEditor, { props: { value: { Author: 'John' } } })

    for (const field of screen.getAllByRole('textbox')) {
      expect(field).toHaveClass('input-field__input')
    }
    expect(styles).not.toMatch(/\.metadata-editor__input\b/)
  })

  it('takes durations and letter spacing from tokens', () => {
    expect(styles).not.toMatch(/transition:[^;]*\b\d*\.?\d+m?s\b/)
    expect(styles).not.toMatch(/letter-spacing:\s*-?\d/)
  })
})
