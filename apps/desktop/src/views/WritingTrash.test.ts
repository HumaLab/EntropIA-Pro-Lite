import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import WritingTrash from './WritingTrash.svelte'
import { locale } from '$lib/i18n'
import type { WritingDocumentRow } from '$lib/writing'

afterEach(cleanup)
beforeEach(() => {
  locale.set('es')
})

const row = (id: string, title: string): WritingDocumentRow => ({
  id,
  title,
  document_type: 'article',
  status: 'trashed',
  schema_version: 1,
  current_content_json: '{}',
  revision: 1,
  created_at: 1,
  updated_at: 1,
})

it('lists the trashed manuscripts only when opened', async () => {
  const listTrashed = vi.fn().mockResolvedValue([row('d1', 'Borrador viejo')])
  render(WritingTrash, { listTrashed, onRestore: vi.fn() })

  expect(listTrashed).not.toHaveBeenCalled()
  await fireEvent.click(screen.getByRole('button', { name: 'Papelera' }))

  expect(await screen.findByText('Borrador viejo')).toBeInTheDocument()
  expect(screen.getByText(/^Eliminado el /)).toBeInTheDocument()
  expect(screen.getByRole('button', { name: 'Ocultar la papelera' })).toHaveAttribute(
    'aria-expanded',
    'true'
  )
})

it('says when the trash is empty', async () => {
  render(WritingTrash, { listTrashed: vi.fn().mockResolvedValue([]), onRestore: vi.fn() })

  await fireEvent.click(screen.getByRole('button', { name: 'Papelera' }))

  expect(await screen.findByText('La papelera está vacía.')).toBeInTheDocument()
})

it('restores a manuscript and drops it from the trash', async () => {
  const listTrashed = vi
    .fn()
    .mockResolvedValueOnce([row('d1', 'Borrador viejo'), row('d2', 'Otro')])
    .mockResolvedValueOnce([row('d2', 'Otro')])
  const onRestore = vi.fn().mockResolvedValue(undefined)
  render(WritingTrash, { listTrashed, onRestore })
  await fireEvent.click(screen.getByRole('button', { name: 'Papelera' }))

  await fireEvent.click(await screen.findByRole('button', { name: 'Restaurar «Borrador viejo»' }))

  expect(onRestore).toHaveBeenCalledWith('d1')
  expect(await screen.findByText('«Borrador viejo» volvió a la lista.')).toBeInTheDocument()
  expect(screen.queryByRole('button', { name: 'Restaurar «Borrador viejo»' })).toBeNull()
  expect(screen.getByRole('button', { name: 'Restaurar «Otro»' })).toBeInTheDocument()
})

it('keeps the manuscript listed when the restore fails', async () => {
  const listTrashed = vi.fn().mockResolvedValue([row('d1', 'Borrador viejo')])
  const onRestore = vi.fn().mockRejectedValue({ code: 'document_not_found', message: 'gone' })
  render(WritingTrash, { listTrashed, onRestore })
  await fireEvent.click(screen.getByRole('button', { name: 'Papelera' }))

  await fireEvent.click(await screen.findByRole('button', { name: 'Restaurar «Borrador viejo»' }))

  expect(await screen.findByRole('alert')).toHaveTextContent('No se pudo restaurar: gone')
  expect(screen.getByRole('button', { name: 'Restaurar «Borrador viejo»' })).toBeInTheDocument()
})

it('scrolls its own list instead of squeezing the document grid', () => {
  // The grid above is the page's scroll area in a full-height column: a trash
  // that may shrink or grow without bound pushed it down to zero height.
  const styles = readFileSync(resolve(import.meta.dirname, 'WritingTrash.svelte'), 'utf-8').replace(
    /\/\*[\s\S]*?\*\//g,
    ''
  )
  const rule = (selector: string) => {
    const at = styles.indexOf(selector)
    expect(at, selector).toBeGreaterThan(-1)
    return styles.slice(at, styles.indexOf('}', at))
  }
  expect(rule('.writing-trash {')).toMatch(/flex:\s*none/)
  expect(rule('.writing-trash__list {')).toMatch(/max-height:/)
  expect(rule('.writing-trash__list {')).toMatch(/overflow-y:\s*auto/)
})

it('follows a manuscript trashed from the list while it is open', async () => {
  const listTrashed = vi
    .fn()
    .mockResolvedValueOnce([row('d1', 'Borrador viejo')])
    .mockResolvedValueOnce([])
    .mockResolvedValueOnce([row('d1', 'Borrador viejo')])
  const onRestore = vi.fn().mockResolvedValue(undefined)
  const { rerender } = render(WritingTrash, { listTrashed, onRestore, listed: [] })
  await fireEvent.click(screen.getByRole('button', { name: 'Papelera' }))
  await fireEvent.click(await screen.findByRole('button', { name: 'Restaurar «Borrador viejo»' }))
  expect(await screen.findByText('«Borrador viejo» volvió a la lista.')).toBeInTheDocument()

  // The list trashes it again: the workspace list changes under the open trash.
  await rerender({ listTrashed, onRestore, listed: [row('d2', 'Otro')] })

  expect(
    await screen.findByRole('button', { name: 'Restaurar «Borrador viejo»' })
  ).toBeInTheDocument()
  expect(screen.queryByText('«Borrador viejo» volvió a la lista.')).toBeNull()
})
