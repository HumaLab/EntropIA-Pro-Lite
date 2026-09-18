import { readdirSync, readFileSync, statSync } from 'node:fs'
import { join, relative, resolve } from 'node:path'
import { describe, expect, it } from 'vitest'
import { FONT_PRESET_DEFAULT, FONT_PRESETS } from './lib/typography'

/**
 * The typography presets are stated in three files, in two languages:
 * `lib/typography.ts` names them, `tokens.css` maps each to its families, and
 * `fonts.css` ships the faces. Nothing but this test notices when one of them
 * moves without the others — a family named and never shipped falls back to
 * the system font, silently, on only some machines.
 */

const UI_SRC = resolve(import.meta.dirname, '../../../packages/ui/src')
const TOKENS = readFileSync(join(UI_SRC, 'tokens/tokens.css'), 'utf-8')
const FONTS = readFileSync(resolve(import.meta.dirname, 'fonts.css'), 'utf-8')

const SEMANTIC_TOKENS = ['--font-ui', '--font-reading', '--font-mono'] as const

/** The declarations inside the first rule whose selector matches. */
function ruleBody(selector: RegExp): string | null {
  const match = new RegExp(`${selector.source}\\s*\\{([^}]*)\\}`).exec(TOKENS)
  return match?.[1] ?? null
}

/** The first family of a token's stack, unquoted. */
function firstFamily(body: string, token: string): string | null {
  const match = new RegExp(`${token}:\\s*'([^']+)'`).exec(body)
  return match?.[1] ?? null
}

function declaredFaces(): { family: string; style: string }[] {
  return Array.from(FONTS.matchAll(/@font-face\s*\{([^}]*)\}/g), ([, body = '']) => ({
    family: /font-family:\s*'([^']+)'/.exec(body)?.[1] ?? '',
    style: /font-style:\s*(\w+)/.exec(body)?.[1] ?? '',
  }))
}

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name)
    if (statSync(path).isDirectory()) return name === 'node_modules' ? [] : sourceFiles(path)
    return /\.(svelte|css)$/.test(name) ? [path] : []
  })
}

describe('each preset in tokens.css', () => {
  for (const preset of FONT_PRESETS) {
    it(`maps ${preset.id} to the families the registry names`, () => {
      const body = ruleBody(new RegExp(`\\[data-font='${preset.id}'\\]`))
      expect(body, `no [data-font='${preset.id}'] block`).not.toBeNull()

      expect(firstFamily(body!, '--font-ui')).toBe(preset.families.ui)
      expect(firstFamily(body!, '--font-reading')).toBe(preset.families.reading)
      expect(firstFamily(body!, '--font-mono')).toBe(preset.families.mono)
    })
  }

  /** Before anything sets `data-font`, the page must already read as the default. */
  it('declares the default on :root as well', () => {
    expect(TOKENS).toMatch(new RegExp(`:root,\\s*\\[data-font='${FONT_PRESET_DEFAULT}'\\]`))
  })

  /**
   * Unscoped by `:root` on purpose: a preview card sets `data-font` on itself
   * and inherits that preset's tokens, whatever the application is using.
   */
  it('scopes presets to any element, not only the root', () => {
    expect(TOKENS).not.toMatch(/:root\[data-font=/)
  })
})

describe('fonts.css', () => {
  const faces = declaredFaces()

  it('ships every family a preset names', () => {
    const shipped = new Set(faces.map((face) => face.family))
    for (const preset of FONT_PRESETS) {
      for (const family of Object.values(preset.families)) {
        expect(shipped, `${preset.id}: ${family}`).toContain(family)
      }
    }
  })

  /** Reading text carries emphasis and titles; a synthesised slant is not italic. */
  it('ships a real italic for every reading family', () => {
    for (const preset of FONT_PRESETS) {
      const italic = faces.some(
        (face) => face.family === preset.families.reading && face.style === 'italic'
      )
      expect(italic, preset.families.reading).toBe(true)
    }
  })

  /**
   * Every interface family puts its lowercase higher in the line box than
   * Segoe UI did, and the controls were sized against Segoe: text in badges and
   * buttons read as top-aligned. Each UI face overrides its ascent and descent
   * to sit where Segoe sat, without changing their sum — so no box changes
   * height, only where the glyphs land in it.
   */
  it('recentres every interface face on the line box', () => {
    const uiFamilies = new Set(FONT_PRESETS.map((preset) => preset.families.ui))
    const bodies = Array.from(FONTS.matchAll(/@font-face\s*\{([^}]*)\}/g), ([, body = '']) => body)

    for (const body of bodies) {
      const family = /font-family:\s*'([^']+)'/.exec(body)?.[1] ?? ''
      if (!uiFamilies.has(family)) continue
      expect(body, family).toMatch(/ascent-override:\s*[\d.]+%/)
      expect(body, family).toMatch(/descent-override:\s*[\d.]+%/)
    }
  })

  it('declares only the latin subsets', () => {
    const files = FONTS.match(/url\([^)]+\)/g) ?? []
    expect(files.length).toBeGreaterThan(0)
    for (const file of files) expect(file).toMatch(/-latin(-ext)?-/)
  })

  it('never reaches the network', () => {
    for (const file of FONTS.match(/url\([^)]+\)/g) ?? []) expect(file).not.toMatch(/https?:/)
  })
})

describe('components', () => {
  /**
   * A component names what text is for, never which font it is in. Anything
   * else survives a preset change looking like the old one.
   */
  it('name a semantic token or inherit, never a family', () => {
    const allowed = /^(var\(--font-(ui|reading|mono)\)|inherit)$/
    const offenders: string[] = []

    for (const file of [...sourceFiles(resolve(import.meta.dirname)), ...sourceFiles(UI_SRC)]) {
      if (file.endsWith('tokens.css') || file.endsWith('fonts.css')) continue
      const text = readFileSync(file, 'utf-8')
      for (const [, value = ''] of text.matchAll(/font-family:\s*([^;}\n]+)/g)) {
        if (!allowed.test(value.trim())) offenders.push(`${relative(UI_SRC, file)}: ${value}`)
      }
    }

    expect(offenders).toEqual([])
  })

  it('no longer name the retired tokens', () => {
    const offenders: string[] = []
    for (const file of [...sourceFiles(resolve(import.meta.dirname)), ...sourceFiles(UI_SRC)]) {
      if (/--font-(sans|display)\b/.test(readFileSync(file, 'utf-8'))) offenders.push(file)
    }
    expect(offenders).toEqual([])
  })

  it('have all three semantic tokens to name', () => {
    const body = ruleBody(/:root,\s*\[data-font='academic'\]/)
    for (const token of SEMANTIC_TOKENS) expect(body).toContain(`${token}:`)
  })
})
