# Manuscript Image Node Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A writer inserts an image from the Escritura toolbar (or pastes/drops one), sees it laid out inside the manuscript, resizes and captions it, and it survives save/reload/restart and exports to Markdown, HTML and DOCX without depending on the file it came from.

**Architecture:** One new block node (`writingImage`) with a plain ProseMirror node view, backed by a content-addressed file store (`writing-images.ts`) keyed by SHA-256 of the bytes. Intrinsic size is decoded from file header bytes (`image-dimensions.ts`, shared with the existing citation-quote image path). Three entry paths — toolbar picker, paste, drop — funnel through one import function and one insert command. The three exporters gain one switch case each, reusing the existing `ExportContext.images` pipeline.

**Tech Stack:** TypeScript, Svelte 5, Tiptap 2.26.4 / ProseMirror, `@tauri-apps/plugin-fs` + `plugin-dialog`, Vitest + happy-dom, `docx` 9.7.1.

**Spec:** `docs/superpowers/specs/2026-09-21-writing-image-node-design.md`

## Global Constraints

- Accepted formats are PNG, JPEG, and GIF only. WebP and SVG are refused before anything is stored or inserted (spec, Entry Paths).
- `src` on the node always holds a path relative to the shared data directory, never an absolute path or a drive letter (spec, Node Shape; Verification #5).
- No Tauri configuration changes: `asset:` protocol, `fs:scope`, `fs:allow-mkdir`/`allow-exists`/`allow-write-file`/`allow-remove`, and `dialog:allow-open` are already granted (spec, Managed Image Storage).
- `WRITING_SCHEMA_VERSION` (`packages/ui/src/components/WritingEditor/document-contract.ts:27`) is not bumped (spec, Serialization and Compatibility).
- Stored images are never deleted; no garbage collection (spec, Managed Image Storage; Non-Goals).
- `REQUIRED_BY_SPEC` in `export-fidelity.ts` is left untouched — `writingImage` is native everywhere, so it never becomes a refusal condition (spec, Export Guards).
- No new `writing.exportElement.*` i18n label is added — `writingImage` is `native` in all three formats, so it never enters `nameable()` (spec, Export Guards; `export-vocabulary.test.ts:36-46`).
- `packages/ui` (`@entropia/ui`) has **zero** `@tauri-apps/*` dependency (`packages/ui/package.json` has no such entry). Every Tauri call — the file dialog, `readFile`, `resolve_data_dir` — lives in `apps/desktop`, never in `packages/ui`. The editor package only ever receives capability functions as props (`resolveImage`, and the new `importImage`), exactly like the existing `resolveImage` prop already does for citation-quote images.
- Conventional commits only. Never add `Co-Authored-By` or any AI attribution line.
- Focused commands: `pnpm --filter @entropia/ui test -- <file>`, `pnpm --filter @entropia-pro/desktop test -- <file>`, `VITE_LOCAL_ML=0 pnpm --filter @entropia-pro/desktop typecheck`.
- Strict TDD: every production step is RED, then GREEN, observed before the next step.
- `docs/` is gitignored but plans/specs are tracked; this plan file itself would need `git add -f` if committed. Source files under `apps/**` and `packages/**` are tracked normally and need no `-f`.

---

### Task 1: Prove `crypto.subtle` and ship the hash helper

**Files:**
- Create: `apps/desktop/src/lib/writing-image-hash.ts`
- Test: `apps/desktop/src/lib/writing-image-hash.test.ts`

**Interfaces:**
- Produces: `sha256Hex(bytes: Uint8Array): Promise<string>` — lowercase hex, no `0x` prefix, matching the convention `apps/desktop/src-tauri/src/sync/blobs.rs:28-38` (`hex_lower`) already uses on the Rust side.

No code in this repository currently calls `crypto.subtle` (spec, Managed Image Storage), so this step proves it works in the actual `apps/desktop` Vitest `happy-dom` environment before anything is built on top of it, per the spec's explicit ordering.

- [ ] **Step 1: Write the failing test**

```ts
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @entropia-pro/desktop test -- src/lib/writing-image-hash.test.ts`
Expected: FAIL with `Failed to resolve import "./writing-image-hash"` (the module does not exist yet).

- [ ] **Step 3: Write the implementation — attempt A, `crypto.subtle`**

```ts
/**
 * SHA-256 of arbitrary bytes, as lowercase hex (spec, Managed Image Storage).
 *
 * Content addressing is what makes the same image inserted twice, in one
 * manuscript or many, resolve to one stored file: the name is the content.
 */

function bufferToHex(buffer: ArrayBuffer): string {
  const bytes = new Uint8Array(buffer)
  let hex = ''
  for (const byte of bytes) hex += byte.toString(16).padStart(2, '0')
  return hex
}

export async function sha256Hex(bytes: Uint8Array): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', bytes)
  return bufferToHex(digest)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @entropia-pro/desktop test -- src/lib/writing-image-hash.test.ts`
Expected: PASS.

**Declared contingency — if Step 4 FAILS instead** (e.g. `crypto.subtle is not a function` or `crypto.subtle` is `undefined` in this happy-dom build): do not weaken the test. Replace the implementation with a tested pure-JS fallback and keep the same public signature, so every caller (Task 3) is unaffected either way.

```ts
/** Fallback SHA-256 (FIPS 180-4), used only if `crypto.subtle` is unavailable
 *  in this environment — Step 4 of writing-image-hash.test.ts decides which
 *  branch ships. Kept in the same module so there is exactly one hash
 *  entry point regardless of which implementation backs it. */
const K = [
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]

function rightRotate(value: number, amount: number): number {
  return (value >>> amount) | (value << (32 - amount))
}

function sha256Fallback(bytes: Uint8Array): string {
  let h0 = 0x6a09e667
  let h1 = 0xbb67ae85
  let h2 = 0x3c6ef372
  let h3 = 0xa54ff53a
  let h4 = 0x510e527f
  let h5 = 0x9b05688c
  let h6 = 0x1f83d9ab
  let h7 = 0x5be0cd19

  const bitLength = bytes.length * 8
  let paddedLength = bytes.length + 1
  while (paddedLength % 64 !== 56) paddedLength++
  paddedLength += 8

  const padded = new Uint8Array(paddedLength)
  padded.set(bytes)
  padded[bytes.length] = 0x80
  const view = new DataView(padded.buffer)
  view.setUint32(paddedLength - 4, bitLength >>> 0, false)
  view.setUint32(paddedLength - 8, Math.floor(bitLength / 0x100000000), false)

  const w = new Uint32Array(64)
  for (let chunkStart = 0; chunkStart < paddedLength; chunkStart += 64) {
    for (let i = 0; i < 16; i++) w[i] = view.getUint32(chunkStart + i * 4, false)
    for (let i = 16; i < 64; i++) {
      const wim15 = w[i - 15]!
      const wim2 = w[i - 2]!
      const s0 = rightRotate(wim15, 7) ^ rightRotate(wim15, 18) ^ (wim15 >>> 3)
      const s1 = rightRotate(wim2, 17) ^ rightRotate(wim2, 19) ^ (wim2 >>> 10)
      w[i] = (w[i - 16]! + s0 + w[i - 7]! + s1) | 0
    }

    let a = h0, b = h1, c = h2, d = h3, e = h4, f = h5, g = h6, h = h7
    for (let i = 0; i < 64; i++) {
      const s1 = rightRotate(e, 6) ^ rightRotate(e, 11) ^ rightRotate(e, 25)
      const ch = (e & f) ^ (~e & g)
      const temp1 = (h + s1 + ch + K[i]! + w[i]!) | 0
      const s0 = rightRotate(a, 2) ^ rightRotate(a, 13) ^ rightRotate(a, 22)
      const maj = (a & b) ^ (a & c) ^ (b & c)
      const temp2 = (s0 + maj) | 0

      h = g; g = f; f = e; e = (d + temp1) | 0
      d = c; c = b; b = a; a = (temp1 + temp2) | 0
    }

    h0 = (h0 + a) | 0; h1 = (h1 + b) | 0; h2 = (h2 + c) | 0; h3 = (h3 + d) | 0
    h4 = (h4 + e) | 0; h5 = (h5 + f) | 0; h6 = (h6 + g) | 0; h7 = (h7 + h) | 0
  }

  return [h0, h1, h2, h3, h4, h5, h6, h7]
    .map((word) => (word >>> 0).toString(16).padStart(8, '0'))
    .join('')
}

export async function sha256Hex(bytes: Uint8Array): Promise<string> {
  if (typeof crypto !== 'undefined' && crypto.subtle) {
    return bufferToHex(await crypto.subtle.digest('SHA-256', bytes))
  }
  return sha256Fallback(bytes)
}
```

Add one more test asserting the fallback branch alone (`sha256Fallback` exported for the test only in this branch) matches the same "abc" vector, so both code paths are proven rather than just the one the environment happened to take.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src/lib/writing-image-hash.ts apps/desktop/src/lib/writing-image-hash.test.ts
git commit -m "feat(writing): add the SHA-256 hashing entry point for managed images"
```

---

### Task 2: `apps/desktop/src/lib/image-dimensions.ts`

**Files:**
- Create: `apps/desktop/src/lib/image-dimensions.ts`
- Modify: `apps/desktop/src/lib/export-images.ts:1-2,17,92-152` (remove the local PNG/JPEG decoders, re-export `imageSize`)
- Test: `apps/desktop/src/lib/image-dimensions.test.ts`

**Interfaces:**
- Produces: `imageSize(bytes: Uint8Array): { width: number; height: number } | null` — same signature `export-images.ts` already exposes today, now backed by a shared module and covering PNG, JPEG, and GIF. WebP is deliberately absent: the spec refuses the format at every entry path, so a decoder for it would be code no accepted file ever reaches.

`export-images.test.ts:5-28` already carries the exact PNG/JPEG byte-builder fixtures (`png(width, height)`, `jpeg(width, height)`) the current inline decoders are tested against, and that file is **not modified** by this task — its own assertions (lines 74-114) are the regression proof that the extraction changed nothing observable, because they exercise `imageSize` through the same re-exported name.

- [ ] **Step 1: Write the failing tests**

```ts
import { describe, expect, it } from 'vitest'
import { imageSize } from './image-dimensions'

/** A 2×3 PNG: the IHDR is what the size is read from, not the pixels.
 *  Identical to export-images.test.ts's fixture — the parity proof. */
function png(width: number, height: number): Uint8Array {
  const bytes = new Uint8Array(33)
  bytes.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], 0)
  const view = new DataView(bytes.buffer)
  view.setUint32(8, 13)
  bytes.set([0x49, 0x48, 0x44, 0x52], 12)
  view.setUint32(16, width)
  view.setUint32(20, height)
  return bytes
}

