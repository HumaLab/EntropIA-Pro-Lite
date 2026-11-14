#!/usr/bin/env node
// Mapa de pantallas de la ayuda en la app (S11, D8): genera
// `manual/manual-usuario/mapa-de-pantallas.md` desde las cadenas españolas de
// la interfaz, para que el asistente nombre pantallas, pestañas, botones y
// errores por su nombre real. Sin dependencias: solo Node.
//
// Fuentes:
//   - `apps/desktop/src/lib/i18n.ts`: solo el bloque español (`const es = {`
//     hasta `} as const`); el bloque inglés (`const en: ... = {`) no se lee.
//   - `apps/desktop/src/lib/navigation.ts`: los nombres de `View`.
//   - `apps/desktop/src/views/SettingsView.svelte`: las pestañas en el orden
//     en que aparecen (`t('settings.*Tab')`).
//
// Uso:
//   node apps/desktop/scripts/build-help-screen-map.mjs [--root <dir>]
//   node apps/desktop/scripts/build-help-screen-map.mjs --check [--root <dir>]
//
// `--check` sale 1 cuando el archivo comprometido difiere (como
// `version:check`): el test `src/lib/help-screen-map.test.ts` lo ejecuta.

import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))

const I18N_PATH = 'apps/desktop/src/lib/i18n.ts'
const NAVIGATION_PATH = 'apps/desktop/src/lib/navigation.ts'
const SETTINGS_VIEW_PATH = 'apps/desktop/src/views/SettingsView.svelte'
const MAP_PATH = 'manual/manual-usuario/mapa-de-pantallas.md'

/** Cada sección `###` se parte para no pasar este tamaño (la indexa FTS5). */
const SECTION_MAX_BYTES = 2800
/** Un grupo con menos líneas que esto se junta en «Varios» de su área. */
const MIN_GROUP_LINES = 5

/** Título español del área cuando es obvio; si no, el prefijo tal cual. */
const AREA_TITLES = {
  appshell: 'Ventana',
  app: 'Aplicación',
  appearance: 'Apariencia',
  assistants: 'Asistentes',
  batch: 'Lotes',
  biblioteca: 'Biblioteca',
  bibliography: 'Bibliografía',
  bibliographyAnnotations: 'Anotaciones',
  bibliographyWork: 'Obra',
  collection: 'Colección',
  collectionAnalysis: 'Análisis',
  collections: 'Colecciones',
  dbBrowser: 'Base de datos',
  entityAliases: 'Alias',
  errors: 'Errores',
  explorer: 'Explorador',
  graphSettings: 'Grafo',
  help: 'Ayuda',
  home: 'Inicio',
  internetArchive: 'Internet Archive',
  investigation: 'Informes de investigación',
  item: 'Documento',
  navegador: 'Navegador',
  nav: 'Navegación',
  numberField: 'Campos numéricos',
  occurrence: 'Ocurrencias',
  pantallas: 'Pantallas',
  phraseAnalysis: 'Frases',
  ragChat: 'Chat',
  research: 'Investigación',
  settings: 'Configuración',
  storeUpdate: 'Actualizaciones',
  sync: 'Sincronización',
  tabs: 'Pestañas',
  topbar: 'Barra superior',
  typography: 'Tipografía',
  workpane: 'Paneles',
  writing: 'Escritura',
  zotero: 'Zotero',
}

/** Claves de error van al área «Errores», con su prefijo como sub-área. */
const ERROR_KEY = /(^|\.)error(\.|$)|error$|failed$|(^|\.)fail(\.|$)/i

const areaTitle = (prefix) => AREA_TITLES[prefix] ?? prefix

/** El subtítulo es el nombre de la pestaña cuando el grupo es una pestaña
 *  (`settings.backup` → «Copias» por `settings.backupTab`); si no, el
 *  segmento tal cual. */
const subTitle = (es, area, sub) => {
  const tabKey = `${area}.${sub}Tab`
  if (Object.hasOwn(es, tabKey)) return es[tabKey]
  return sub
}

