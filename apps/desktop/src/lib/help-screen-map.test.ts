import { describe, expect, it } from 'vitest'
import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const desktopRoot = resolve(here, '../..')
const script = resolve(desktopRoot, 'scripts/build-help-screen-map.mjs')
const mapPath = resolve(desktopRoot, '../../manual/manual-usuario/mapa-de-pantallas.md')

const readMap = (): string => readFileSync(mapPath, 'utf-8')

describe('mapa de pantallas (S11, D8)', () => {
  it('el mapa comprometido está al día con el generador (--check)', () => {
    // D8: como version:check, el mapa se genera desde el código y el test
    // falla cuando está desactualizado. El script sale 1 si difiere.
    execFileSync(process.execPath, [script, '--check'], { stdio: 'pipe' })
  })

  it('contiene etiquetas conocidas por su nombre real en español', () => {
    const map = readMap()
    // nav.help
    expect(map).toContain('«Ayuda»')
    // settings.save + item.*SaveTitle
    expect(map).toContain('«Guardar cambios»')
    // nav.settings / settings.title
    expect(map).toContain('«Configuración»')
    // Nombres de las pestañas de Configuración (SettingsView.svelte).
    for (const tab of ['APIs remotas', 'Sincronización', 'Asistentes', 'Lotes', 'Copias']) {
      expect(map).toContain(`«${tab}»`)
    }
  })

  it('no deja escapar etiquetas solo-inglesas', () => {
    const map = readMap()
    // help.openSettings y help.placeholder en inglés: el mapa solo lee el
    // bloque español de i18n.ts, así que estas cadenas no pueden aparecer.
    expect(map).not.toContain('Open Settings')
    expect(map).not.toContain('Type your question…')
  })

  it('avisa que es generado y cómo regenerarlo', () => {
    const map = readMap()
    expect(map).toContain('build-help-screen-map.mjs')
    expect(map).toContain('pnpm help:screen-map')
  })

  it('cada sección ### es lo bastante chica para indexarla (< 3 KB)', () => {
    const map = readMap()
    const sections = map.split(/^### /m).slice(1)
    expect(sections.length).toBeGreaterThan(10)
    for (const section of sections) {
      const title = section.split('\n', 1)[0]
      expect(section.length, `### ${title}`).toBeLessThan(3072)
    }
  })
})
