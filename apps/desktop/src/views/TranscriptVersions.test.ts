import { cleanup, fireEvent, render, screen } from '@testing-library/svelte'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import TranscriptVersions from './TranscriptVersions.svelte'
import { t, locale } from '$lib/i18n'
import type { TranscriptVersion } from '$lib/transcription'

afterEach(cleanup)
beforeEach(() => {
  vi.clearAllMocks()
  locale.set('es')
})

const version = (id: string, text: string, replacedAt: number): TranscriptVersion => ({
  id,
  textContent: text,
  model: 'assemblyai/best',
  language: 'es',
  durationMs: 1000,
  transcribedAt: replacedAt - 10,
  replacedAt,
})

function setup(onRestore = vi.fn().mockResolvedValue(undefined)) {
  render(TranscriptVersions, { assetId: 'audio-1', translate: t, onRestore })
  return onRestore
}

it('lists the replaced transcripts only when opened', async () => {
  vi.mocked(invoke).mockResolvedValue([version('v1', 'Entrevistador: hola\nresto', 20)])
  setup()
  expect(invoke).not.toHaveBeenCalled()
  await fireEvent.click(screen.getByRole('button', { name: 'Versiones anteriores' }))
  expect(invoke).toHaveBeenCalledWith('list_transcription_versions', { assetId: 'audio-1' })
  expect(await screen.findByText('Entrevistador: hola')).toBeInTheDocument()
  expect(screen.getByText(/assemblyai\/best · 25 caracteres/)).toBeInTheDocument()
  expect(screen.getByRole('button', { name: 'Ocultar versiones anteriores' })).toHaveAttribute(
    'aria-expanded',
    'true'
  )
})

it('says when there are no previous versions', async () => {
  vi.mocked(invoke).mockResolvedValue([])
  setup()
  await fireEvent.click(screen.getByRole('button', { name: 'Versiones anteriores' }))
  expect(await screen.findByText('Este audio no tiene versiones anteriores.')).toBeInTheDocument()
})

it('restores a version and reloads the list', async () => {
  vi.mocked(invoke)
    .mockResolvedValueOnce([version('v1', 'con roles', 20)])
    .mockResolvedValueOnce([version('v2', 'la que estaba', 30)])
  const onRestore = setup()
  await fireEvent.click(screen.getByRole('button', { name: 'Versiones anteriores' }))
  await fireEvent.click(await screen.findByRole('button', { name: 'Restaurar' }))
  expect(onRestore).toHaveBeenCalledWith(expect.objectContaining({ id: 'v1' }))
  expect(await screen.findByRole('status')).toHaveTextContent(
    'Versión restaurada. La transcripción que estaba quedó en «Versiones anteriores».'
  )
  expect(await screen.findByText('la que estaba')).toBeInTheDocument()
  expect(screen.queryByText('con roles')).toBeNull()
})

it('reports a failed restore and keeps the list', async () => {
  vi.mocked(invoke).mockResolvedValue([version('v1', 'con roles', 20)])
  setup(vi.fn().mockRejectedValue('disk full'))
  await fireEvent.click(screen.getByRole('button', { name: 'Versiones anteriores' }))
  await fireEvent.click(await screen.findByRole('button', { name: 'Restaurar' }))
  expect(await screen.findByRole('alert')).toHaveTextContent(
    'No se pudo restaurar la versión: disk full'
  )
  expect(screen.getByText('con roles')).toBeInTheDocument()
})
