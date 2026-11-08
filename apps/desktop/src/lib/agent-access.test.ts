import { beforeEach, describe, expect, it, vi } from 'vitest'
import {
  agentAssistantConnect,
  agentClientCreate,
  agentClientRevoke,
  connectorRecipes,
  connectorInvocation,
  agentSetupInstruction,
} from './agent-access'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ invoke: invokeMock }))

describe('universal assistant instruction', () => {
  const program = 'C:\\Users\\José Pérez\\EntropIA Lite\\connector.exe'
  const location = { installed: true, path: program, kind: 'bundled' as const }

  it.each([null, 'investigacion'])('carries profile %s in every recipe', (profile) => {
    const suffix = profile ? ' --profile investigacion' : ''
    expect(connectorInvocation('cl_ab12', location, profile)).toBe(
      `& '${program}' --client cl_ab12${suffix}`
    )
    const recipes = connectorRecipes('cl_ab12', location, profile)
    expect(recipes[0]!.line).toContain(`-- '${program}' --client cl_ab12${suffix}`)
    expect(recipes[1]!.line).toBe(
      `codex mcp add entropia -- '${program}' --client cl_ab12${suffix}`
    )
    const args = ['--client', 'cl_ab12', ...(profile ? ['--profile', profile] : [])]
    expect(JSON.parse(recipes[2]!.line).mcp.entropia.command).toEqual([program, ...args])
    expect(JSON.parse(recipes[3]!.line).mcpServers.entropia).toEqual({ command: program, args })
  })

  it.each(['es', 'en'])('provides safe, self-contained %s instructions', (lang) => {
    for (const profile of [null, 'investigacion']) {
      const input = {
        program,
        clientId: 'cl_ab12',
        profile,
        name: 'Qwen',
        lang,
        token: 'eat_secret',
      }
      const text = agentSetupInstruction(input)
      const args = ['--client', 'cl_ab12', ...(profile ? ['--profile', profile] : [])]
      const json = text.split('```json\n')[1]!.split('\n```')[0]!
      expect(JSON.parse(json)).toEqual({ mcpServers: { entropia: { command: program, args } } })
      expect(text).toContain('[mcp_servers.entropia]')
      expect(text).toContain(
        'command = "C:\\\\Users\\\\José Pérez\\\\EntropIA Lite\\\\connector.exe"'
      )
      expect(text).toContain(`args = [${args.map((arg) => JSON.stringify(arg)).join(', ')}]`)
      expect(text).toContain(
        `& '${program}' --check --client cl_ab12${profile ? ' --profile investigacion' : ''}`
      )
      expect(text).toContain(`claude mcp add -s user entropia -- '${program}' --client cl_ab12`)
      expect(text).toContain(`codex mcp add entropia -- '${program}' --client cl_ab12`)
      expect(text).toContain('MCP stdio')
      expect(text).toContain('ok: ...')
      expect(text).toContain(lang === 'es' ? 'llavero del sistema' : 'OS keyring')
      expect(text).toContain(
        lang === 'es' ? 'sin editar nada más' : 'without editing anything else'
      )
      expect(text).not.toContain('eat_secret')
      if (!profile) expect(text).not.toContain('--profile')
    }
  })

  it('escapes quotes and control characters in TOML basic strings', () => {
    const text = agentSetupInstruction({
      program: '/a/"b\t\n\u0001\u007f',
      clientId: 'cl_a',
      profile: null,
      name: 'Grok',
      lang: 'en',
    })
    expect(text).toContain('command = "/a/\\"b\\t\\n\\u0001\\u007f"')
  })
})

describe('assistant installation IPC', () => {
  beforeEach(() => invokeMock.mockReset().mockResolvedValue(undefined))

  it.each(['claude', 'codex', 'opencode', 'pi'] as const)(
    'connects %s with only its name',
    async (assistant) => {
      await agentAssistantConnect(assistant, 'New assistant')
      expect(invokeMock).toHaveBeenCalledExactlyOnceWith('agent_assistant_connect', {
        assistant,
        name: 'New assistant',
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
