import { existsSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, expect, it } from 'vitest'

/**
 * The Lite MSI must force the WebView2 bootstrapper on every install/update:
 * Windows Sandbox can expose the machine (HKLM) runtime key while the runtime
 * is still unavailable to the real user, so a registry-presence check alone
 * would skip a repair that is still needed. But the forced run exits with
 * 0x80040828 (-2147219416) when the runtime is already installed, which WiX
 * `Return="check"` treats as fatal (MSI 1722 + rollback).
 *
 * The wiring under test: the fragment points the forced deferred action at a
 * tiny repair wrapper (embedded via a `Binary` the pre-bundle hook prepares)
 * that normalizes only that one code to success, and the platform config runs
 * the Windows-only hook before bundling. These tests parse the real XML DOM
 * (not substrings) and cross-check the hook, the Binary source, and the
 * wrapper source so one of the three cannot move without the others.
 */

const SRC_TAURI = resolve(import.meta.dirname, '../../src-tauri')
// DESKTOP_ROOT is the selected workspace package root (apps/desktop). The
// pnpm --filter selector anchors exec CWD there (probed independently from
// repo root, apps/desktop and src-tauri); no claim is made about the
// bundler's own CWD. Never resolve the hook against src-tauri itself.
const DESKTOP_ROOT = resolve(SRC_TAURI, '..')
const FRAGMENT = readFileSync(
  resolve(SRC_TAURI, 'windows/fragments/force-webview2-bootstrapper.wxs'),
  'utf-8'
)
const WINDOWS_CONF = JSON.parse(
  readFileSync(resolve(SRC_TAURI, 'tauri.windows.conf.json'), 'utf-8')
) as { build?: { beforeBundleCommand?: string } }
const HOOK = readFileSync(resolve(SRC_TAURI, 'scripts/prepare-webview2-repair.ps1'), 'utf-8')
const WRAPPER = readFileSync(resolve(SRC_TAURI, 'windows/webview2-repair.rs'), 'utf-8')

const ACTION_ID = 'ForceInvokeWebView2Bootstrapper'
const WRAPPER_BINARY_ID = 'EntropiaWebView2Repair'
const ALREADY_INSTALLED = -2147219416

function fragmentDocument(): Document {
  // happy-dom's XML parser rejects `<![CDATA[ ... ]]>` (valid WiX for the
  // `NOT REMOVE` condition), so unwrap CDATA to its character data first.
  // The condition text itself is still asserted below.
  const withoutCdata = FRAGMENT.replace(/<!\[CDATA\[(.*?)\]\]>/gs, '$1')
  const doc = new DOMParser().parseFromString(withoutCdata, 'text/xml')
  const errors = doc.getElementsByTagName('parsererror')
  expect(errors.length, 'fragment must be well-formed XML').toBe(0)
  return doc
}

function singleElement(doc: Document, tag: string, idAttr: string, id: string): Element {
  const matches = Array.from(doc.getElementsByTagName(tag)).filter(
    (el) => el.getAttribute(idAttr) === id
  )
  expect(matches, `<${tag} ${idAttr}="${id}"> must appear exactly once`).toHaveLength(1)
  return matches[0]!
}

