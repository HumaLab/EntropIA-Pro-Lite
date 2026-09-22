import { describe, expect, it, vi } from 'vitest'
import { normalizeExternalUrl, openExternalUrl } from './external-links'

const { invokeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
}))

vi.mock('@tauri-apps/api/core', () => ({
  invoke: invokeMock,
}))

describe('external-links', () => {
  it('normalizes complete HTTP(S) URLs', () => {
    expect(normalizeExternalUrl('https://openrouter.ai/settings/keys')).toBe(
      'https://openrouter.ai/settings/keys'
    )
  })

  it('rejects non-HTTP(S) protocols', () => {
    expect(() => normalizeExternalUrl('file:///C:/Windows/System32/cmd.exe')).toThrow(
      'Only HTTP(S) URLs can be opened externally.'
    )
    expect(() => normalizeExternalUrl('javascript:alert(1)')).toThrow(
      'Only HTTP(S) URLs can be opened externally.'
    )
  })

  it('passes the exact Lite Store listing through unchanged', () => {
    expect(normalizeExternalUrl('ms-windows-store://pdp/?ProductId=9N328K9L95JD')).toBe(
      'ms-windows-store://pdp/?ProductId=9N328K9L95JD'
    )
  })

  it('rejects any other Store URI', () => {
    for (const url of [
      'ms-windows-store://pdp/?ProductId=9NBLGGH4NNS1',
      'ms-windows-store://pdp/?ProductId=9N328K9L95JD&cid=x',
      'ms-windows-store://pdp/?productid=9N328K9L95JD',
      ' ms-windows-store://pdp/?ProductId=9N328K9L95JD',
      'ms-windows-store://home',
    ]) {
      expect(() => normalizeExternalUrl(url)).toThrow('Only HTTP(S) URLs can be opened externally.')
    }
  })

  it('opens URLs through the Tauri external URL command', async () => {
    invokeMock.mockResolvedValue(undefined)

    await openExternalUrl('https://z.ai/manage-apikey/apikey-list')

    expect(invokeMock).toHaveBeenCalledWith('open_external_url', {
      url: 'https://z.ai/manage-apikey/apikey-list',
    })
  })
})
