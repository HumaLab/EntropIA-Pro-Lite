import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  agentAssistantInstall,
  agentClientCreate,
  agentClientRevoke,
  connectorRecipes,
} from './agent-access'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

describe('assistant installation IPC', () => {
  beforeEach(() => invokeMock.mockReset().mockResolvedValue(undefined))

  it.each(['claude', 'codex', 'opencode', 'pi'] as const)(
    'installs %s with only its paired client id',
    async (assistant) => {
      await agentAssistantInstall(assistant, 'cl_test')
      expect(invokeMock).toHaveBeenCalledExactlyOnceWith('agent_assistant_install', {
        assistant,
        clientId: 'cl_test',
      })
    }
  )

  it('preserves pairing normalization and revocation payloads', async () => {
    await agentClientCreate('  Codex  ')
    await agentClientRevoke('cl_test')
    expect(invokeMock.mock.calls).toEqual([
      ['agent_client_create', { name: 'Codex' }],
      ['agent_client_revoke', { clientId: 'cl_test' }],
    ])
  })

  it('preserves all four copy recipes', () => {
    const recipes = connectorRecipes('cl_test')
    expect(recipes.map(({ id }) => id)).toEqual(['claude', 'codex', 'opencode', 'pi'])
    expect(recipes.every(({ line }) => line.includes('cl_test'))).toBe(true)
    expect(JSON.parse(recipes[2]!.line)).toEqual({
      mcp: {
        entropia: { type: 'local', command: ['entropia-agent-connector', '--client', 'cl_test'] },
      },
    })
    expect(JSON.parse(recipes[3]!.line)).toEqual({
      mcpServers: {
        entropia: { command: 'entropia-agent-connector', args: ['--client', 'cl_test'] },
      },
    })
  })
})