describe('forced WebView2 repair action', () => {
  it('runs the repair wrapper deferred with failure checking and the MSI arguments', () => {
    const action = singleElement(fragmentDocument(), 'CustomAction', 'Id', ACTION_ID)
    expect(action.getAttribute('BinaryKey')).toBe(WRAPPER_BINARY_ID)
    expect(action.getAttribute('Execute')).toBe('deferred')
    expect(action.getAttribute('Return')).toBe('check')
    expect(action.getAttribute('ExeCommand')).toBe('/silent /install')
  })

  it('embeds the hook-built wrapper Binary without copying the Tauri template', () => {
    const binary = singleElement(fragmentDocument(), 'Binary', 'Id', WRAPPER_BINARY_ID)
    const source = binary.getAttribute('SourceFile') ?? ''
    // Fragment-relative: windows/fragments -> src-tauri/target/webview2-installer.
    expect(source.replace(/\//g, '\\')).toContain('target\\webview2-installer\\repair.exe')
    expect(source).toContain('$(sys.SOURCEFILEDIR)')
  })

  it('keeps forced execution for installs/updates, including the Sandbox case', () => {
    const doc = fragmentDocument()
    const sequence = doc.getElementsByTagName('InstallExecuteSequence')[0]!
    const custom = Array.from(sequence.getElementsByTagName('Custom')).filter(
      (el) => el.getAttribute('Action') === ACTION_ID
    )
    expect(custom, 'forced action must stay sequenced').toHaveLength(1)
    expect(custom[0]!.getAttribute('Before')).toBe('InstallFinalize')
    expect(custom[0]!.textContent ?? '').toContain('NOT REMOVE')
    // The documented Sandbox/HKLM false-positive rationale must survive:
    // a registry-only skip would regress that workaround.
    expect(FRAGMENT).toContain('Windows Sandbox')
    expect(FRAGMENT).toContain('HKLM')
  })
})

describe('pre-bundle hook wiring', () => {
  it('runs the Windows-only hook before bundling without a policy bypass', () => {
    const command = WINDOWS_CONF.build?.beforeBundleCommand ?? ''
    expect(command).toContain('prepare-webview2-repair.ps1')
    expect(command).toContain('-NoProfile')
    expect(command).toContain('-NonInteractive')
    expect(command).toContain('-File')
    expect(command).not.toMatch(/Bypass|Unrestricted/i)
  })

  it('scopes the hook through the workspace package selector', () => {
    const command = WINDOWS_CONF.build?.beforeBundleCommand ?? ''
    // The MSI-2 failure proved a bare relative -File path cannot rely on the
    // bundler CWD (the actual CI CWD was never logged). The pnpm --filter
    // selector anchors exec CWD at the selected package root (apps/desktop,
    // probed independently from repo root, apps/desktop and src-tauri), so
    // the hook must run through the workspace scope, not a bare powershell.
    expect(command, 'beforeBundleCommand must start with the workspace selector').toMatch(
      /^pnpm --filter @entropia-pro\/desktop exec /
    )
    expect(command).toContain('powershell')
    const match = command.match(/-File\s+(['"]?)([^\s'"]+)\1/)
    expect(match, 'beforeBundleCommand must pass a -File script path').not.toBeNull()
    const resolved = resolve(DESKTOP_ROOT, match![2]!)
    expect(existsSync(resolved), resolved + ' must exist').toBe(true)
  })

  it('resolves the -File hook path against the selected package root to a real file', () => {
    const command = WINDOWS_CONF.build?.beforeBundleCommand ?? ''
    const match = command.match(/-File\s+(['"]?)([^\s'"]+)\1/)
    expect(match, 'beforeBundleCommand must pass a -File script path').not.toBeNull()
    const fileArg = match![2]!
    // Package-relative: pnpm --filter anchors exec at the selected package
    // root (apps/desktop), so the argument must start with src-tauri/ to
    // reach the real script.
    expect(fileArg, '-File argument must be package-relative under src-tauri/').toMatch(
      /^src-tauri\//
    )
    const resolved = resolve(DESKTOP_ROOT, fileArg)
    expect(existsSync(resolved), resolved + ' must exist').toBe(true)
    // The script itself must stay cwd-independent via $PSScriptRoot so the
    // fixed invocation path does not break staging derivation.
    expect(HOOK).toContain('$PSScriptRoot')
  })

  it('downloads the exact Tauri bootstrapper URL and gates on the build triple', () => {
    expect(HOOK).toContain('https://go.microsoft.com/fwlink/p/?LinkId=2124703')
    expect(HOOK).toContain('TAURI_ENV_TARGET_TRIPLE')
    for (const triple of ['x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc']) {
      expect(HOOK).toContain(triple)
    }
  })

  it('verifies Authenticode and compiles the wrapper with a static CRT', () => {
    expect(HOOK).toContain('Get-AuthenticodeSignature')
    expect(HOOK).toContain('Microsoft Corporation')
    expect(HOOK).toContain('WEBVIEW2_BOOTSTRAPPER_PATH')
    expect(HOOK).toContain('--target')
    expect(HOOK).toContain('+crt-static')
  })
})

describe('selective exit-code normalization', () => {
  it('maps only the already-installed code to success in the wrapper source', () => {
    expect(WRAPPER).toContain(String(ALREADY_INSTALLED))
    expect(WRAPPER).toContain('0x8004_0828')
    // The normalization must be selective: a blanket `0` return would hide
    // real bootstrapper failures from the MSI action.
    expect(WRAPPER).not.toMatch(/fn normalize_bootstrapper_exit_code\([^)]*\)[^{]*\{\s*0\s*\}/)
  })
})
