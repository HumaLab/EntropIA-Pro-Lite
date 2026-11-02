// Unit tests for the pure parts of build-agent-connector.mjs (D15).
//
// The script itself runs two cargo builds and, on macOS, lipo, so the parts that
// decide *what* gets built and *where* it lands are separated out and pinned
// here. Everything else (a real compile, a real lipo) is proven by running the
// script, which every installer workflow does before the Tauri build.
//
// Usage: node --test apps/desktop/scripts/build-agent-connector.test.mjs

import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  buildTriples,
  cargoArtifactPath,
  cargoBuildArgs,
  parseRustcHost,
  parseVariant,
  sidecarPath,
  sidecarStem,
  universalSidecarPath,
} from './build-agent-connector.mjs'

const RUSTC_OUTPUT = `rustc 1.90.0 (1159e78c4 2025-09-14)
binary: rustc
commit-hash: 1159e78c4747b02ef996e55082b704c09b970588
commit-date: 2025-09-14
host: x86_64-unknown-linux-gnu
release: 1.90.0
LLVM version: 20.1.8
`

test('reads the host triple rustc reports', () => {
  assert.equal(parseRustcHost(RUSTC_OUTPUT), 'x86_64-unknown-linux-gnu')
  assert.equal(
    parseRustcHost('binary: rustc\nhost: aarch64-apple-darwin\nrelease: 1.90.0'),
    'aarch64-apple-darwin'
  )
})

test('fails loudly when rustc reports no host triple', () => {
  assert.throws(() => parseRustcHost('rustc 1.90.0\nbinary: rustc\n'), /host/)
})

test('names the sidecar Tauri expects, with .exe only on Windows', () => {
  assert.equal(
    sidecarPath('/repo', 'x86_64-pc-windows-msvc'),
    '/repo/apps/desktop/src-tauri/binaries/entropia-agent-connector-x86_64-pc-windows-msvc.exe'
  )
  assert.equal(
    sidecarPath('/repo', 'x86_64-unknown-linux-gnu'),
    '/repo/apps/desktop/src-tauri/binaries/entropia-agent-connector-x86_64-unknown-linux-gnu'
  )
  assert.equal(
    sidecarPath('/repo', 'aarch64-apple-darwin'),
    '/repo/apps/desktop/src-tauri/binaries/entropia-agent-connector-aarch64-apple-darwin'
  )
})

test('keeps the universal sidecar name the bundler looks for', () => {
  assert.equal(
    universalSidecarPath('/repo'),
    '/repo/apps/desktop/src-tauri/binaries/entropia-agent-connector-universal-apple-darwin'
  )
})

test('builds both Apple arches for a universal bundle, one otherwise', () => {
  assert.deepEqual(buildTriples('universal-apple-darwin'), [
    'aarch64-apple-darwin',
    'x86_64-apple-darwin',
  ])
  assert.deepEqual(buildTriples('aarch64-apple-darwin'), ['aarch64-apple-darwin'])
  assert.deepEqual(buildTriples('x86_64-pc-windows-msvc'), ['x86_64-pc-windows-msvc'])
})

test('reads the release artifact cargo writes for a target triple', () => {
  assert.equal(
    cargoArtifactPath('/repo', 'aarch64-apple-darwin'),
    '/repo/crates/entropia-agent-connector/target/aarch64-apple-darwin/release/entropia-agent-connector'
  )
  assert.equal(
    cargoArtifactPath('/repo', 'x86_64-pc-windows-msvc'),
    '/repo/crates/entropia-agent-connector/target/x86_64-pc-windows-msvc/release/entropia-agent-connector.exe'
  )
})

test('builds the connector from its own locked manifest for one target', () => {
  assert.deepEqual(cargoBuildArgs('x86_64-unknown-linux-gnu'), [
    'build',
    '--release',
    '--locked',
    '--manifest-path',
    'crates/entropia-agent-connector/Cargo.toml',
    '--target',
    'x86_64-unknown-linux-gnu',
  ])
})

test('gives Pro its own sidecar name so both .deb packages can be installed together', () => {
  assert.equal(sidecarStem('lite'), 'entropia-agent-connector')
  assert.equal(sidecarStem('pro'), 'entropia-pro-agent-connector')
  assert.equal(
    sidecarPath('/repo', 'x86_64-unknown-linux-gnu', 'pro'),
    '/repo/apps/desktop/src-tauri/binaries/entropia-pro-agent-connector-x86_64-unknown-linux-gnu'
  )
  assert.equal(
    sidecarPath('/repo', 'x86_64-pc-windows-msvc', 'pro'),
    '/repo/apps/desktop/src-tauri/binaries/entropia-pro-agent-connector-x86_64-pc-windows-msvc.exe'
  )
  assert.equal(
    universalSidecarPath('/repo', 'pro'),
    '/repo/apps/desktop/src-tauri/binaries/entropia-pro-agent-connector-universal-apple-darwin'
  )
})

test('cargo still writes the crate-named artifact for either variant', () => {
  assert.equal(
    cargoArtifactPath('/repo', 'x86_64-unknown-linux-gnu'),
    '/repo/crates/entropia-agent-connector/target/x86_64-unknown-linux-gnu/release/entropia-agent-connector'
  )
})

test('reads --variant, defaults to lite and refuses anything else', () => {
  assert.equal(parseVariant([]), 'lite')
  assert.equal(parseVariant(['--target', 'x86_64-unknown-linux-gnu']), 'lite')
  assert.equal(parseVariant(['--variant', 'pro']), 'pro')
  assert.equal(parseVariant(['--variant', 'lite']), 'lite')
  assert.throws(() => parseVariant(['--variant', 'store']), /--variant must be lite or pro/)
  assert.throws(() => parseVariant(['--variant']), /--variant must be lite or pro/)
})