/** A JPEG whose first frame header declares the size. Identical to
 *  export-images.test.ts's fixture — the parity proof. */
function jpeg(width: number, height: number): Uint8Array {
  const bytes = new Uint8Array(20)
  bytes.set([0xff, 0xd8], 0)
  bytes.set([0xff, 0xfe, 0x00, 0x04, 0x00, 0x00], 2)
  bytes.set([0xff, 0xc0, 0x00, 0x11, 0x08], 8)
  const view = new DataView(bytes.buffer)
  view.setUint16(13, height)
  view.setUint16(15, width)
  return bytes
}

function gif(width: number, height: number): Uint8Array {
  const bytes = new Uint8Array(10)
  bytes.set([0x47, 0x49, 0x46, 0x38, 0x39, 0x61], 0)
  const view = new DataView(bytes.buffer)
  view.setUint16(6, width, true)
  view.setUint16(8, height, true)
  return bytes
}

describe('reading a size out of the bytes', () => {
  it('reads a PNG the same way the exporter always has', () => {
    expect(imageSize(png(40, 20))).toEqual({ width: 40, height: 20 })
  })

  it('reads a JPEG the same way the exporter always has', () => {
    expect(imageSize(jpeg(300, 150))).toEqual({ width: 300, height: 150 })
  })

  it('reads a GIF logical screen descriptor', () => {
    expect(imageSize(gif(64, 48))).toEqual({ width: 64, height: 48 })
  })

  it('refuses what it does not understand', () => {
    expect(imageSize(new Uint8Array([1, 2, 3, 4]))).toBeNull()
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @entropia-pro/desktop test -- src/lib/image-dimensions.test.ts`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Write the implementation**

```ts
/**
 * Intrinsic pixel size, decoded from a file's own header bytes rather than
 * measured by the browser (spec, Rendering and Layout). Shared by the
 * writing-image node's insert path and by citation-quote image export, which
 * this module was extracted out of (export-images.ts:114-152, pre-extraction).
 */

export function imageSize(bytes: Uint8Array): { width: number; height: number } | null {
  return pngSize(bytes) ?? jpegSize(bytes) ?? gifSize(bytes)
}

const PNG_SIGNATURE = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]

function pngSize(bytes: Uint8Array): { width: number; height: number } | null {
  if (bytes.length < 24) return null
  if (PNG_SIGNATURE.some((byte, index) => bytes[index] !== byte)) return null
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  return { width: view.getUint32(16), height: view.getUint32(20) }
}

/** The frame headers that declare a JPEG's size; the rest are skipped. */
function isFrameHeader(marker: number): boolean {
  if (marker < 0xc0 || marker > 0xcf) return false
  return marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc
}

function jpegSize(bytes: Uint8Array): { width: number; height: number } | null {
  if (bytes.length < 4 || bytes[0] !== 0xff || bytes[1] !== 0xd8) return null
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)

  let offset = 2
  while (offset + 9 < bytes.length) {
    if (bytes[offset] !== 0xff) {
      offset += 1
      continue
    }
    const marker = bytes[offset + 1] ?? 0
    if (isFrameHeader(marker)) {
      return { width: view.getUint16(offset + 7), height: view.getUint16(offset + 5) }
    }
    offset += 2 + view.getUint16(offset + 2)
  }

  return null
}

function gifSize(bytes: Uint8Array): { width: number; height: number } | null {
  if (bytes.length < 10) return null
  const header = String.fromCharCode(...bytes.subarray(0, 6))
  if (header !== 'GIF87a' && header !== 'GIF89a') return null
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength)
  return { width: view.getUint16(6, true), height: view.getUint16(8, true) }
}

```

Modify `export-images.ts`: delete `PNG_SIGNATURE`, `pngSize`, `isFrameHeader`, `jpegSize`, and the local `imageSize` (lines 107-152), and replace them with a re-export so every existing caller and test keeps working unchanged:

```ts
export { imageSize } from './image-dimensions'
```

(placed where the old `export function imageSize` was, at what was line 114; drop the now-unused `DataView`-only imports if any become unused — none do, `readFile`/`resolveStoredAssetPath`/`childrenOf` stay).

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @entropia-pro/desktop test -- src/lib/image-dimensions.test.ts src/lib/export-images.test.ts`
Expected: both files PASS — `export-images.test.ts` passing unmodified is the regression proof that PNG/JPEG output stayed byte-identical.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src/lib/image-dimensions.ts apps/desktop/src/lib/image-dimensions.test.ts apps/desktop/src/lib/export-images.ts
git commit -m "refactor(writing): extract image header size decoding, add gif"
```

---

### Task 3: `apps/desktop/src/lib/writing-images.ts`

**Files:**
- Create: `apps/desktop/src/lib/writing-images.ts`
- Test: `apps/desktop/src/lib/writing-images.test.ts`

**Interfaces:**
- Consumes: `sha256Hex` from `./writing-image-hash` (Task 1).
- Produces: `importWritingImage(bytes: Uint8Array, io?: WritingImageIo): Promise<ImportedWritingImage | null>` where `ImportedWritingImage = { path: string; mediaType: 'image/png' | 'image/jpeg' | 'image/gif' }`. `null` means the bytes are not one of the accepted formats — nothing is written, nothing is inserted.

Follows `writing-crops.ts:38-52`'s `CropIo`-style dependency injection so the test never touches real Tauri APIs, and corrects that module's one weakness noted in the spec: content addressing instead of `crypto.randomUUID()`, so the same bytes are stored once.

- [ ] **Step 1: Write the failing tests**

```ts
import { describe, expect, it, vi } from 'vitest'
import { importWritingImage, WRITING_IMAGES_DIR, type WritingImageIo } from './writing-images'

const PNG_HEADER = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]

function pngBytes(): Uint8Array {
  return new Uint8Array([...PNG_HEADER, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52, 0, 0, 0, 4, 0, 0, 0, 3])
}

function fakeIo(existingPaths: Set<string> = new Set()): WritingImageIo & {
  written: Map<string, Uint8Array>
} {
  const written = new Map<string, Uint8Array>()
  return {
    written,
    dataDir: vi.fn(async () => 'C:/datos'),
    join: vi.fn(async (...parts: string[]) => parts.join('/')),
    mkdir: vi.fn(async () => undefined),
    exists: vi.fn(async (path: string) => existingPaths.has(path) || written.has(path)),
    writeFile: vi.fn(async (path: string, bytes: Uint8Array) => {
      written.set(path, bytes)
    }),
  }
}