const unescape = (value) => value.replace(/\\'/g, "'").replace(/\\\\/g, '\\')

/** Pares `'clave': 'valor'` del bloque dado (valores multilínea y con
 *  escapes simples; los `{placeholders}` se conservan: se leen bien). */
function extractPairs(block) {
  const pairs = []
  const re = /'((?:[^'\\]|\\.)*)'\s*:\s*'((?:[^'\\]|\\.)*)'/gs
  let match
  while ((match = re.exec(block)) !== null) {
    pairs.push([match[1], unescape(match[2])])
  }
  return pairs
}

function readEsBlock(root) {
  const text = readFileSync(resolve(root, I18N_PATH), 'utf-8')
  const start = text.indexOf('const es = {')
  if (start < 0) throw new Error(`${I18N_PATH}: falta el bloque 'const es = {'`)
  const end = text.indexOf('} as const', start)
  if (end < 0) throw new Error(`${I18N_PATH}: falta el cierre '} as const' del español`)
  const block = text.slice(start, end)
  if (block.includes('const en')) throw new Error(`${I18N_PATH}: el bloque español trae inglés`)
  const pairs = extractPairs(block)
  if (pairs.length < 1000) throw new Error(`${I18N_PATH}: solo ${pairs.length} pares, raro`)
  return pairs
}

const toCamel = (name) => name.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase())

/** Nombres de `View` en el orden en que se declaran en navigation.ts. */
function readViews(root) {
  const text = readFileSync(resolve(root, NAVIGATION_PATH), 'utf-8')
  const names = []
  const re = /\{\s*name:\s*'([^']+)'/g
  let match
  while ((match = re.exec(text)) !== null) {
    if (!names.includes(match[1])) names.push(match[1])
  }
  if (names.length < 10) throw new Error(`${NAVIGATION_PATH}: solo ${names.length} vistas, raro`)
  return names
}

/** Claves `settings.*Tab` en el orden de las pestañas de SettingsView. */
function readSettingsTabs(root) {
  const text = readFileSync(resolve(root, SETTINGS_VIEW_PATH), 'utf-8')
  const keys = []
  const re = /\{t\('settings\.([A-Za-z]+Tab)'\)\}/g
  let match
  while ((match = re.exec(text)) !== null) {
    const key = `settings.${match[1]}`
    if (!keys.includes(key)) keys.push(key)
  }
  if (keys.length < 10) throw new Error(`${SETTINGS_VIEW_PATH}: solo ${keys.length} pestañas, raro`)
  return keys
}

/** Glosa española de una vista cuando alguna clave la nombra. */
function glossView(es, view) {
  const camel = toCamel(view)
  for (const key of [`nav.${camel}`, `topbar.${camel}Title`, `${camel}.title`]) {
    if (Object.hasOwn(es, key) && es[key].trim() !== '') return es[key]
  }
  return null
}

function lineFor(value, keys) {
  const shown = keys.length > 1 ? ` \`${[...keys].sort().join(', ')}\`` : ''
  return `- «${value}»${shown}`
}

/** Parte las líneas ordenadas en trozos que no pasen SECTION_MAX_BYTES. */
function chunkLines(lines) {
  const chunks = [[]]
  let size = 0
  for (const line of lines) {
    const width = Buffer.byteLength(line, 'utf-8') + 1
    if (size + width > SECTION_MAX_BYTES && chunks[chunks.length - 1].length > 0) {
      chunks.push([])
      size = 0
    }
    chunks[chunks.length - 1].push(line)
    size += width
  }
  return chunks
}

