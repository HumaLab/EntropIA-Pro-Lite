// Generates src/migrations.generated.ts from src/migrations/*.sql.
// Run with: pnpm --filter @entropia/store generate-migrations
//
// The .sql files are the single source of truth for the SQL the runner applies.
// The generated module inlines their exact bytes so the Tauri webview bundle
// never needs runtime filesystem access. The freshness test in
// src/migration-files.test.ts fails when the checked-in module goes stale.
//
// Node v24+ strips types when importing the .ts sources directly, so this stays
// dependency-free (no tsx/ts-node).

import { readdir, readFile, writeFile } from 'node:fs/promises'
import { fileURLToPath, pathToFileURL } from 'node:url'
import { dirname, resolve } from 'node:path'

const here = dirname(fileURLToPath(import.meta.url))
const migrationsDir = resolve(here, '../src/migrations')
const outputPath = resolve(here, '../src/migrations.generated.ts')

const HEADER = [
  '// Generated file — do not edit. Regenerate with:',
  '//   pnpm --filter @entropia/store generate-migrations',
  '//',
  '// The .sql files under src/migrations/ are the single source of truth for the',
  '// SQL the runner applies; this committed module inlines their exact content so',
  '// the Tauri webview bundle never needs runtime filesystem access.',
  '// Machine-generated: listed in .prettierignore.',
  'export const MIGRATION_FILES: Readonly<Record<string, string>> = {',
]

/**
 * Renders the generated module for the given [name, sql] entries.
 * Pure so the freshness test can compare its output with the checked-in file.
 */
export function renderMigrationsModule(files) {
  const entries = [...files].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))
  const body = entries.map(([name, sql]) => `  ${JSON.stringify(name)}: ${JSON.stringify(sql)},`)
  return [...HEADER, ...body, '}', ''].join('\n')
}

/** Reads every src/migrations/*.sql file as [name, exact content], sorted by name. */
export async function readMigrationFiles(dir = migrationsDir) {
  const names = (await readdir(dir)).filter((file) => file.endsWith('.sql')).sort()
  return Promise.all(
    names.map(async (file) => [
      file.slice(0, -'.sql'.length),
      await readFile(resolve(dir, file), 'utf8'),
    ])
  )
}

async function main() {
  const files = await readMigrationFiles()
  await writeFile(outputPath, renderMigrationsModule(files), 'utf8')
  console.log(`Wrote ${files.length} migrations to ${outputPath}`)
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  await main()
}
