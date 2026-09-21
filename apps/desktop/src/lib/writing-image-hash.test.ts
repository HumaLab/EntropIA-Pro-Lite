import { describe, expect, it } from 'vitest'
import { sha256Hex } from './writing-image-hash'

/**
 * The hashing entry point every managed-image path is built on (spec,
 * Managed Image Storage). Exercised against a fixed byte vector with a known
 * expected digest — the NIST SHA-256 test vector for "abc" — in the real
 * happy-dom environment this suite runs in, not assumed from documentation.
 */
describe('the hashing entry point this feature is built on', () => {
  it('computes the known SHA-256 digest of a fixed byte vector', async () => {
    const bytes = new TextEncoder().encode('abc')

    const hex = await sha256Hex(bytes)

    expect(hex).toBe('ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad')
  })

  it('is lowercase hex with no separator or prefix', async () => {
    const hex = await sha256Hex(new Uint8Array([0]))

    expect(hex).toMatch(/^[0-9a-f]{64}$/)
  })
})
