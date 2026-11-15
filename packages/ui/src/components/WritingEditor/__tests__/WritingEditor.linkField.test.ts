import { fireEvent, render, screen } from '@testing-library/svelte'
import { tick } from 'svelte'
import { describe, expect, it } from 'vitest'

import WritingEditor from '../WritingEditor.svelte'
import { WRITING_SCHEMA_VERSION, type CanonicalDocument } from '../document-contract'

/**
 * The one-field link disclosure is the shared Input (sm), not a field the
 * editor dresses itself; its behaviour (focus on open, Escape closes) stays.
 */
function manuscript(text: string): CanonicalDocument {
  return {
    schemaVersion: WRITING_SCHEMA_VERSION,
    doc: { type: 'doc', content: [{ type: 'paragraph', content: [{ type: 'text', text }] }] },
  }
}

async function openLinkField() {
  render(WritingEditor, { props: { document: manuscript('Hola mundo') } })
  await tick()
  const [trigger] = screen.getAllByRole('button', { name: 'Enlace', hidden: true })
  await fireEvent.click(trigger!)
  await tick()
  return screen.getByRole('textbox', { name: 'Enlace' }) as HTMLInputElement
}

describe('WritingEditor link field', () => {
  it('is the shared small Input', async () => {
    const field = await openLinkField()

    expect(field.type).toBe('url')
    expect(field.classList.contains('input-field__input')).toBe(true)
    expect(field.closest('.input-field--sm')).not.toBeNull()
  })

  it('takes the focus when it opens and closes on Escape', async () => {
    const field = await openLinkField()

    expect(document.activeElement).toBe(field)
    await fireEvent.keyDown(field, { key: 'Escape' })
    await tick()
    expect(screen.queryByRole('textbox', { name: 'Enlace' })).toBeNull()
  })
})
