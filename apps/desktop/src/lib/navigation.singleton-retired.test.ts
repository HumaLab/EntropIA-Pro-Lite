import { describe, it, expect } from 'vitest'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, sep } from 'node:path'

const SRC_ROOT = join(__dirname, '..')
const SINGLETON_IMPORT =
  /import\s*\{[^}]*\bnavigation\b[^}]*\}\s*from\s*['"](\$lib\/navigation|\.\.?\/(?:lib\/)?navigation)['"]/

function collectSourceFiles(dir: string, out: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry)
    const stats = statSync(full)
    if (stats.isDirectory()) {
      collectSourceFiles(full, out)
    } else if (/\.(ts|svelte)$/.test(entry)) {
      out.push(full)
    }
  }
  return out
}

describe('the retired navigation singleton', () => {
  it('is exported by nothing (navigation.ts no longer declares it)', () => {
    const source = readFileSync(join(SRC_ROOT, 'lib', 'navigation.ts'), 'utf8')
    expect(source).not.toMatch(/export const navigation = new NavigationStore\(\)/)
  })

  it('is imported by no file except navigation.ts itself and its own test', () => {
    const offenders = collectSourceFiles(SRC_ROOT).filter((file) => {
      if (file.endsWith(`${sep}lib${sep}navigation.ts`)) return false
      if (file.endsWith(`${sep}lib${sep}navigation.test.ts`)) return false
      const content = readFileSync(file, 'utf8')
      return SINGLETON_IMPORT.test(content)
    })
    expect(offenders).toEqual([])
  })
})
