import { describe, expect, it, vi, beforeEach } from 'vitest'
import { invoke } from '@tauri-apps/api/core'
import {
  HELP_MANUAL_URL,
  HELP_NO_OPENROUTER_KEY,
  helpAsk,
  helpDiagnosticsPreview,
  helpStatus,
  isNoKeyError,
  openHelpManual,
} from './help'

describe('help backend wrappers', () => {
  beforeEach(() => {
    vi.mocked(invoke).mockReset()
  })

  it('publishes the online manual URL', () => {
    expect(HELP_MANUAL_URL).toBe(
      'https://humalab.github.io/EntropIA-Pro-Lite/manual-usuario/manual-usuario.html'
    )
  })

  it('names the missing-key rejection the backend sends', () => {
    expect(HELP_NO_OPENROUTER_KEY).toBe('help_no_openrouter_key')
  })

  it('helpStatus calls help_status and returns the key flag', async () => {
    vi.mocked(invoke).mockResolvedValue({ hasOpenrouterKey: true })

    await expect(helpStatus()).resolves.toEqual({ hasOpenrouterKey: true })
    expect(invoke).toHaveBeenCalledWith('help_status')
  })

  it('helpDiagnosticsPreview returns the exact report text', async () => {
    vi.mocked(invoke).mockResolvedValue('Producto: EntropIA Lite\n- algo')

    await expect(helpDiagnosticsPreview()).resolves.toBe('Producto: EntropIA Lite\n- algo')
    expect(invoke).toHaveBeenCalledWith('help_diagnostics_preview')
  })

  it('helpAsk sends question, history and the opted-in diagnostics', async () => {
    vi.mocked(invoke).mockResolvedValue({ answer: 'hacelo así', sources: [] })
    const history = [{ role: 'user' as const, content: 'hola' }]

    await helpAsk('¿Cómo importo?', history, 'reporte exacto')

    expect(invoke).toHaveBeenCalledWith('help_ask', {
      question: '¿Cómo importo?',
      history,
      diagnostics: 'reporte exacto',
    })
  })

  it('helpAsk leaves diagnostics out when the checkbox is off', async () => {
    vi.mocked(invoke).mockResolvedValue({ answer: 'hacelo así', sources: [] })

    await helpAsk('¿Cómo importo?', [], null)

    expect(invoke).toHaveBeenCalledWith('help_ask', {
      question: '¿Cómo importo?',
      history: [],
    })
    const args = vi.mocked(invoke).mock.calls[0]?.[1] as Record<string, unknown>
    expect('diagnostics' in args).toBe(false)
  })

  it('helpAsk sends at most the last 6 turns of history', async () => {
    vi.mocked(invoke).mockResolvedValue({ answer: 'hacelo así', sources: [] })
    const history = Array.from({ length: 10 }, (_, index) => ({
      role: (index % 2 === 0 ? 'user' : 'assistant') as 'user' | 'assistant',
      content: `turno ${index}`,
    }))

    await helpAsk('pregunta', history, null)

    const args = vi.mocked(invoke).mock.calls[0]?.[1] as {
      history: Array<{ content: string }>
    }
    expect(args.history).toHaveLength(6)
    expect(args.history[0]?.content).toBe('turno 4')
    expect(args.history[5]?.content).toBe('turno 9')
  })

  it('isNoKeyError recognizes the backend missing-key rejection', () => {
    expect(isNoKeyError('help_no_openrouter_key')).toBe(true)
    expect(isNoKeyError(new Error('help_no_openrouter_key'))).toBe(true)
    expect(isNoKeyError(new Error('invoke help_ask failed: help_no_openrouter_key'))).toBe(true)
    expect(isNoKeyError(new Error('boom'))).toBe(false)
    expect(isNoKeyError(null)).toBe(false)
    expect(isNoKeyError(undefined)).toBe(false)
  })

  it('openHelpManual opens the published manual through the desktop bridge', async () => {
    vi.mocked(invoke).mockResolvedValue(undefined)

    await openHelpManual()

    expect(invoke).toHaveBeenCalledWith('open_external_url', { url: HELP_MANUAL_URL })
  })
})