export function generateScreenMap(root) {
  const pairs = readEsBlock(root)
  const es = Object.fromEntries(pairs)

  // Área por prefijo; los errores a su área propia (sub-área = prefijo).
  const groups = new Map()
  const add = (area, sub, value, key) => {
    const id = `${area} ${sub}`
    if (!groups.has(id)) groups.set(id, { area, sub, values: new Map() })
    const values = groups.get(id).values
    if (!values.has(value)) values.set(value, [])
    if (!values.get(value).includes(key)) values.get(value).push(key)
  }
  for (const [key, value] of pairs) {
    if (value.trim() === '') continue
    const segments = key.split('.')
    if (ERROR_KEY.test(key)) {
      add('errors', segments[0], value, key)
    } else {
      add(segments[0], segments[1] ?? 'general', value, key)
    }
  }

  // Bloques unidos por líneas en blanco (como pide prettier): cada título
  // trae su contenido en el mismo bloque.
  const blocks = []
  blocks.push('# Mapa de pantallas de EntropIA')
  blocks.push(
    '> Generado por `apps/desktop/scripts/build-help-screen-map.mjs`; no editar a mano.\n' +
      `> Fuentes: \`${I18N_PATH}\` (bloque español), \`${NAVIGATION_PATH}\` (vistas) y\n` +
      `> \`${SETTINGS_VIEW_PATH}\` (pestañas).\n` +
      '> Regenerar desde `apps/desktop` con `pnpm help:screen-map`;\n' +
      '> comprobar con `pnpm help:screen-map:check`.'
  )

  // Pantallas primero: vistas y pestañas para titular cada pantalla.
  blocks.push('## Pantallas')
  const viewLines = []
  for (const view of readViews(root)) {
    const gloss = glossView(es, view)
    viewLines.push(gloss ? `- «${gloss}» \`${view}\`` : `- \`${view}\``)
  }
  blocks.push('### Vistas de la app')
  blocks.push(viewLines.join('\n'))
  const tabLines = []
  for (const key of readSettingsTabs(root)) {
    const label = es[key]
    if (label === undefined) throw new Error(`${key}: la pestaña no está en i18n.ts`)
    tabLines.push(`- «${label}» \`${key}\``)
  }
  blocks.push('### Pestañas de Configuración')
  blocks.push(tabLines.join('\n'))

  // Los grupos chicos (< MIN_GROUP_LINES) se juntan en «Varios»: con el
  // espacio de nombres tan plano, un ### por segmento daría cientos de
  // secciones de una línea. En «Varios» la línea lleva siempre la clave,
  // que dice de qué pantalla es la etiqueta.
  const byArea = new Map()
  for (const group of groups.values()) {
    if (!byArea.has(group.area)) byArea.set(group.area, { big: [], small: new Map() })
    const slot = byArea.get(group.area)
    if (group.values.size >= MIN_GROUP_LINES) {
      slot.big.push(group)
    } else {
      for (const [value, keys] of group.values) {
        if (!slot.small.has(value)) slot.small.set(value, [])
        for (const key of keys) {
          if (!slot.small.get(value).includes(key)) slot.small.get(value).push(key)
        }
      }
    }
  }

  const emitLines = (heading, lines) => {
    const chunks = chunkLines(lines)
    chunks.forEach((chunk, index) => {
      const title = chunks.length > 1 ? `${heading} (${index + 1}/${chunks.length})` : heading
      blocks.push(title)
      blocks.push(chunk.join('\n'))
    })
  }
  for (const area of [...byArea.keys()].sort()) {
    blocks.push(`## ${areaTitle(area)}`)
    const { big, small } = byArea.get(area)
    big.sort((a, b) => (a.sub < b.sub ? -1 : a.sub > b.sub ? 1 : 0))
    for (const { sub, values } of big) {
      const lines = [...values.entries()]
        .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
        .map(([value, keys]) => lineFor(value, keys))
      emitLines(`### ${subTitle(es, area, sub)}`, lines)
    }
    if (small.size > 0) {
      const lines = [...small.entries()]
        .sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
        .map(([value, keys]) => `- «${value}» \`${[...keys].sort().join(', ')}\``)
      emitLines('### Varios', lines)
    }
  }
  return `${blocks.join('\n\n')}\n`
}

function main() {
  const args = process.argv.slice(2)
  const root = resolve(
    here,
    args.includes('--root') ? args[args.indexOf('--root') + 1] : '../../..'
  )
  const check = args.includes('--check')
  const map = generateScreenMap(root)
  const target = resolve(root, MAP_PATH)
  if (check) {
    let current
    try {
      current = readFileSync(target, 'utf-8')
    } catch {
      console.error(`Falta ${MAP_PATH}: generarlo con pnpm help:screen-map`)
      process.exit(1)
    }
    if (current !== map) {
      console.error(`${MAP_PATH} está desactualizado: regenerarlo con pnpm help:screen-map`)
      process.exit(1)
    }
    console.log(`${MAP_PATH} al día`)
    return
  }
  writeFileSync(target, map)
  const sections = map.split(/^### /m).length - 1
  console.log(`${MAP_PATH}: ${Buffer.byteLength(map, 'utf-8')} bytes, ${sections} secciones`)
}

const invoked =
  process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)
if (invoked) main()