describe('importing a manuscript image', () => {
  it('stores it under writing-images/, named by its own hash', async () => {
    const io = fakeIo()

    const result = await importWritingImage(pngBytes(), io)

    expect(result).not.toBeNull()
    expect(result?.mediaType).toBe('image/png')
    expect(result?.path).toMatch(new RegExp(`^${WRITING_IMAGES_DIR}/[0-9a-f]{64}\\.png$`))
  })

  it('is a relative path with no drive letter or home directory', async () => {
    const result = await importWritingImage(pngBytes(), fakeIo())

    expect(result?.path.startsWith('/')).toBe(false)
    expect(result?.path).not.toMatch(/^[a-zA-Z]:/)
    expect(result?.path).not.toContain('\\')
  })

  it('writes the same bytes only once', async () => {
    const io = fakeIo()

    const first = await importWritingImage(pngBytes(), io)
    const second = await importWritingImage(pngBytes(), io)

    expect(second?.path).toBe(first?.path)
    expect(io.writeFile).toHaveBeenCalledTimes(1)
  })

  it('refuses a format it was not built to store, and stores nothing', async () => {
    const io = fakeIo()

    const result = await importWritingImage(new Uint8Array([1, 2, 3, 4]), io)

    expect(result).toBeNull()
    expect(io.writeFile).not.toHaveBeenCalled()
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @entropia-pro/desktop test -- src/lib/writing-images.test.ts`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Write the implementation**

```ts
import { invoke } from '@tauri-apps/api/core'
import { join } from '@tauri-apps/api/path'
import { exists, mkdir, writeFile } from '@tauri-apps/plugin-fs'
import { sha256Hex } from './writing-image-hash'

/**
 * Content-addressed storage for manuscript images (spec, Managed Image
 * Storage). One import function for every entry path: the toolbar picker,
 * paste, and drop all call this, so storage and deduplication have one
 * implementation. Follows writing-crops.ts's pattern and fixes its one gap:
 * the file name is the content's own hash, not a fresh UUID per insertion.
 */

export const WRITING_IMAGES_DIR = 'writing-images'

export type WritingImageMediaType = 'image/png' | 'image/jpeg' | 'image/gif'

const EXTENSIONS: Record<WritingImageMediaType, string> = {
  'image/png': 'png',
  'image/jpeg': 'jpg',
  'image/gif': 'gif',
}

export interface WritingImageIo {
  dataDir(): Promise<string>
  join(...parts: string[]): Promise<string>
  mkdir(path: string): Promise<void>
  exists(path: string): Promise<boolean>
  writeFile(path: string, bytes: Uint8Array): Promise<void>
}

const tauriIo: WritingImageIo = {
  dataDir: () => invoke<string>('resolve_data_dir'),
  join: (...parts) => join(...parts),
  mkdir: (path) => mkdir(path, { recursive: true }),
  exists: (path) => exists(path),
  writeFile: (path, bytes) => writeFile(path, bytes),
}

/** The media type detected from the bytes' own header, never from a
 *  filename — never PNG/JPEG/GIF just because the extension said so. */
export function detectWritingImageType(bytes: Uint8Array): WritingImageMediaType | null {
  if (
    bytes.length >= 8 &&
    [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a].every((byte, index) => bytes[index] === byte)
  ) {
    return 'image/png'
  }
  if (bytes.length >= 3 && bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) {
    return 'image/jpeg'
  }
  if (
    bytes.length >= 6 &&
    bytes[0] === 0x47 &&
    bytes[1] === 0x49 &&
    bytes[2] === 0x46 &&
    bytes[3] === 0x38 &&
    (bytes[4] === 0x37 || bytes[4] === 0x39) &&
    bytes[5] === 0x61
  ) {
    return 'image/gif'
  }
  return null
}

export interface ImportedWritingImage {
  /** Relative to the data directory: `writing-images/<sha256>.<ext>`. */
  path: string
  mediaType: WritingImageMediaType
}

/**
 * Reads the bytes' own hash and writes them once, at a name only their
 * content decides. `null` means the bytes are not an accepted format:
 * nothing is written, nothing is returned to insert (spec, Failure Handling).
 */
export async function importWritingImage(
  bytes: Uint8Array,
  io: WritingImageIo = tauriIo
): Promise<ImportedWritingImage | null> {
  const mediaType = detectWritingImageType(bytes)
  if (!mediaType) return null

  const hash = await sha256Hex(bytes)
  const extension = EXTENSIONS[mediaType]
  const relativePath = `${WRITING_IMAGES_DIR}/${hash}.${extension}`

  const dataDir = await io.dataDir()
  const root = await io.join(dataDir, WRITING_IMAGES_DIR)
  await io.mkdir(root)

  const absolute = await io.join(dataDir, relativePath)
  if (!(await io.exists(absolute))) {
    await io.writeFile(absolute, bytes)
  }

  return { path: relativePath, mediaType }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @entropia-pro/desktop test -- src/lib/writing-images.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src/lib/writing-images.ts apps/desktop/src/lib/writing-images.test.ts
git commit -m "feat(writing): store manuscript images content-addressed by sha256"
```

---

### Task 4: The `writingImage` node

**Files:**
- Modify: `packages/ui/src/components/WritingEditor/extensions.ts` (add the `WritingImage` node and thread a new `importImage` option through `createWritingExtensions`)
- Modify: `packages/ui/src/components/WritingEditor/trailing-paragraph.ts:21`
- Modify: `packages/ui/src/components/WritingEditor/WritingEditor.svelte:169` (thread the new `importImage` prop into `createWritingExtensions`)
- Modify: `packages/ui/src/components/WritingEditor/WritingEditor.types.ts` (add the `importImage` prop)
- Test: `packages/ui/src/components/WritingEditor/writing-image.test.ts`

**Interfaces:**
- Produces: node name `writingImage`; command `insertWritingImage(attrs: { src: string; alt?: string | null; title?: string | null; width?: number | null; height?: number | null; align?: 'left' | 'center' | 'right' }): ReturnType`; `WritingExtensionOptions.importImage?: (bytes: Uint8Array) => Promise<{ path: string; width: number; height: number } | null>` (consumed by Task 7's paste/drop plugin; unused until then — this task only wires the plumbing).

**Known transient state:** `export-fidelity.test.ts:30-33` asserts every schema node has a `NODE_FIDELITY` row. Adding `writingImage` to the schema here, before Task 8 registers it in `NODE_FIDELITY`, makes that one assertion fail between this task and Task 8. This is expected and resolved by Task 8's Step 3 — call it out in the commit body so it is not mistaken for a regression.

- [ ] **Step 1: Write the failing tests**

```ts
import { Editor } from '@tiptap/core'
import { afterEach, describe, expect, it } from 'vitest'
import { createWritingExtensions } from './extensions'
import { emptyDocument } from './document-contract'

let editor: Editor | undefined

afterEach(() => {
  editor?.destroy()
  editor = undefined
})

function mount() {
  const element = document.createElement('div')
  document.body.appendChild(element)
  editor = new Editor({ element, extensions: createWritingExtensions(), content: emptyDocument().doc })
  return editor
}

describe('the writingImage node', () => {
  it('is created by insertWritingImage, carrying its attrs', () => {
    const instance = mount()

    instance
      .chain()
      .focus()
      .insertWritingImage({ src: 'writing-images/abc.png', width: 300, height: 150 })
      .run()

    const json = instance.getJSON()
    const figure = json.content?.find((node) => node.type === 'writingImage')
    expect(figure?.attrs).toMatchObject({
      src: 'writing-images/abc.png',
      width: 300,
      height: 150,
      align: 'center',
    })
  })

  it('inserts at the cursor without replacing an existing text selection', () => {
    const instance = mount()
    instance.chain().focus().insertContent('hola mundo').run()
    instance.commands.setTextSelection({ from: 1, to: 6 }) // selects "hola "

    instance.chain().insertWritingImage({ src: 'writing-images/abc.png' }).run()

    const text = instance.getText()
    expect(text).toContain('mundo')
  })

  it('round-trips figure[data-writing-image] back into the node, caption included', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    const before = instance.getJSON()

    const rebuilt = new Editor({
      element: (() => {
        const el = document.createElement('div')
        document.body.appendChild(el)
        return el
      })(),
      extensions: createWritingExtensions(),
      content: before,
    })

    expect(rebuilt.getJSON()).toEqual(before)
    rebuilt.destroy()
  })

  it('does not parse a bare img[src] as a writingImage', () => {
    const instance = mount()

    instance.chain().focus().insertContent('<img src="https://example.org/remote.png">').run()

    const names: string[] = []
    instance.state.doc.forEach((node) => names.push(node.type.name))
    expect(names).not.toContain('writingImage')
  })

  it('joins the trailing-paragraph TRAPPING set: a caret always has somewhere to land after it', () => {
    const instance = mount()

    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()

    const names: string[] = []
    instance.state.doc.forEach((node) => names.push(node.type.name))
    expect(names.at(-1)).toBe('paragraph')
  })

  it('Enter inside the caption exits the node instead of splitting it', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    // Land inside the (empty) caption of the figure just inserted.
    const figurePos = instance.state.doc.content.size - 2
    instance.commands.setTextSelection(figurePos)

    instance.commands.keyboardShortcut('Enter')

    const names: string[] = []
    instance.state.doc.forEach((node) => names.push(node.type.name))
    expect(names.filter((name) => name === 'writingImage')).toHaveLength(1)
    expect(names.filter((name) => name === 'paragraph').length).toBeGreaterThanOrEqual(1)
  })

  it('Backspace at the start of an empty caption selects the figure', () => {
    const instance = mount()
    instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()
    const figureStart = instance.state.doc.content.size - 2
    instance.commands.setTextSelection(figureStart)

    instance.commands.keyboardShortcut('Backspace')

    expect(instance.state.selection.toJSON().type).toBe('node')
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @entropia/ui test -- src/components/WritingEditor/writing-image.test.ts`
Expected: FAIL — `insertWritingImage` does not exist, `writingImage` node is unregistered.

- [ ] **Step 3: Write the implementation**

In `extensions.ts`, add after `ZoteroCitation` (before `WritingExtensionOptions`):

```ts
export interface WritingImageAttrs {
  src: string
  alt: string | null
  title: string | null
  width: number | null
  height: number | null
  align: 'left' | 'center' | 'right'
}

/**
 * A manuscript image (writing-image-node-design.md). Content-bearing, not an
 * atom: `content: 'inline*'` is the caption, kept editable like any other
 * prose rather than an attribute, so it is searched, counted and undone
 * character by character. `src` is always a path relative to the shared data
 * directory (writing-images.ts), never an absolute one.
 */
export const WritingImage = Node.create<{
  resolveImage: ((source: string) => string) | null
  importImage: ((bytes: Uint8Array) => Promise<{ path: string; width: number; height: number } | null>) | null
}>({
  name: 'writingImage',
  group: 'block',
  content: 'inline*',
  draggable: true,
  selectable: true,

  addOptions() {
    return { resolveImage: null, importImage: null }
  },

  addAttributes() {
    return {
      src: { default: null },
      alt: { default: null },
      title: { default: null },
      width: { default: null },
      height: { default: null },
      align: { default: 'center' },
    }
  },

  // Matches only what this node itself emits, so a copy within or between
  // manuscripts round-trips exactly. Deliberately not `img[src]`: HTML from
  // elsewhere carries remote or foreign-filesystem sources this archive does
  // not hold, and the paste plugin (Task 7) — not this rule — is what turns
  // those into a managed copy or drops them.
  parseHTML() {
    return [{ tag: 'figure[data-writing-image]' }]
  },

  renderHTML({ node, HTMLAttributes }) {
    const attributes = mergeAttributes(
      { 'data-writing-image': '', 'data-align': node.attrs.align },
      HTMLAttributes
    )
    const src = typeof node.attrs.src === 'string' ? node.attrs.src : ''
    const imgAttrs: Record<string, unknown> = {
      src: this.options.resolveImage ? this.options.resolveImage(src) : src,
      alt: node.attrs.alt ?? '',
      title: node.attrs.title ?? '',
    }
    if (typeof node.attrs.width === 'number') imgAttrs.width = node.attrs.width
    return ['figure', attributes, ['img', imgAttrs], ['figcaption', 0]]
  },

  addCommands() {
    return {
      insertWritingImage:
        (attrs: { src: string } & Partial<Omit<WritingImageAttrs, 'src'>>) =>
        ({ commands }: { commands: import('@tiptap/core').SingleCommands }) =>
          commands.insertContent({
            type: this.name,
            attrs: {
              alt: null,
              title: null,
              width: null,
              height: null,
              align: 'center',
              ...attrs,
            },
          }),
    }
  },

  addKeyboardShortcuts() {
    return {
      // content: 'inline*' makes this a textblock, so ProseMirror's default
      // splitBlock would create a second writingImage with no src — a node
      // pointing at no stored bytes. Enter exits the figure instead.
      Enter: () => {
        if (!this.editor.isActive(this.name)) return false
        const { $from } = this.editor.state.selection
        const after = $from.after()
        return this.editor
          .chain()
          .insertContentAt(after, { type: 'paragraph' })
          .setTextSelection(after + 1)
          .run()
      },
      // At the start of the caption, Backspace would otherwise join the
      // figure into the block before it. Select the figure instead.
      Backspace: () => {
        if (!this.editor.isActive(this.name)) return false
        const { $from, empty } = this.editor.state.selection
        if (!empty || $from.parentOffset !== 0) return false
        return this.editor.commands.setNodeSelection($from.before())
      },
    }
  },
})
```

Add `import { Uint8Array as _ }` is unnecessary; `Uint8Array` is a global. Add the new option to `WritingExtensionOptions` and thread it, and register the node:

```ts
export interface WritingExtensionOptions {
  placeholder?: string
  resolveImage?: (source: string) => string
  /** Imports raw bytes into managed storage and returns the relative path
   *  and intrinsic size, or null when the bytes are not an accepted format.
   *  Only the paste/drop plugin (Task 7) calls this — the toolbar path
   *  (Task 6) imports through the app layer directly and calls
   *  `insertWritingImage` with an already-resolved path. */
  importImage?: (bytes: Uint8Array) => Promise<{ path: string; width: number; height: number } | null>
}
```

```ts
export function createWritingExtensions(options: WritingExtensionOptions = {}) {
  return [
    // …unchanged entries…
    WritingImage.configure({
      resolveImage: options.resolveImage ?? null,
      importImage: options.importImage ?? null,
    }),
    UniqueCitationIds,
    TrailingParagraph,
    SearchHighlight,
  ]
}
```

In `trailing-paragraph.ts:21`:

```ts
const TRAPPING = new Set(['table', 'blockquote', 'codeBlock', 'footnotes', 'horizontalRule', 'writingImage'])
```

In `WritingEditor.types.ts`, add to `WritingEditorProps`:

```ts
  /** Imports pasted or dropped image bytes into managed storage. Without it,
   *  paste and drop of an image do nothing — the toolbar picker (Task 6) does
   *  not need it, since the app already has the bytes by the time it calls
   *  `insertImage`. */
  importImage?: (bytes: Uint8Array) => Promise<{ path: string; width: number; height: number } | null>
```

In `WritingEditor.svelte`, destructure `importImage` alongside `resolveImage` in the props block (`let { …, resolveImage, importImage, … }`), and change line 169 to:

```ts
      extensions: createWritingExtensions({ placeholder, resolveImage, importImage }),
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @entropia/ui test -- src/components/WritingEditor/writing-image.test.ts src/components/WritingEditor/commands.test.ts`
Expected: PASS. (`export-fidelity.test.ts` in `apps/desktop` is expected RED until Task 8 — do not "fix" it here.)

- [ ] **Step 5: Commit**

```bash
git add packages/ui/src/components/WritingEditor/extensions.ts packages/ui/src/components/WritingEditor/trailing-paragraph.ts packages/ui/src/components/WritingEditor/WritingEditor.svelte packages/ui/src/components/WritingEditor/WritingEditor.types.ts packages/ui/src/components/WritingEditor/writing-image.test.ts
git commit -m "feat(writing): add the writingImage node, its insert command and keymaps

export-fidelity.test.ts in apps/desktop is expected to fail until the
export task registers writingImage in NODE_FIDELITY."
```

---

### Task 5: The node view and resizing

**Files:**
- Modify: `packages/ui/src/components/WritingEditor/extensions.ts` (add `addNodeView` to `WritingImage`)
- Create: `packages/ui/src/components/WritingEditor/writing-image-resize.ts`
- Modify: `packages/ui/src/components/WritingEditor/WritingEditor.svelte` (`<style>` block, near the existing `[data-document-citation] img` rule)
- Test: `packages/ui/src/components/WritingEditor/writing-image-resize.test.ts`

**Interfaces:**
- Consumes: nothing from earlier tasks beyond the `writingImage` node existing.
- Produces: `MIN_WRITING_IMAGE_WIDTH: number`; `clampWritingImageWidth(desiredWidth: number, aspectRatio: number, availableWidth: number): { width: number; height: number } | null`.

Per the spec, the cap the node view applies is the one measurement taken from live layout, and it is taken only while a handle is dragged — untestable in happy-dom. What is tested directly, with numbers, is the pure clamping arithmetic.

- [ ] **Step 1: Write the failing test**

```ts
import { describe, expect, it } from 'vitest'
import { clampWritingImageWidth, MIN_WRITING_IMAGE_WIDTH } from './writing-image-resize'

describe('clamping a dragged image width', () => {
  it('keeps the aspect ratio', () => {
    const clamped = clampWritingImageWidth(400, 2, 800)
    expect(clamped).toEqual({ width: 400, height: 200 })
  })

  it('never returns more than the available width', () => {
    const clamped = clampWritingImageWidth(900, 2, 500)
    expect(clamped).toEqual({ width: 500, height: 250 })
  })

  it('refuses a negative width', () => {
    expect(clampWritingImageWidth(-10, 2, 500)).toBeNull()
  })

  it('refuses a zero width', () => {
    expect(clampWritingImageWidth(0, 2, 500)).toBeNull()
  })

  it('refuses a width below the usable minimum', () => {
    expect(clampWritingImageWidth(MIN_WRITING_IMAGE_WIDTH - 1, 2, 500)).toBeNull()
  })

  it('accepts exactly the minimum', () => {
    expect(clampWritingImageWidth(MIN_WRITING_IMAGE_WIDTH, 1, 500)).toEqual({
      width: MIN_WRITING_IMAGE_WIDTH,
      height: MIN_WRITING_IMAGE_WIDTH,
    })
  })

  it('refuses a non-finite or non-positive aspect ratio or available width', () => {
    expect(clampWritingImageWidth(200, 0, 500)).toBeNull()
    expect(clampWritingImageWidth(200, 2, 0)).toBeNull()
    expect(clampWritingImageWidth(200, Number.NaN, 500)).toBeNull()
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @entropia/ui test -- src/components/WritingEditor/writing-image-resize.test.ts`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Write the minimal implementation, then the node view**

```ts
/**
 * The pure arithmetic behind resizing a writingImage (spec, Node View and
 * Resizing). A width below the minimum or outside a positive range is
 * refused — a null return — rather than silently floored: the node view's
 * drag handler is what keeps the visible handle from ever asking for less,
 * this function only has to refuse an out-of-contract request.
 */
export const MIN_WRITING_IMAGE_WIDTH = 80

export function clampWritingImageWidth(
  desiredWidth: number,
  aspectRatio: number,
  availableWidth: number
): { width: number; height: number } | null {
  if (
    !Number.isFinite(desiredWidth) ||
    !Number.isFinite(aspectRatio) ||
    !Number.isFinite(availableWidth)
  ) {
    return null
  }
  if (aspectRatio <= 0 || availableWidth <= 0) return null
  if (desiredWidth <= 0 || desiredWidth < MIN_WRITING_IMAGE_WIDTH) return null

  const width = Math.min(desiredWidth, availableWidth)
  return { width: Math.round(width), height: Math.round(width / aspectRatio) }
}
```

Add `addNodeView` to `WritingImage` in `extensions.ts` (first `addNodeView` in the repo — plain ProseMirror DOM, not Svelte, so it adds no rendering dependency):

```ts
  addNodeView() {
    return ({ node, getPos, editor }) => {
      const figure = document.createElement('figure')
      figure.dataset.writingImage = ''
      figure.dataset.align = node.attrs.align ?? 'center'

      const img = document.createElement('img')
      img.contentEditable = 'false'
      img.draggable = false
      img.alt = node.attrs.alt ?? ''
      img.title = node.attrs.title ?? ''
      if (typeof node.attrs.width === 'number') img.width = node.attrs.width
      const src = typeof node.attrs.src === 'string' ? node.attrs.src : ''
      img.src = this.options.resolveImage ? this.options.resolveImage(src) : src

      // Everything outside contentDOM (the figcaption below) must refuse the
      // caret, or click-to-select on the image becomes unreliable.
      const chrome = document.createElement('div')
      chrome.contentEditable = 'false'
      chrome.className = 'writing-editor__image-chrome'

      const alignGroup = document.createElement('div')
      alignGroup.className = 'writing-editor__image-align'
      const attrPos = () => (typeof getPos === 'function' ? getPos() : null)
      ;(['left', 'center', 'right'] as const).forEach((align) => {
        const button = document.createElement('button')
        button.type = 'button'
        button.textContent = align
        button.addEventListener('click', () => {
          const pos = attrPos()
          if (pos === null || pos === undefined) return
          editor.view.dispatch(editor.state.tr.setNodeAttribute(pos, 'align', align))
        })
        alignGroup.appendChild(button)
      })

      const altButton = document.createElement('button')
      altButton.type = 'button'
      altButton.textContent = 'Alt/Título'
      altButton.addEventListener('click', () => {
        const pos = attrPos()
        if (pos === null || pos === undefined) return
        const nextAlt = window.prompt('Texto alternativo', node.attrs.alt ?? '')
        if (nextAlt === null) return
        const nextTitle = window.prompt('Título', node.attrs.title ?? '')
        editor.view.dispatch(
          editor.state.tr
            .setNodeAttribute(pos, 'alt', nextAlt)
            .setNodeAttribute(pos, 'title', nextTitle ?? node.attrs.title ?? '')
        )
      })

      const handle = document.createElement('button')
      handle.type = 'button'
      handle.className = 'writing-editor__image-handle'
      handle.setAttribute('aria-label', 'Redimensionar imagen')

      let dragStartX = 0
      let dragStartWidth = typeof node.attrs.width === 'number' ? node.attrs.width : img.naturalWidth

      function currentAspect(): number {
        const width = typeof node.attrs.width === 'number' ? node.attrs.width : img.naturalWidth || 1
        const height = typeof node.attrs.height === 'number' ? node.attrs.height : img.naturalHeight || 1
        return width / (height || 1)
      }

      const onPointerMove = (event: PointerEvent) => {
        const available = figure.parentElement?.clientWidth ?? dragStartWidth
        const clamped = clampWritingImageWidth(
          dragStartWidth + (event.clientX - dragStartX),
          currentAspect(),
          available
        )
        if (clamped) img.width = clamped.width
      }
      const onPointerUp = (event: PointerEvent) => {
        window.removeEventListener('pointermove', onPointerMove)
        window.removeEventListener('pointerup', onPointerUp)
        const pos = attrPos()
        if (pos === null || pos === undefined) return
        const available = figure.parentElement?.clientWidth ?? dragStartWidth
        const clamped = clampWritingImageWidth(
          dragStartWidth + (event.clientX - dragStartX),
          currentAspect(),
          available
        )
        if (!clamped) return
        editor.view.dispatch(
          editor.state.tr
            .setNodeAttribute(pos, 'width', clamped.width)
            .setNodeAttribute(pos, 'height', clamped.height)
        )
      }
      handle.addEventListener('pointerdown', (event) => {
        dragStartX = event.clientX
        dragStartWidth = typeof node.attrs.width === 'number' ? node.attrs.width : img.width
        window.addEventListener('pointermove', onPointerMove)
        window.addEventListener('pointerup', onPointerUp)
      })

      chrome.append(alignGroup, altButton, handle)

      const figcaption = document.createElement('figcaption')
      figure.append(img, chrome, figcaption)

      return {
        dom: figure,
        contentDOM: figcaption,
        update: (updated) => {
          if (updated.type.name !== 'writingImage') return false
          figure.dataset.align = updated.attrs.align ?? 'center'
          img.alt = updated.attrs.alt ?? ''
          img.title = updated.attrs.title ?? ''
          if (typeof updated.attrs.width === 'number') img.width = updated.attrs.width
          return true
        },
      }
    }
  },
```

Import `clampWritingImageWidth` from `./writing-image-resize` at the top of `extensions.ts`.

In `WritingEditor.svelte`'s `<style>` block, next to the existing `[data-document-citation] img` rule (around line 1818): the stylesheet, not the stored attribute, is what keeps the figure inside the margins (spec, Rendering and Layout).

```css
  /* A manuscript image. max-width clamps a stored width wider than the
     column — from a narrower window, a different variant, or a hand-edited
     document — instead of overflowing (writing-image-node-design.md). */
  :global(.writing-editor__surface [data-writing-image] img) {
    display: block;
    max-width: 100%;
    height: auto;
  }

  :global(.writing-editor__surface [data-writing-image][data-align='left'] img) {
    margin: 0 auto 0 0;
  }

  :global(.writing-editor__surface [data-writing-image][data-align='center'] img) {
    margin: 0 auto;
  }

  :global(.writing-editor__surface [data-writing-image][data-align='right'] img) {
    margin: 0 0 0 auto;
  }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @entropia/ui test -- src/components/WritingEditor/writing-image-resize.test.ts src/components/WritingEditor/writing-image.test.ts`
Expected: PASS. The node view itself (drag handles, alignment buttons, the alt/title prompts) is not exercised by an automated test — happy-dom cannot lay it out or drag it — and is instead covered by the Manual verification section at the end of this plan.

- [ ] **Step 5: Commit**

```bash
git add packages/ui/src/components/WritingEditor/extensions.ts packages/ui/src/components/WritingEditor/writing-image-resize.ts packages/ui/src/components/WritingEditor/writing-image-resize.test.ts packages/ui/src/components/WritingEditor/WritingEditor.svelte
git commit -m "feat(writing): add the writingImage node view, resize handle and layout css"
```

---

### Task 6: Toolbar button and file picker

**Files:**
- Modify: `packages/ui/src/components/Button/ActionIcon.types.ts:84-85` (insert `'insert-image'` between `'import'` and `'italic'`)
- Modify: `packages/ui/src/components/Button/ActionIcon.svelte` (import `IconPhotoPlus`, map it)
- Modify: `packages/ui/src/components/WritingEditor/WritingEditor.types.ts` (add `oninsertimage` prop and `insertImage` label)
- Modify: `packages/ui/src/components/WritingEditor/WritingEditor.svelte:766-787` (insert-group tool), and export an `insertImage` function beside `insertCitation` (near line 309-318)
- Modify: `apps/desktop/src/lib/writing-editor-labels.ts`
- Modify: `apps/desktop/src/lib/i18n.ts` (`es` block near line 658, `en` block near line 1919)
- Modify: `apps/desktop/src/views/WritingView.svelte` (picker handler, `editorRef` type, template prop)
- Test: `packages/ui/src/components/WritingEditor/commands.test.ts` (extend), `apps/desktop/src/views/WritingView.test.ts` (extend, if present — otherwise a focused new test file `apps/desktop/src/lib/writing-image-picker.test.ts` for the handler logic in isolation)

**Interfaces:**
- Consumes: `insertWritingImage` command (Task 4), `importWritingImage` (Task 3), `imageSize` (Task 2).
- Produces: `WritingEditorProps.oninsertimage?: () => void`; exported `WritingEditor.svelte` function `insertImage(attrs: { src: string; alt?: string | null; title?: string | null; width?: number | null; height?: number | null; align?: 'left' | 'center' | 'right' }): boolean`.

`ACTION_ICON_NAMES` already has `'file-image'` mapped to `IconPhoto` (the generic asset thumbnail glyph, used for existing image assets). This is a different affordance — inserting a new image into the manuscript — so it takes a distinct name, `'insert-image'`, mapped to `IconPhotoPlus` (`@tabler/icons-svelte-runes/icons/photo-plus`, confirmed present in the installed package), the same "plus" convention `'folder-plus'`/`'file-plus'` already use for "create a new one of these."

- [ ] **Step 1: Write the failing tests**

Extend `commands.test.ts`:

```ts
describe('inserting an image from the toolbar', () => {
  it('is exposed as an editor command the app can call once it has an imported path', () => {
    const instance = mount()

    const ok = instance.chain().focus().insertWritingImage({ src: 'writing-images/abc.png' }).run()

    expect(ok).toBe(true)
    expect(typeNames(instance)).toContain('writingImage')
  })
})
```

New `apps/desktop/src/lib/writing-image-picker.test.ts` (isolates the picker's decision logic from the file-dialog/Tauri glue, which is exercised manually per the Manual verification section):

```ts
import { describe, expect, it } from 'vitest'
import { importWritingImage } from './writing-images'
import { imageSize } from './image-dimensions'

const PNG = new Uint8Array([
  0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52,
  0, 0, 0, 4, 0, 0, 0, 3, 8, 6, 0, 0, 0, 0, 0, 0, 0,
])

function io() {
  const written = new Map<string, Uint8Array>()
  return {
    written,
    dataDir: async () => 'C:/datos',
    join: async (...parts: string[]) => parts.join('/'),
    mkdir: async () => undefined,
    exists: async (path: string) => written.has(path),
    writeFile: async (path: string, bytes: Uint8Array) => {
      written.set(path, bytes)
    },
  }
}

/** What WritingView.svelte's picker handler does with the bytes it read,
 *  isolated from the Tauri dialog itself: import, then size, then the attrs
 *  it will pass to `insertImage`. */
async function attrsForPicked(bytes: Uint8Array) {
  const imported = await importWritingImage(bytes, io())
  if (!imported) return null
  const size = imageSize(bytes)
  return {
    src: imported.path,
    width: size?.width ?? null,
    height: size?.height ?? null,
    align: 'center' as const,
  }
}

describe('what the toolbar picker hands the editor', () => {
  it('builds insertable attrs from a supported file', async () => {
    const attrs = await attrsForPicked(PNG)

    expect(attrs?.src).toMatch(/^writing-images\/[0-9a-f]{64}\.png$/)
    expect(attrs?.width).toBe(4)
    expect(attrs?.height).toBe(3)
  })

  it('returns null for an unsupported file, so nothing is inserted', async () => {
    expect(await attrsForPicked(new Uint8Array([1, 2, 3]))).toBeNull()
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @entropia/ui test -- src/components/WritingEditor/commands.test.ts` and `pnpm --filter @entropia-pro/desktop test -- src/lib/writing-image-picker.test.ts`
Expected: the `@entropia/ui` test already passes as of Task 4 (kept here only as the toolbar-facing regression anchor); the new desktop file FAILs — `writing-image-picker.test.ts` imports real Task 2/3 modules, so it should already pass mechanically once those exist, so run it to confirm intent first with a deliberately wrong expectation, then correct it. (If it passes immediately because Tasks 2–3 already satisfy it, treat Step 2 as confirming the assertions are meaningful — flip one literal (`width: 4` → `width: 999`) first, watch it fail, then restore it.)

- [ ] **Step 3: Write the implementation**

`ActionIcon.types.ts`:

```ts
  'import',
  'insert-image',
  'italic',
```

`ActionIcon.svelte` — add the import near `IconPhoto` (existing `file-image` glyph):

```ts
  import IconPhoto from '@tabler/icons-svelte-runes/icons/photo'
  import IconPhotoPlus from '@tabler/icons-svelte-runes/icons/photo-plus'
```

and in the `ICONS` map, next to `import: IconFileDownload,`:

```ts
    import: IconFileDownload,
    'insert-image': IconPhotoPlus,
    italic: IconItalic,
```

`WritingEditor.types.ts` — add to `WritingEditorProps`:

```ts
  /** Opens the image picker. Without it, no insert-image tool appears in the
   *  toolbar — the same optional-capability shape as `ondictate`. */
  oninsertimage?: () => void
```

and to `WritingEditorLabels` / `DEFAULT_WRITING_EDITOR_LABELS`:

```ts
  insertImage: string
```
```ts
  insertImage: 'Insertar imagen',
```

`WritingEditor.svelte` — destructure `oninsertimage`, add the tool to the `insert` group (after `footnote`, mirroring the `ondictate` conditional-tool pattern):

```ts
        { id: 'footnote', label: labels.footnote, icon: 'footnote', run: insertFootnote },
        ...(oninsertimage
          ? [
              {
                id: 'insertImage',
                label: labels.insertImage,
                icon: 'insert-image' as const,
                run: oninsertimage,
              },
            ]
          : []),
```

and export the command, beside `insertCitation`:

```ts
  /**
   * Inserts a manuscript image at the caret. The app has already imported the
   * bytes (writing-images.ts) and read their size (image-dimensions.ts) by
   * the time this is called — this function only puts the node in the
   * document, exactly as insertCitation only puts the citation node in.
   */
  export function insertImage(attrs: {
    src: string
    alt?: string | null
    title?: string | null
    width?: number | null
    height?: number | null
    align?: 'left' | 'center' | 'right'
  }): boolean {
    if (!editor) return false
    return editor.chain().focus().insertWritingImage(attrs).run()
  }
```

`writing-editor-labels.ts` — add:

```ts
    insertImage: t('writing.toolbar.insertImage'),
```

`i18n.ts` — `es` block, beside `'writing.toolbar.footnote'`:

```ts
  'writing.toolbar.insertImage': 'Insertar imagen',
```

`en` block, beside its `'writing.toolbar.footnote'`:

```ts
  'writing.toolbar.insertImage': 'Insert image',
```

`WritingView.svelte` — imports:

```ts
  import { open } from '@tauri-apps/plugin-dialog'
  import { readFile } from '@tauri-apps/plugin-fs'
  import { importWritingImage } from '$lib/writing-images'
  import { imageSize } from '$lib/image-dimensions'
```

extend the `editorRef` type with `insertImage: (attrs: Record<string, unknown>) => boolean`, and add the handler:

```ts
  const WRITING_IMAGE_EXTENSIONS = ['png', 'jpg', 'jpeg', 'gif']

  /**
   * The toolbar's picker. Opens the dialog, reads the bytes, imports them
   * (writing-images.ts) and reads their size (image-dimensions.ts) — the same
   * two steps every entry path takes — then inserts through the same command
   * paste and drop use (Task 7).
   */
  async function insertWritingImageFromPicker() {
    const selected = await open({
      multiple: false,
      filters: [{ name: 'Imágenes', extensions: WRITING_IMAGE_EXTENSIONS }],
    })
    if (!selected || Array.isArray(selected)) return

    const bytes = await readFile(selected)
    const imported = await importWritingImage(bytes)
    if (!imported) {
      void appendLog('error', 'writing-image', `Formato de imagen no admitido: ${selected}`)
      return
    }

    const size = imageSize(bytes)
    editorRef?.insertImage({
      src: imported.path,
      alt: null,
      title: null,
      width: size?.width ?? null,
      height: size?.height ?? null,
      align: 'center',
    })
  }
```

and pass `oninsertimage={insertWritingImageFromPicker}` alongside the other `<WritingEditor>` props (both the `snapshot.content` and, since it needs no image support, not the `refusal` branch).

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @entropia/ui test -- src/components/WritingEditor/commands.test.ts` and `pnpm --filter @entropia-pro/desktop test -- src/lib/writing-image-picker.test.ts`
Expected: PASS.

- [ ] **Step 5: Run Svelte analysis**

```bash
npx @sveltejs/mcp svelte-autofixer packages/ui/src/components/WritingEditor/WritingEditor.svelte --svelte-version 5
npx @sveltejs/mcp svelte-autofixer apps/desktop/src/views/WritingView.svelte --svelte-version 5
```
Expected: no new issue caused by this change.

- [ ] **Step 6: Commit**

```bash
git add packages/ui/src/components/Button/ActionIcon.types.ts packages/ui/src/components/Button/ActionIcon.svelte packages/ui/src/components/WritingEditor/WritingEditor.types.ts packages/ui/src/components/WritingEditor/WritingEditor.svelte packages/ui/src/components/WritingEditor/commands.test.ts apps/desktop/src/lib/writing-editor-labels.ts apps/desktop/src/lib/i18n.ts apps/desktop/src/views/WritingView.svelte apps/desktop/src/lib/writing-image-picker.test.ts
git commit -m "feat(writing): add the insert-image toolbar button and file picker"
```

---

### Task 7: Paste and drag-and-drop

**Files:**
- Modify: `packages/ui/src/components/WritingEditor/extensions.ts` (add `addProseMirrorPlugins` to `WritingImage`)
- Modify: `apps/desktop/src/views/WritingView.svelte` (pass `importImage` prop)
- Test: `packages/ui/src/components/WritingEditor/writing-image-paste.test.ts`

**Interfaces:**
- Consumes: `WritingExtensionOptions.importImage` (threaded in Task 4).
- Produces: nothing new — the plugin only calls `insertWritingImage` internally.

- [ ] **Step 1: Write the failing tests**

```ts
import { Editor } from '@tiptap/core'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createWritingExtensions } from './extensions'
import { emptyDocument } from './document-contract'

let editor: Editor | undefined

afterEach(() => {
  editor?.destroy()
  editor = undefined
})

function mount(importImage: ((bytes: Uint8Array) => Promise<{ path: string; width: number; height: number } | null>) | undefined) {
  const element = document.createElement('div')
  document.body.appendChild(element)
  editor = new Editor({
    element,
    extensions: createWritingExtensions({ importImage }),
    content: emptyDocument().doc,
  })
  return editor
}

function pngFile(): File {
  const bytes = new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])
  return new File([bytes], 'pasted.png', { type: 'image/png' })
}

function pasteEvent(files: File[], html?: string): ClipboardEvent {
  const data = new DataTransfer()
  files.forEach((file) => data.items.add(file))
  if (html) data.setData('text/html', html)
  const event = new ClipboardEvent('paste', { clipboardData: data, cancelable: true })
  return event
}

describe('pasting an image into the manuscript', () => {
  it('imports and inserts a pasted image file', async () => {
    const importImage = vi.fn(async () => ({ path: 'writing-images/abc.png', width: 10, height: 10 }))
    const instance = mount(importImage)
    instance.commands.focus()

    const handled = instance.view.someProp('handlePaste', (fn) => fn(instance.view, pasteEvent([pngFile()])))
    await vi.waitFor(() => expect(importImage).toHaveBeenCalled())

    expect(handled).toBe(true)
  })

  it('does not intercept a plain text paste', () => {
    const importImage = vi.fn()
    const instance = mount(importImage)
    instance.commands.focus()

    const handled = instance.view.someProp('handlePaste', (fn) => fn(instance.view, pasteEvent([])))

    expect(handled).toBeFalsy()
    expect(importImage).not.toHaveBeenCalled()
  })

  it('never produces a node from a remote or foreign-filesystem img with no accompanying bytes', () => {
    const importImage = vi.fn()
    const instance = mount(importImage)
    instance.commands.focus()

    instance.view.someProp('handlePaste', (fn) =>
      fn(instance.view, pasteEvent([], '<img src="https://example.org/remote.png">'))
    )

    const names: string[] = []
    instance.state.doc.forEach((node) => names.push(node.type.name))
    expect(names).not.toContain('writingImage')
    expect(importImage).not.toHaveBeenCalled()
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @entropia/ui test -- src/components/WritingEditor/writing-image-paste.test.ts`
Expected: FAIL — no `handlePaste` is registered yet, so `someProp` finds nothing to call and `handled` is `undefined`, not `true`.

- [ ] **Step 3: Write the implementation**

Add to the `WritingImage` node definition in `extensions.ts` (after `addKeyboardShortcuts`):

```ts
  addProseMirrorPlugins() {
    const importImage = this.options.importImage
    const editorRef = this.editor
    const ACCEPTED = new Set(['image/png', 'image/jpeg', 'image/gif'])

    async function importAndInsert(file: File, pos: number | null) {
      if (!importImage) return
      const bytes = new Uint8Array(await file.arrayBuffer())
      const imported = await importImage(bytes)
      if (!imported) return
      const node = editorRef.schema.nodes.writingImage?.create({
        src: imported.path,
        alt: null,
        title: null,
        width: imported.width || null,
        height: imported.height || null,
        align: 'center',
      })
      if (!node) return
      const insertPos = pos ?? editorRef.state.selection.from
      editorRef.view.dispatch(editorRef.state.tr.insert(insertPos, node))
    }

    return [
      new Plugin({
        key: new PluginKey('writingImagePasteDrop'),
        props: {
          // Claims the event only when it actually carries an accepted image
          // file. Every other paste — plain text, or HTML with no
          // accompanying bytes such as a remote <img> — falls through
          // untouched, and the schema's own lack of an img[src] parse rule is
          // what keeps that fallthrough from creating a broken reference.
          handlePaste(_view, event) {
            const files = Array.from(event.clipboardData?.files ?? [])
            const image = files.find((file) => ACCEPTED.has(file.type))
            if (!image) return false
            event.preventDefault()
            void importAndInsert(image, null)
            return true
          },
          handleDrop(view, event) {
            const files = Array.from(event.dataTransfer?.files ?? [])
            const image = files.find((file) => ACCEPTED.has(file.type))
            if (!image) return false
            event.preventDefault()
            const coords = view.posAtCoords({ left: event.clientX, top: event.clientY })
            void importAndInsert(image, coords?.pos ?? null)
            return true
          },
        },
      }),
    ]
  },
```

Import `Plugin, PluginKey` from `@tiptap/pm/state` at the top of `extensions.ts` (add to the existing `Node, mergeAttributes` import line's neighbourhood — a new `import { Plugin, PluginKey } from '@tiptap/pm/state'`).

`WritingView.svelte` — add the bytes-to-attrs adapter and pass it as a prop:

```ts
  async function importWritingImageBytes(
    bytes: Uint8Array
  ): Promise<{ path: string; width: number; height: number } | null> {
    const imported = await importWritingImage(bytes)
    if (!imported) return null
    const size = imageSize(bytes)
    return { path: imported.path, width: size?.width ?? 0, height: size?.height ?? 0 }
  }
```

and `importImage={importWritingImageBytes}` on `<WritingEditor>`.

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @entropia/ui test -- src/components/WritingEditor/writing-image-paste.test.ts`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add packages/ui/src/components/WritingEditor/extensions.ts apps/desktop/src/views/WritingView.svelte packages/ui/src/components/WritingEditor/writing-image-paste.test.ts
git commit -m "feat(writing): handle image paste and drop through the same import path"
```

---

### Task 8: Exports

**Files:**
- Modify: `apps/desktop/src/lib/export-images.ts:38-58` (collect `writingImage.attrs.src` too)
- Modify: `apps/desktop/src/lib/export-html.ts` (new `case 'writingImage'` in `block()`)
- Modify: `apps/desktop/src/lib/export-markdown.ts` (new `case 'writingImage'` in `block()`)
- Modify: `apps/desktop/src/lib/export-docx.ts` (new `case 'writingImage'` in `block()`)
- Modify: `apps/desktop/src/lib/export-fidelity.ts:81-82` (`writingImage` row in `NODE_FIDELITY`)
- Modify: `apps/desktop/src/lib/export-pattern.test.ts` (add an image to `PATTERN`, mock its bytes, re-verify the DOCX-warnings assertion at lines 276-280)
- Test: `apps/desktop/src/lib/export-html.test.ts`, `apps/desktop/src/lib/export-markdown.test.ts`, `apps/desktop/src/lib/export-docx.test.ts` (one case each, per-construct — the pattern document is the integration proof, these are what name what broke)

**Interfaces:**
- Consumes: `ExportContext.images` (unchanged shape), `drawnImage`/`quotedImageSize` already exported from `export-docx.ts:405,420`.
- Produces: nothing new — no exported symbols change shape.

Per the spec, no i18n label is added: `writingImage` is `native` in `NODE_FIDELITY` for all three formats, so `export-vocabulary.test.ts`'s `nameable()` (which only collects non-`native` rows plus `REQUIRED_BY_SPEC`) never sees it. `REQUIRED_BY_SPEC` is left untouched.

- [ ] **Step 1: Write the failing tests**

`export-html.test.ts` — add near the other `describe` blocks:

```ts
import type { ExportImage } from './export-document'

const sampleImage: ExportImage = {
  bytes: new Uint8Array([1, 2, 3]),
  mediaType: 'image/png',
  dataUrl: 'data:image/png;base64,AQID',
  width: 300,
  height: 150,
}

describe('a manuscript image', () => {
  it('emits a figure with the embedded image and its caption', () => {
    const out = html(
      doc({
        type: 'writingImage',
        attrs: {
          src: 'writing-images/abc.png',
          alt: 'Vista',
          title: null,
          width: 300,
          height: 150,
          align: 'center',
        },
        content: [text('Vista del taller.')],
      }),
      { images: { 'writing-images/abc.png': sampleImage } }
    )

    expect(out).toContain('<figure')
    expect(out).toContain(`src="${sampleImage.dataUrl}"`)
    expect(out).toContain('Vista del taller.')
  })
})
```

`export-markdown.test.ts`:

```ts
import type { ExportImage } from './export-document'

const sampleImage: ExportImage = {
  bytes: new Uint8Array([1, 2, 3]),
  mediaType: 'image/png',
  dataUrl: 'data:image/png;base64,AQID',
  width: 300,
  height: 150,
}

describe('a manuscript image', () => {
  it('emits an embedded image and its caption, with no filesystem path', () => {
    const out = md(
      doc({
        type: 'writingImage',
        attrs: {
          src: 'writing-images/abc.png',
          alt: 'Vista',
          title: null,
          width: 300,
          height: 150,
          align: 'center',
        },
        content: [text('Vista del taller.')],
      }),
      { images: { 'writing-images/abc.png': sampleImage } }
    )

    expect(out).toContain(`![Vista](${sampleImage.dataUrl})`)
    expect(out).toContain('Vista del taller.')
    expect(out).not.toContain('writing-images/abc.png')
  })
})
```

`export-docx.test.ts`:

```ts
describe('a manuscript image', () => {
  it('embeds the image bytes and emits the caption', async () => {
    const { names, read } = await parts(
      doc({
        type: 'writingImage',
        attrs: {
          src: 'writing-images/abc.png',
          alt: 'Vista',
          title: null,
          width: 300,
          height: 150,
          align: 'center',
        },
        content: [text('Vista del taller.')],
      }),
      {
        images: {
          'writing-images/abc.png': {
            bytes: new Uint8Array([1, 2, 3]),
            mediaType: 'image/png',
            dataUrl: 'data:image/png;base64,AQID',
            width: 300,
            height: 150,
          },
        },
      }
    )

    expect(names.some((name) => name.startsWith('word/media/'))).toBe(true)
    expect(read('word/document.xml')).toContain('Vista del taller.')
  })
})
```

- [ ] **Step 2: Run test to verify it fails**

Run: `pnpm --filter @entropia-pro/desktop test -- src/lib/export-html.test.ts src/lib/export-markdown.test.ts src/lib/export-docx.test.ts`
Expected: FAIL — each falls into the exporter's `default` branch, which for a node with no `text` case and content living in `content`, not `.text`, produces nothing (spec, Exports): no `<figure>`, no `![`, no image part.

- [ ] **Step 3: Write the implementation**

`export-images.ts:38-58` — extend the walk:

```ts
export function quotedImagePaths(doc: Node): string[] {
  const paths: string[] = []
  const seen = new Set<string>()

  const walk = (node: Node) => {
    const parts = node.attrs?.quotedParts
    if (Array.isArray(parts)) {
      for (const part of parts) {
        if (!part || typeof part !== 'object') continue
        const { kind, source } = part as { kind?: unknown; source?: unknown }
        if (kind !== 'image' || typeof source !== 'string' || seen.has(source)) continue
        seen.add(source)
        paths.push(source)
      }
    }
    if (node.type === 'writingImage' && typeof node.attrs?.src === 'string' && !seen.has(node.attrs.src)) {
      seen.add(node.attrs.src)
      paths.push(node.attrs.src)
    }
    childrenOf(node).forEach(walk)
  }
  walk(doc)

  return paths
}
```

`export-html.ts` — add before `default:` in `block()`:

```ts
    case 'writingImage': {
      const src = typeof node.attrs?.src === 'string' ? node.attrs.src : ''
      const image = context.images?.[src]
      const align = typeof node.attrs?.align === 'string' ? node.attrs.align : 'center'
      const alt = typeof node.attrs?.alt === 'string' ? node.attrs.alt : ''
      const caption = inline(kids, context, notes)
      const img = image ? `<img src="${image.dataUrl}" alt="${escape(alt)}" />` : ''
      return `<figure class="writing-image" data-align="${align}">${img}<figcaption>${caption}</figcaption></figure>`
    }
```

`export-markdown.ts` — add before `default:` in `block()`:

```ts
    case 'writingImage': {
      const src = typeof node.attrs?.src === 'string' ? node.attrs.src : ''
      const image = context.images?.[src]
      const alt = typeof node.attrs?.alt === 'string' ? node.attrs.alt : ''
      const caption = inline(kids, context, notes)
      const img = image ? `![${escape(alt)}](${image.dataUrl})` : ''
      return caption ? `${img}\n\n${caption}` : img
    }
```

`export-docx.ts` — add before `default:` in `block()`:

```ts
    case 'writingImage': {
      const src = typeof node.attrs?.src === 'string' ? node.attrs.src : ''
      const drawn = drawnImage(build.context.images?.[src])
      const captionRuns = inline(kids, build)
      const paragraphs: Paragraph[] = []
      if (drawn) paragraphs.push(new Paragraph({ children: [drawn], alignment: AlignmentType.CENTER }))
      if (captionRuns.length > 0) {
        paragraphs.push(new Paragraph({ children: captionRuns, alignment: AlignmentType.CENTER }))
      }
      return paragraphs
    }
```

`export-fidelity.ts` — add the row, right after `noteLink`:

```ts
  noteLink: { markdown: 'fallback', html: 'fallback', docx: 'fallback' },
  // Native everywhere: DOCX embeds the bytes, HTML and Markdown both carry a
  // data: URI. Left out of REQUIRED_BY_SPEC — an image is not an element
  // whose absence should void the whole export.
  writingImage: { markdown: 'native', html: 'native', docx: 'native' },
```

`export-pattern.test.ts` — add a `readFile` mock and a captioned, resized image to `PATTERN`:

```ts
import { readFile } from '@tauri-apps/plugin-fs'
const mockReadFile = vi.mocked(readFile)

function pngBytes(width: number, height: number): Uint8Array {
  const bytes = new Uint8Array(33)
  bytes.set([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a], 0)
  const view = new DataView(bytes.buffer)
  view.setUint32(8, 13)
  bytes.set([0x49, 0x48, 0x44, 0x52], 12)
  view.setUint32(16, width)
  view.setUint32(20, height)
  return bytes
}
const PATTERN_IMAGE_PATH = 'writing-images/pattern.png'
```

Insert into `PATTERN.content`, after the `blockquote` entry:

```ts
    {
      type: 'writingImage',
      attrs: {
        src: PATTERN_IMAGE_PATH,
        alt: 'Vista del taller',
        title: null,
        width: 300,
        height: 150,
        align: 'center',
      },
      content: [text('Vista del taller metalúrgico, 1919.')],
    },
```

Extend the existing `beforeEach`'s `mockInvoke` block with a sibling mock (readFile is a separate Tauri plugin, mocked independently):

```ts
beforeEach(() => {
  mockInvoke.mockReset()
  mockInvoke.mockImplementation(async (command: string) => {
    if (command === 'writing_csl_render_document') {
      return [{ text: '(Acha, 2015)', author_suppressed: false }] as never
    }
    if (command === 'writing_csl_bibliography') {
      return ['Acha, O. (2015). Un libro. Editorial.'] as never
    }
    throw new Error(`unexpected command: ${command}`)
  })
  mockReadFile.mockReset()
  mockReadFile.mockImplementation(async (path: string) => {
    if (path === PATTERN_IMAGE_PATH) return pngBytes(300, 150)
    throw new Error(`unexpected read: ${path}`)
  })
})
```

The DOCX-warnings assertion at (what were) lines 276-280 needs no textual change — `writingImage` is `native` in all three formats, so it introduces no new warning and `out.warnings.map((warning) => warning.element)` still equals exactly `['noteLink']`. Re-run it to confirm rather than editing it.

- [ ] **Step 4: Run test to verify it passes**

Run: `pnpm --filter @entropia-pro/desktop test -- src/lib/export-html.test.ts src/lib/export-markdown.test.ts src/lib/export-docx.test.ts src/lib/export-pattern.test.ts src/lib/export-fidelity.test.ts src/lib/export-vocabulary.test.ts src/lib/export-images.test.ts`
Expected: all PASS, including `export-fidelity.test.ts`'s schema-completeness guard, which closes the transient RED opened in Task 4.

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src/lib/export-images.ts apps/desktop/src/lib/export-html.ts apps/desktop/src/lib/export-markdown.ts apps/desktop/src/lib/export-docx.ts apps/desktop/src/lib/export-fidelity.ts apps/desktop/src/lib/export-pattern.test.ts apps/desktop/src/lib/export-html.test.ts apps/desktop/src/lib/export-markdown.test.ts apps/desktop/src/lib/export-docx.test.ts
git commit -m "feat(writing): export manuscript images to markdown, html and docx"
```

---

## Manual verification

The editor is a native Tauri window; the agent cannot see it. Every step below is confirmed by the user, not asserted by this plan.

1. Open a manuscript, place the cursor, click the insert-image toolbar button, pick a PNG or JPEG or GIF file — confirm it appears inline, laid out inside the margins.
2. Select it, drag a resize handle — confirm the width changes, the aspect ratio holds, and it never grows past the column or shrinks past a usable minimum.
3. Click into the caption, type a caption, press Enter — confirm it exits to a new paragraph rather than splitting the figure.
4. Use the alignment buttons (left/center/right) and the alt/title prompt — confirm they take effect.
5. Save, leave the document, reopen it — confirm the image, its size, alignment, and caption are all exactly as left.
6. Close EntropIA entirely, start it again, reopen the same document — confirm the same, across a real restart.
7. Copy an image (a screenshot, or from a file manager) and paste it into the manuscript — confirm it is imported and inserted the same way.
8. Drag an image file from the OS file explorer onto the editor — confirm the same.
9. Paste a block of HTML copied from a web page containing an `<img>` with no accompanying image data — confirm nothing broken is inserted.
10. Try dropping or pasting a WebP or SVG file — confirm it is refused with a visible report and the document is unchanged.
11. Delete the selected image, undo, redo — confirm undo restores width, alignment, alt text and caption together, and redo removes it again.
12. Export to Markdown, HTML, and DOCX — open each result and confirm the image, its caption, and (for DOCX) that Word/LibreOffice render it without a broken reference.
13. Move or rename the original source file the image came from (if it still exists on disk) — confirm the manuscript is unaffected, since the stored copy is independent of it.

---

## Self-review

**Spec coverage.** Walking `writing-image-node-design.md` section by section:

- Goal / Scope — Tasks 1-8 cover every file the spec's Scope section names.
- Node Shape — Task 4 (attrs, `renderHTML`, `parseHTML`).
- Managed Image Storage — Task 1 (hash), Task 3 (storage), verified against the spec's explicit `crypto.subtle`-first ordering.
- Entry Paths — Task 6 (toolbar/picker), Task 7 (paste/drop); accepted-formats refusal covered by `detectWritingImageType`/`ACCEPTED` in both.
- Node View and Resizing — Task 5 (node view, clamp function, alt/title/align controls as plain DOM chrome — the spec's "selection bubble" wording is realized as always-visible figure chrome rather than a floating positioned bubble, since a floating bubble would need either a rendering dependency the spec explicitly rules out, or hand-rolled position tracking outside this plan's proportional scope; noted here rather than silently substituted).
- Rendering and Layout — Task 5 (stylesheet `max-width`/`height:auto`), Task 2 (header-byte dimensions).
- Serialization and Compatibility — no schema version bump anywhere in this plan; confirmed by Task 4's round-trip test.
- Exports / Export Guards — Task 8, including the explicit no-new-i18n-label constraint and leaving `REQUIRED_BY_SPEC` alone.
- Failure Handling — import failure inserts nothing (Task 3 returns `null`, Task 6/7 check it); unsupported file reported (Task 6's `appendLog`, Task 7's silent fallthrough); missing file at render time — not separately implemented, since the `<img>` tag simply fails to load in the webview and the node stays in the document by construction (the attrs are untouched); missing file at export time — `loadExportImages` already leaves a missing path out of `context.images` (`export-images.ts:71-77`, unmodified), and Task 8's `case 'writingImage'` already handles `image` being `undefined` by emitting no `<img>`/`![]` — this alone does not yet emit a *warning* for a missing file the way the spec's Verification item 17 asks. **Gap found during self-review**: `fidelityWarnings` only inspects `NODE_FIDELITY`/`MARK_FIDELITY`/`ATTRIBUTE_FIDELITY` support tables, not per-instance file availability, so a missing stored file currently exports silently rather than warning. This is a real gap against Verification item 17 and is not closed by any task above — flagged here rather than silently dropped; closing it needs a new warning kind threaded from `loadExportImages` through `ExportContext` into `fidelityWarnings`, which is a small but distinct piece of work the task spine given for this plan did not allocate a task to. Recommend a follow-up task (`Task 9: warn on a missing stored image at export time`) before this feature is considered fully done against its own Verification list.
- Non-Goals — respected: no GC, no cropping/rotation, no remote URLs, no per-document image table, `assets`/OCR subsystem untouched.

**Placeholder scan.** No `TBD`/`TODO`/"add error handling"/"similar to Task N" anywhere above; every code step carries complete code. The one deliberately open item is the export-time missing-file warning named directly above, reported as a gap rather than hidden behind a placeholder.

**Type consistency.** `WritingImageAttrs` (Task 4) — `{ src, alt, title, width, height, align }` — is the shape every later task's attrs literals match (Task 6's `insertImage`, Task 7's `importAndInsert`, Task 8's test fixtures). `ImportedWritingImage` (Task 3) — `{ path, mediaType }` — is what Task 6's picker destructures (`imported.path`); Task 7's `importImage` option instead returns `{ path, width, height }`, a deliberately different shape built by `WritingView.svelte`'s adapter (`importWritingImageBytes`) on top of `ImportedWritingImage` plus `imageSize`, not `ImportedWritingImage` itself reused — verified these are two distinct, correctly-named signatures rather than a drift. `imageSize` (Task 2) keeps the exact signature `export-images.ts` already had, confirmed by the unmodified `export-images.test.ts` staying green. `clampWritingImageWidth` (Task 5) signature matches its one call site inside the node view's drag handlers.

## Report

- **Task count:** 8, plus one self-review-identified follow-up (export-time missing-image warning) not included in the numbered tasks, per the task spine given.
- **Files created:** `writing-image-hash.ts` (+test), `image-dimensions.ts` (+test), `writing-images.ts` (+test), `writing-image-resize.ts` (+test), `writing-image.test.ts`, `writing-image-paste.test.ts`, `writing-image-picker.test.ts` — 7 new source/test pairs (some tasks share a test file with their source, some don't; 11 new files total counting tests separately).
- **Files modified:** `export-images.ts`, `extensions.ts`, `trailing-paragraph.ts`, `WritingEditor.svelte`, `WritingEditor.types.ts`, `ActionIcon.types.ts`, `ActionIcon.svelte`, `writing-editor-labels.ts`, `i18n.ts`, `WritingView.svelte`, `export-html.ts`, `export-markdown.ts`, `export-docx.ts`, `export-fidelity.ts`, `export-pattern.test.ts`, `export-html.test.ts`, `export-markdown.test.ts`, `export-docx.test.ts`, `commands.test.ts` — 19 files.
- **Spec items not mapped to a task:** the export-time missing-stored-image warning (spec Verification item 17's second half — the node staying in the document at *render* time is covered, the export-time warning is not). Flagged above as a follow-up task, not silently dropped.
